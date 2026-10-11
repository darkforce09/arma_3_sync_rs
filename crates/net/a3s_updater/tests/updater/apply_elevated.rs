//! The elevated updater: its arguments, the child process and the parent's hand-off.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_foundation::paths::Os;
use a3s_updater::apply::replace::{FsOps, SelfReplacer};
use a3s_updater::apply::{ELEVATED_SUBCOMMAND, ElevatedInstallArgs, run_elevated_install_with};
use a3s_updater::elevation::{ElevatedLaunch, LaunchExit};
use a3s_updater::{
    ApplyError, CancellationToken, CheckOutcome, Release, UpdateError, UpdateEvent, UpdateSession,
};

use crate::apply_fixture::{
    Events, FakeLauncher, Fixture, NEW_BINARY, OLD_BINARY, OLD_VERSION, release_zip,
};
use crate::support::{HASH, sha256_hex};

fn absolute(name: &str) -> PathBuf {
    std::path::absolute(name).expect("absolute")
}

fn sample_args(debug_log: Option<PathBuf>) -> ElevatedInstallArgs {
    ElevatedInstallArgs {
        archive: absolute("staging dir/Arma3Sync 9.0.0.zip"),
        sha256: HASH.to_owned(),
        version: "9.0.0".to_owned(),
        marker: absolute("staging dir/elevated.startup"),
        result: absolute("staging dir/elevated.result"),
        debug_log,
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&s| s.to_owned()).collect()
}

#[test]
fn arguments_round_trip_in_any_order() {
    for debug_log in [None, Some(absolute("logs/updater.log"))] {
        let args = sample_args(debug_log);
        let list = args.to_args().expect("to_args");
        assert_eq!(list[0], ELEVATED_SUBCOMMAND);
        assert_eq!(ElevatedInstallArgs::parse(&list[1..]), Ok(args.clone()));

        let mut pairs: Vec<Vec<String>> = list[1..].chunks(2).map(<[String]>::to_vec).collect();
        pairs.reverse();
        assert_eq!(ElevatedInstallArgs::parse(&pairs.concat()), Ok(args));
    }
}

#[test]
fn invalid_arguments_are_rejected() {
    let valid = sample_args(None).to_args().expect("to_args")[1..].to_vec();
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
    let mut duplicate = valid.clone();
    duplicate.extend(strings(&["--version", "9.0.1"]));
    let mut unknown = valid.clone();
    unknown.extend(strings(&["--install-dir", "/somewhere"]));
    let mut dangling = valid.clone();
    dangling.push("--debug-log".to_owned());

    for invalid in [
        without("--archive"),
        without("--sha256"),
        without("--version"),
        without("--marker"),
        without("--result"),
        duplicate,
        unknown,
        dangling,
        with("--archive", "relative/update.zip"),
        with("--sha256", &HASH[..63]),
        with("--sha256", &format!("{}g", &HASH[..63])),
        with("--version", "  "),
    ] {
        assert!(ElevatedInstallArgs::parse(&invalid).is_err(), "{invalid:?}");
    }

    let upper = ElevatedInstallArgs::parse(&with("--sha256", &HASH.to_ascii_uppercase()))
        .expect("uppercase sha");
    assert_eq!(upper.sha256, HASH);
}

