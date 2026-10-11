//! The elevated updater: its arguments, the child process and the parent's hand-off.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_foundation::paths::Os;
use a3s_updater::apply::replace::{FsOps, SelfReplacer};
use a3s_updater::apply::{ELEVATED_SUBCOMMAND, ElevatedInstallArgs, run_elevated_install_with};
use a3s_updater::elevation::LaunchExit;
use a3s_updater::{
    ApplyError, ApplyOutcome, CancellationToken, UpdateEvent, UpdateSession, UpdateSource,
};

use crate::apply_fixture::{
    Events, FakeLauncher, Fixture, NEW_BINARY, NEW_VERSION, OLD_BINARY, OLD_VERSION,
};
use crate::support::{Entry, zip_bytes};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&s| s.to_owned()).collect()
}

fn os_strings(args: &[String]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn sample_args(dev: bool, debug: bool) -> ElevatedInstallArgs {
    ElevatedInstallArgs {
        archive: std::path::absolute("staging dir/Arma3Sync 9.0.0.zip").expect("absolute"),
        version: NEW_VERSION.to_owned(),
        nonce: ElevatedInstallArgs::new_nonce(),
        dev,
        source: UpdateSource::GitHub,
        debug,
    }
}

#[test]
fn arguments_round_trip_in_any_order() {
    for (dev, debug) in [(false, false), (true, true)] {
        let args = sample_args(dev, debug);
        let list = args.to_args().expect("to_args");
        assert_eq!(list[0], ELEVATED_SUBCOMMAND);
        assert!(!list.iter().any(|a| a.contains("sha")));
        assert_eq!(ElevatedInstallArgs::parse(&list[1..]), Ok(args.clone()));

        let mut reversed = list[1..].to_vec();
        reversed.reverse();
        // Reversing swaps each option with its value; swap them back.
        let mut fixed = Vec::new();
        let mut rest = reversed.into_iter();
        while let Some(item) = rest.next() {
            if item.starts_with("--") {
                fixed.push(item);
            } else {
                let option = rest.next().expect("option");
                fixed.extend([option, item]);
            }
        }
        assert_eq!(ElevatedInstallArgs::parse(&fixed), Ok(args));
    }
    assert_ne!(
        ElevatedInstallArgs::new_nonce(),
        ElevatedInstallArgs::new_nonce()
    );
}

#[test]
fn invalid_arguments_are_rejected() {
    let valid = sample_args(false, false).to_args().expect("to_args")[1..].to_vec();
    let with = |option: &str, value: &str| {
        let mut list = valid.clone();
        let index = list.iter().position(|a| a == option).expect("option");
        list[index + 1] = value.to_owned();
        list
    };
    let without = |option: &str| {
        let mut list = valid.clone();
        let index = list.iter().position(|a| a == option).expect("option");
        list.drain(index..index + 2);
        list
    };
    let extended = |extra: &[&str]| {
        let mut list = valid.clone();
        list.extend(strings(extra));
        list
    };

    for invalid in [
        without("--archive"),
        without("--version"),
        without("--nonce"),
        without("--source"),
        extended(&["--version", "9.0.1"]),
        extended(&["--dev", "--dev"]),
        extended(&["--sha256", &"0".repeat(64)]),
        extended(&["--archive"]),
        with("--archive", "relative/update.zip"),
        with("--nonce", "0123"),
        with("--nonce", &"g".repeat(32)),
        with("--source", "ftp"),
        with("--version", "  "),
    ] {
        assert!(ElevatedInstallArgs::parse(&invalid).is_err(), "{invalid:?}");
    }

    let upper = ElevatedInstallArgs::parse(&with("--nonce", &"AB".repeat(16))).expect("nonce");
    assert_eq!(upper.nonce, "ab".repeat(16));
}

/// The child's arguments for the fixture's release, downloaded into the staging directory.
fn child_args(fixture: &Fixture, archive: &[u8]) -> ElevatedInstallArgs {
    let path = fixture.staging.path().join("Arma3Sync-9.0.0.zip");
    std::fs::write(&path, archive).expect("archive");
    ElevatedInstallArgs {
        archive: path,
        version: NEW_VERSION.to_owned(),
        nonce: ElevatedInstallArgs::new_nonce(),
        dev: false,
        source: UpdateSource::Configured,
        debug: true,
    }
}

/// Runs the child for `fixture`'s installation on a blocking thread, as an elevated process
/// of version `current_version` would.
async fn run_child(fixture: &Fixture, args: &ElevatedInstallArgs, current_version: &str) -> i32 {
    let list = os_strings(&args.to_args().expect("to_args")[1..]);
    let exe = fixture.binary();
    let os = fixture.os;
    let version = current_version.to_owned();
    tokio::task::spawn_blocking(move || run_elevated_install_with(&list, &exe, os, &version))
        .await
        .expect("child")
}

/// The work directory the child created for `nonce`.
fn work_dir(fixture: &Fixture, nonce: &str) -> Option<PathBuf> {
    std::fs::read_dir(fixture.dir())
        .expect("list")
        .map(|e| e.expect("entry").path())
        .find(|path| {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            name.starts_with(".a3s-elevated-") && name.ends_with(nonce)
        })
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read")
}

#[tokio::test]
async fn child_installs_the_release_it_resolved_itself() {
    let fixture = Fixture::new(Os::Linux).await;
    let args = child_args(&fixture, &crate::apply_fixture::release_zip(Os::Linux));
    assert_eq!(run_child(&fixture, &args, OLD_VERSION).await, 0);

    assert_eq!(read(&fixture.binary()), NEW_BINARY);
    assert!(fixture.dir().join("LICENSE.txt").is_file());
    let work = work_dir(&fixture, &args.nonce).expect("work dir");
    assert!(work.join("elevated.startup").is_file());
    assert_eq!(read(&work.join("elevated.result")), b"");
    assert!(work.join("elevated-updater.log").is_file());
    assert!(
        args.archive.is_file(),
        "the child leaves the archive to the parent"
    );
}

#[tokio::test]
async fn child_refuses_an_archive_that_does_not_match_the_resolved_checksum() {
    let fixture = Fixture::new(Os::Linux).await;
    let hostile = zip_bytes(&[Entry::File("arma3sync", b"hostile")]);
    let args = child_args(&fixture, &hostile);
    assert_eq!(run_child(&fixture, &args, OLD_VERSION).await, 1);
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
    let work = work_dir(&fixture, &args.nonce).expect("work dir");
    assert!(!read(&work.join("elevated.result")).is_empty());
}

#[tokio::test]
async fn child_refuses_a_release_that_is_not_newer() {
    let fixture = Fixture::new(Os::Linux).await;
    let args = child_args(&fixture, &crate::apply_fixture::release_zip(Os::Linux));
    assert_eq!(run_child(&fixture, &args, NEW_VERSION).await, 1);
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
}

#[tokio::test]
async fn child_refuses_a_version_other_than_the_resolved_one() {
    let fixture = Fixture::new(Os::Linux).await;
    let args = ElevatedInstallArgs {
        version: "8.0.0".to_owned(),
        ..child_args(&fixture, &crate::apply_fixture::release_zip(Os::Linux))
    };
    assert_eq!(run_child(&fixture, &args, OLD_VERSION).await, 1);
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
}

#[tokio::test]
async fn child_refuses_invalid_arguments_and_an_existing_work_directory() {
    let fixture = Fixture::new(Os::Linux).await;
    let exe = fixture.binary();
    let invalid = os_strings(&strings(&["--bogus", "x"]));
    let code = tokio::task::spawn_blocking(move || {
        run_elevated_install_with(&invalid, &exe, Os::Linux, OLD_VERSION)
    })
    .await
    .expect("child");
    assert_eq!(code, 1);

    let args = child_args(&fixture, &crate::apply_fixture::release_zip(Os::Linux));
    // The tests run the child in this process, so its work directory carries this pid.
    let taken = fixture.dir().join(format!(
        ".a3s-elevated-{}-{}",
        std::process::id(),
        args.nonce
    ));
    let outside = tempfile::tempdir().expect("outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &taken).expect("symlink");
    #[cfg(not(unix))]
    std::fs::create_dir(&taken).expect("mkdir");
    assert_eq!(run_child(&fixture, &args, OLD_VERSION).await, 1);
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
    assert_eq!(std::fs::read_dir(outside.path()).expect("list").count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn child_without_write_access_fails() {
    let fixture = Fixture::new(Os::Linux).await;
    let args = child_args(&fixture, &crate::apply_fixture::release_zip(Os::Linux));
    let read_only = crate::apply_fixture::ReadOnly::new(fixture.dir());
    assert_eq!(run_child(&fixture, &args, OLD_VERSION).await, 1);
    drop(read_only);
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
}

/// A launcher that grants the child write access to the installation (as UAC does) and runs
/// it in this process.
#[cfg(unix)]
fn granting_launcher(fixture: &Fixture, archive: Option<Vec<u8>>) -> Arc<FakeLauncher> {
    let exe = fixture.binary();
    let dir = fixture.dir().to_path_buf();
    Arc::new(FakeLauncher::new(move |launch| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
        if let Some(archive) = &archive {
            let index = launch
                .args
                .iter()
                .position(|a| a == "--archive")
                .expect("archive");
            std::fs::write(&launch.args[index + 1], archive)?;
        }
        let args = os_strings(&launch.args[1..]);
        Ok(LaunchExit::Exited(run_elevated_install_with(
            &args,
            &exe,
            Os::Windows,
            OLD_VERSION,
        )))
    }))
}

/// Applies the fixture's update through `launcher` with a read-only installation.
#[cfg(unix)]
async fn apply_read_only(
    fixture: &Fixture,
    launcher: &Arc<FakeLauncher>,
) -> (Result<ApplyOutcome, ApplyError>, Events) {
    let session = fixture.session(OLD_VERSION).with_launcher(launcher.clone());
    let release = crate::apply_fixture::available(&session).await;
    let read_only = crate::apply_fixture::ReadOnly::new(fixture.dir());
    let events = Events::default();
    let result = session
        .apply(&release, &|e| events.record(e), &CancellationToken::new())
        .await;
    drop(read_only);
    (result, events)
}

#[cfg(unix)]
#[tokio::test]
async fn parent_hands_the_install_to_the_elevated_child() {
    let fixture = Fixture::new(Os::Windows).await;
    let launcher = granting_launcher(&fixture, None);
    let (result, events) = apply_read_only(&fixture, &launcher).await;

    let outcome = result.expect("apply");
    assert!(outcome.elevated);
    assert!(outcome.report.is_none());
    assert_eq!(read(&fixture.binary()), NEW_BINARY);
    assert_eq!(
        events.after_download(),
        [
            UpdateEvent::WaitingForElevation,
            UpdateEvent::ElevatedInstallStarted,
            UpdateEvent::Installing,
            UpdateEvent::Done
        ]
    );
    let calls = launcher.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].archive_existed);
    assert_eq!(calls[0].value("--version"), NEW_VERSION);
    assert_eq!(calls[0].launch.program, fixture.binary());
    assert_eq!(calls[0].launch.working_dir, fixture.dir());
    let work = work_dir(&fixture, &calls[0].value("--nonce")).expect("work dir");
    assert!(work.join("elevated.startup").is_file());
    assert!(fixture.staging_is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn child_error_reaches_the_parent() {
    let fixture = Fixture::new(Os::Windows).await;
    let hostile = zip_bytes(&[Entry::File("arma3sync.exe", b"hostile")]);
    let launcher = granting_launcher(&fixture, Some(hostile));
    let (result, _) = apply_read_only(&fixture, &launcher).await;
    assert!(
        matches!(result, Err(ApplyError::ElevatedInstallFailed(_))),
        "{result:?}"
    );
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_outcomes_map_to_errors() {
    let fixture = Fixture::new(Os::Windows).await;
    let dir = fixture.dir().to_path_buf();
    let run = |answer: crate::apply_fixture::Answer| {
        let launcher = Arc::new(FakeLauncher::new(answer));
        let fixture = &fixture;
        async move { apply_read_only(fixture, &launcher).await.0 }
    };

    let not_granted = run(Box::new(|_| {
        Ok(LaunchExit::NotStarted {
            detail: "canceled by the user".to_owned(),
        })
    }))
    .await;
    assert!(
        matches!(&not_granted, Err(e @ ApplyError::ElevationNotGranted(_)) if e.needs_admin()),
        "{not_granted:?}"
    );

    let not_started = run(Box::new(|_| Ok(LaunchExit::Exited(1)))).await;
    let failed = run(Box::new(move |launch| {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
        let index = launch
            .args
            .iter()
            .position(|a| a == "--nonce")
            .expect("nonce");
        let work = dir.join(format!(".a3s-elevated-1-{}", launch.args[index + 1]));
        std::fs::create_dir(&work)?;
        std::fs::write(work.join("elevated.startup"), b"started")?;
        Ok(LaunchExit::Exited(3))
    }))
    .await;
    match (not_started, failed) {
        (
            Err(ApplyError::ElevatedInstallFailed(not_started)),
            Err(ApplyError::ElevatedInstallFailed(failed)),
        ) => assert_ne!(not_started, failed),
        other => panic!("{other:?}"),
    }

    assert!(matches!(
        run(Box::new(|_| Ok(LaunchExit::TimedOut))).await,
        Err(ApplyError::ElevatedInstallFailed(_))
    ));
    assert!(matches!(
        run(Box::new(|_| Err(io::Error::other("no PowerShell")))).await,
        Err(ApplyError::ElevationNotGranted(_))
    ));
    assert_eq!(read(&fixture.binary()), OLD_BINARY);
    assert!(fixture.staging_is_empty());
}

/// File operations whose renames are always denied, as for a file Windows will not let go.
struct DeniedFs {
    /// Made read-only on the first rename, as if the installation's permissions changed.
    #[cfg_attr(
        not(unix),
        expect(dead_code, reason = "only Unix tests can make a directory read-only")
    )]
    lock: Option<PathBuf>,
}

impl FsOps for DeniedFs {
    fn copy(&self, _from: &Path, _to: &Path) -> io::Result<u64> {
        Ok(0)
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        #[cfg(unix)]
        if let Some(dir) = &self.lock {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555))?;
        }
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn set_mode(&self, _path: &Path, _mode: u32) -> io::Result<()> {
        Ok(())
    }

    fn exists(&self, _path: &Path) -> bool {
        true
    }
}

/// Applies the fixture's update in a session whose in-process install is denied.
async fn apply_denied(
    fixture: &Fixture,
    elevated: bool,
    lock: Option<PathBuf>,
    launcher: &Arc<FakeLauncher>,
) -> (Result<ApplyOutcome, ApplyError>, Events) {
    let mut ctx = fixture.context(OLD_VERSION);
    ctx.elevated = elevated;
    let session = UpdateSession::new(ctx)
        .with_replacer(Arc::new(SelfReplacer::with_fs(
            Box::new(DeniedFs { lock }),
            true,
        )))
        .with_launcher(launcher.clone());
    let release = crate::apply_fixture::available(&session).await;
    let events = Events::default();
    let result = session
        .apply(&release, &|e| events.record(e), &CancellationToken::new())
        .await;
    (result, events)
}

#[tokio::test]
async fn denied_file_in_a_writable_installation_is_not_elevated() {
    let launcher = Arc::new(FakeLauncher::new(|_| Ok(LaunchExit::Exited(0))));
    for (os, elevated) in [
        (Os::Windows, false),
        (Os::Windows, true),
        (Os::Linux, false),
    ] {
        let fixture = Fixture::new(os).await;
        let (result, events) = apply_denied(&fixture, elevated, None, &launcher).await;
        assert_eq!(events.after_download(), [UpdateEvent::Installing]);
        let err = result.expect_err("denied");
        assert!(matches!(err, ApplyError::Install { .. }), "{err:?}");
        assert!(err.is_permission_failure());
    }
    assert!(launcher.calls().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn installation_that_became_unwritable_is_retried_elevated() {
    let fixture = Fixture::new(Os::Windows).await;
    let launcher = granting_launcher(&fixture, None);
    let lock = Some(fixture.dir().to_path_buf());
    let (result, events) = apply_denied(&fixture, false, lock, &launcher).await;

    assert!(result.expect("apply").elevated);
    assert_eq!(read(&fixture.binary()), NEW_BINARY);
    assert_eq!(launcher.calls().len(), 1);
    assert_eq!(
        events.after_download(),
        [
            UpdateEvent::Installing,
            UpdateEvent::WaitingForElevation,
            UpdateEvent::ElevatedInstallStarted,
            UpdateEvent::Installing,
            UpdateEvent::Done
        ]
    );
}
