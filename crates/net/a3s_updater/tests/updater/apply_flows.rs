//! Updater flags, dialog texts and the relaunch after an update.

use std::path::PathBuf;

use a3s_foundation::paths::Os;
use a3s_updater::apply::console::{UpdaterMode, UsageError, parse_updater_flags};
use a3s_updater::apply::relaunch::RelaunchPlan;
use a3s_updater::apply::{source_for_preference, text};
use a3s_updater::{ApplyError, UpdateContext, UpdateError, UpdateSource};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&s| s.to_owned()).collect()
}

#[test]
fn updater_flags() {
    let flags = parse_updater_flags(&strings(&["-DEV", "-Check"])).expect("flags");
    assert!(flags.dev);
    assert_eq!(flags.mode, UpdaterMode::Check);
    assert_eq!(flags.source, UpdateSource::Configured);

    let flags = parse_updater_flags(&strings(&["-console", "-check"])).expect("flags");
    assert_eq!(flags.mode, UpdaterMode::Console);
    assert!(!flags.dev);

    let flags = parse_updater_flags(&strings(&["-manifest"])).expect("flags");
    assert_eq!(
        (flags.mode, flags.source),
        (UpdaterMode::Gui, UpdateSource::Manifest)
    );
    let flags = parse_updater_flags(&strings(&["-github"])).expect("flags");
    assert_eq!(flags.source, UpdateSource::GitHub);
    assert_eq!(
        parse_updater_flags(&[]).expect("flags").mode,
        UpdaterMode::Gui
    );

    let conflict = parse_updater_flags(&strings(&["-github", "-manifest"])).expect_err("conflict");
    assert_eq!(conflict, UsageError::ConflictingSource);
    let unknown = parse_updater_flags(&strings(&["-console", "-foo"])).expect_err("unknown");
    assert_eq!(unknown, UsageError::Unknown("-foo".to_owned()));
    for error in [conflict, unknown] {
        assert_eq!(error.exit_code(), 1);
        assert!(!error.lines().is_empty());
    }
}

#[test]
fn update_source_from_the_preference() {
    assert_eq!(source_for_preference(true), UpdateSource::GitHub);
    assert_eq!(source_for_preference(false), UpdateSource::Manifest);
}

#[test]
fn error_dialogs() {
    assert!(ApplyError::Cancelled.dialog().is_none());

    for admin in [
        ApplyError::ElevationUnavailable,
        ApplyError::ElevationNotGranted("refused".to_owned()),
    ] {
        let (title, body) = admin.dialog().expect("dialog");
        assert_eq!(title, text::TITLE_ADMIN_REQUIRED);
        assert_eq!(body, format!("{}\n{admin}", text::ADMIN_REQUIRED));
    }

    let integrity = ApplyError::Download(UpdateError::Integrity);
    assert!(!integrity.needs_admin());
    let (title, body) = integrity.dialog().expect("dialog");
    assert_eq!(title, text::TITLE_ERROR);
    assert!(
        body.contains(&format!("\n{}\n", UpdateError::Integrity)),
        "{body}"
    );
    assert!(body.ends_with(text::ABORTED));
}

#[test]
fn progress_percent_is_bounded() {
    assert_eq!(text::progress_percent(50, Some(200)), Some(25));
    assert_eq!(text::progress_percent(300, Some(200)), Some(100));
    assert_eq!(text::progress_percent(u64::MAX, Some(1)), Some(100));
    assert_eq!(text::progress_percent(10, Some(0)), None);
    assert_eq!(text::progress_percent(10, None), None);
}

#[test]
fn relaunch_plan_prefers_the_installed_binary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let exe = PathBuf::from("elsewhere/arma3sync");
    let mut ctx = UpdateContext::new(exe.clone(), temp.path().to_path_buf(), "1.0.0", Os::Linux);
    let plan = RelaunchPlan::for_context(&ctx);
    assert_eq!(plan.program, exe);
    assert!(plan.args.is_empty());
    assert_eq!(plan.working_dir, temp.path());

    std::fs::write(temp.path().join("arma3sync"), b"binary").expect("binary");
    ctx.dev = true;
    let plan = RelaunchPlan::for_context(&ctx);
    assert_eq!(plan.program, temp.path().join("arma3sync"));
    assert_eq!(plan.args, ["-dev"]);
}

#[cfg(unix)]
#[test]
fn relaunch_starts_the_program_detached() {
    use std::os::unix::fs::PermissionsExt as _;
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().expect("tempdir");
    let script = temp.path().join("arma3sync");
    let output = temp.path().join("args.txt");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" > \"{}.tmp\"\nmv \"{0}.tmp\" \"{0}\"\n",
            output.display()
        ),
    )
    .expect("script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let plan = RelaunchPlan {
        program: script,
        args: vec!["-dev".to_owned()],
        working_dir: temp.path().to_path_buf(),
    };

    let mut released = 0;
    // A concurrent fork in another test can briefly hold the new script open for writing.
    loop {
        match a3s_updater::apply::relaunch::relaunch(&plan, || released += 1) {
            Err(err) if err.raw_os_error() == Some(26) && released < 20 => {
                std::thread::sleep(Duration::from_millis(50));
            }
            result => break result.expect("relaunch"),
        }
    }
    assert!(released > 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !output.exists() {
        assert!(
            Instant::now() < deadline,
            "the relaunched program did not run"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&output).expect("read").trim(),
        "-dev"
    );
}