fn os_strings(args: &[String]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// An installation holding an old binary, an archive with a new one, and the child's
/// arguments for them.
fn child_setup() -> (tempfile::TempDir, PathBuf, ElevatedInstallArgs) {
    let temp = tempfile::tempdir().expect("tempdir");
    let install = temp.path().join("installation");
    let staging = temp.path().join("staging");
    std::fs::create_dir_all(&install).expect("mkdir");
    std::fs::create_dir_all(&staging).expect("mkdir");
    std::fs::write(install.join("arma3sync"), OLD_BINARY).expect("old");
    let archive = staging.join("update.zip");
    std::fs::write(&archive, release_zip(Os::Linux)).expect("zip");
    let args = ElevatedInstallArgs {
        sha256: sha256_hex(&std::fs::read(&archive).expect("read")),
        archive,
        version: "9.0.0".to_owned(),
        marker: staging.join("elevated.startup"),
        result: staging.join("elevated.result"),
        debug_log: None,
    };
    (temp, install, args)
}

#[test]
fn child_installs_next_to_its_own_executable() {
    let (_temp, install, args) = child_setup();
    let list = args.to_args().expect("to_args");
    let code = run_elevated_install_with(
        &os_strings(&list[1..]),
        &install.join("arma3sync"),
        Os::Linux,
    );
    assert_eq!(code, 0);
    assert!(args.marker.is_file());
    assert_eq!(std::fs::read(&args.result).expect("result"), b"");
    assert_eq!(
        std::fs::read(install.join("arma3sync")).expect("read"),
        NEW_BINARY
    );
    assert!(install.join("LICENSE.txt").is_file());
    assert!(
        args.archive.is_file(),
        "the child leaves the archive to the parent"
    );
}

#[test]
fn child_rejects_invalid_arguments_and_a_wrong_checksum() {
    let (_temp, install, args) = child_setup();
    let exe = install.join("arma3sync");
    let invalid = strings(&[
        "--result",
        args.result.to_str().expect("utf-8"),
        "--bogus",
        "x",
    ]);
    assert_eq!(
        run_elevated_install_with(&os_strings(&invalid), &exe, Os::Linux),
        1
    );
    assert!(!std::fs::read(&args.result).expect("result").is_empty());
    assert!(!args.marker.exists());

    let wrong = ElevatedInstallArgs {
        sha256: "0".repeat(64),
        ..args.clone()
    };
    let list = wrong.to_args().expect("to_args");
    assert_eq!(
        run_elevated_install_with(&os_strings(&list[1..]), &exe, Os::Linux),
        1
    );
    assert_eq!(
        std::fs::read_to_string(&args.result).expect("result"),
        UpdateError::Integrity.to_string()
    );
    assert_eq!(std::fs::read(&exe).expect("read"), OLD_BINARY);
}

#[cfg(unix)]
#[test]
fn child_without_write_access_fails_without_retrying() {
    let (_temp, install, args) = child_setup();
    let Some(_read_only) = crate::apply_fixture::ReadOnly::new(&install) else {
        eprintln!("skipped: running as root");
        return;
    };
    let list = args.to_args().expect("to_args");
    let code = run_elevated_install_with(
        &os_strings(&list[1..]),
        &install.join("arma3sync"),
        Os::Linux,
    );
    assert_eq!(code, 1);
    assert!(!std::fs::read(&args.result).expect("result").is_empty());
    assert_eq!(
        std::fs::read(install.join("arma3sync")).expect("read"),
        OLD_BINARY
    );
}

/// The child's arguments of an elevated launch.
fn child_args(launch: &ElevatedLaunch) -> Vec<OsString> {
    assert_eq!(launch.args[0], ELEVATED_SUBCOMMAND);
    os_strings(&launch.args[1..])
}

async fn available(session: &UpdateSession) -> Release {
    match session
        .check(&CancellationToken::new())
        .await
        .expect("check")
    {
        CheckOutcome::Available { release, .. } => release,
        CheckOutcome::UpToDate { .. } => panic!("no update"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn parent_hands_the_install_to_the_elevated_child() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let fixture = Fixture::new(Os::Windows).await;
    let Some(read_only) = crate::apply_fixture::ReadOnly::new(fixture.dir()) else {
        eprintln!("skipped: running as root");
        return;
    };
    let exe = fixture.binary();
    let dir = fixture.dir().to_path_buf();
    let marker_written = Arc::new(AtomicBool::new(false));
    let marker_seen = Arc::clone(&marker_written);
    let launcher = Arc::new(FakeLauncher::new(move |launch| {
        use std::os::unix::fs::PermissionsExt as _;
        // Granting UAC gives the child write access to the installation.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
        let code = run_elevated_install_with(&child_args(launch), &exe, Os::Windows);
        let marker = &launch.args[launch
            .args
            .iter()
            .position(|a| a == "--marker")
            .expect("marker")
            + 1];
        marker_seen.store(Path::new(marker).is_file(), Ordering::SeqCst);
        Ok(LaunchExit::Exited(code))
    }));
    let session = fixture.session(OLD_VERSION).with_launcher(launcher.clone());
    let release = available(&session).await;
    let events = Events::default();

    let outcome = session
        .apply(&release, &|e| events.record(e), &CancellationToken::new())
        .await
        .expect("apply");
    drop(read_only);

    assert!(outcome.elevated);
    assert!(outcome.report.is_none());
    assert_eq!(std::fs::read(fixture.binary()).expect("read"), NEW_BINARY);
    assert!(marker_written.load(Ordering::SeqCst));
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
    assert_eq!(calls[0].launch.program, fixture.binary());
    assert_eq!(calls[0].launch.working_dir, fixture.dir());
    assert!(fixture.staging_is_empty());
}

/// File operations whose renames are always denied, as for a file Windows will not let go.
struct DeniedFs;

impl FsOps for DeniedFs {
    fn copy(&self, _from: &Path, _to: &Path) -> io::Result<u64> {
        Ok(0)
    }

    fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
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
    launcher: &Arc<FakeLauncher>,
) -> (Result<a3s_updater::ApplyOutcome, ApplyError>, Events) {
    let mut ctx = fixture.context(OLD_VERSION);
    ctx.elevated = elevated;
    let session = UpdateSession::new(ctx)
        .with_replacer(Arc::new(SelfReplacer::with_fs(Box::new(DeniedFs), true)))
        .with_launcher(launcher.clone());
    let release = available(&session).await;
    let events = Events::default();
    let result = session
        .apply(&release, &|e| events.record(e), &CancellationToken::new())
        .await;
    (result, events)
}

#[tokio::test]
async fn denied_install_is_retried_once_through_the_elevated_child() {
    let fixture = Fixture::new(Os::Windows).await;
    let exe = fixture.binary();
    let launcher = Arc::new(FakeLauncher::new(move |launch| {
        Ok(LaunchExit::Exited(run_elevated_install_with(
            &child_args(launch),
            &exe,
            Os::Windows,
        )))
    }));
    let (result, events) = apply_denied(&fixture, false, &launcher).await;

    let outcome = result.expect("apply");
    assert!(outcome.elevated);
    assert_eq!(std::fs::read(fixture.binary()).expect("read"), NEW_BINARY);
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
    let calls = launcher.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].archive_existed);
    assert!(calls[0].value("--archive").ends_with("Arma3Sync-9.0.0.zip"));
    assert_eq!(
        calls[0].value("--sha256"),
        sha256_hex(&release_zip(Os::Windows))
    );
    assert_eq!(fixture.downloads(), 1);
    assert!(fixture.staging_is_empty());
}

#[tokio::test]
async fn elevated_process_and_other_systems_do_not_request_uac() {
    let fixture = Fixture::new(Os::Windows).await;
    let launcher = Arc::new(FakeLauncher::new(|_| Ok(LaunchExit::Exited(0))));
    let (result, _) = apply_denied(&fixture, true, &launcher).await;
    let err = result.expect_err("denied");
    assert!(matches!(err, ApplyError::Install { .. }), "{err:?}");
    assert!(err.is_permission_failure());
    assert!(launcher.calls().is_empty());

    let fixture = Fixture::new(Os::Linux).await;
    let (result, _) = apply_denied(&fixture, false, &launcher).await;
    assert!(matches!(result, Err(ApplyError::ElevationUnavailable)));
    assert!(launcher.calls().is_empty());
}

#[tokio::test]
async fn child_with_a_wrong_checksum_reports_its_error_to_the_parent() {
    let fixture = Fixture::new(Os::Windows).await;
    let exe = fixture.binary();
    let launcher = Arc::new(FakeLauncher::new(move |launch| {
        let mut args = child_args(launch);
        let index = args.iter().position(|a| a == "--sha256").expect("sha");
        args[index + 1] = OsString::from("0".repeat(64));
        Ok(LaunchExit::Exited(run_elevated_install_with(
            &args,
            &exe,
            Os::Windows,
        )))
    }));
    let (result, _) = apply_denied(&fixture, false, &launcher).await;
    match result {
        Err(ApplyError::ElevatedInstallFailed(message)) => {
            assert_eq!(message, UpdateError::Integrity.to_string());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(fixture.binary()).expect("read"), OLD_BINARY);
}

#[tokio::test]
async fn launcher_outcomes_map_to_errors() {
    let fixture = Fixture::new(Os::Windows).await;
    let run = |answer: fn(&ElevatedLaunch) -> io::Result<LaunchExit>| {
        let launcher = Arc::new(FakeLauncher::new(answer));
        let fixture = &fixture;
        async move { apply_denied(fixture, false, &launcher).await.0 }
    };

    match run(|_| {
        Ok(LaunchExit::NotStarted {
            detail: " canceled by the user ".to_owned(),
        })
    })
    .await
    {
        Err(err @ ApplyError::ElevationNotGranted(_)) => {
            assert!(err.needs_admin());
            assert!(err.to_string().ends_with(". canceled by the user"), "{err}");
        }
        other => panic!("{other:?}"),
    }

    let not_started = run(|_| Ok(LaunchExit::Exited(1))).await;
    let failed = run(|launch| {
        let index = launch
            .args
            .iter()
            .position(|a| a == "--marker")
            .expect("marker");
        std::fs::write(&launch.args[index + 1], b"started")?;
        Ok(LaunchExit::Exited(3))
    })
    .await;
    match (not_started, failed) {
        (
            Err(ApplyError::ElevatedInstallFailed(not_started)),
            Err(ApplyError::ElevatedInstallFailed(failed)),
        ) => {
            assert_ne!(not_started, failed);
            assert!(failed.contains('3'), "{failed}");
        }
        other => panic!("{other:?}"),
    }

    assert!(matches!(
        run(|_| Ok(LaunchExit::TimedOut)).await,
        Err(ApplyError::ElevatedInstallFailed(_))
    ));
    match run(|_| Err(io::Error::other("powershell.exe is missing"))).await {
        Err(err @ ApplyError::ElevationNotGranted(_)) => {
            assert!(err.to_string().contains("powershell.exe is missing"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(fixture.binary()).expect("read"), OLD_BINARY);
    assert!(fixture.staging_is_empty());
}

#[test]
fn archive_path_must_exist_for_the_child() {
    let (_temp, install, args) = child_setup();
    let missing = ElevatedInstallArgs {
        archive: args.archive.with_file_name("missing.zip"),
        ..args.clone()
    };
    let list = missing.to_args().expect("to_args");
    assert_eq!(
        run_elevated_install_with(
            &os_strings(&list[1..]),
            &install.join("arma3sync"),
            Os::Linux
        ),
        1
    );
    assert!(!std::fs::read(&args.result).expect("result").is_empty());
}
