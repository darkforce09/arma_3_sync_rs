//! Update sessions and the console flows against a local update server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_foundation::paths::Os;
use a3s_updater::apply::console::{
    EXIT_FAILURE, EXIT_NO_UPDATE, EXIT_OK, run_check_only, run_console_update,
    run_legacy_update_command,
};
use a3s_updater::apply::text;
use a3s_updater::{
    ApplyError, CancellationToken, CheckOutcome, MANAGED_FILES_NAME, UpdateError, UpdateEvent,
};

use crate::apply_fixture::{
    ARCHIVE_PATH, Events, Fixture, MANIFEST_PATH, NEW_BINARY, NEW_VERSION, OLD_VERSION,
};
use crate::support::{Entry, Reply, sha256_hex, zip_bytes};

fn text_of(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("utf-8")
}

#[tokio::test]
async fn console_update_installs_and_reports_progress() {
    let fixture = Fixture::new(Os::Linux).await;
    let session = fixture.session(OLD_VERSION);
    let (mut out, mut err) = (Vec::new(), Vec::new());

    let code = run_console_update(&session, &mut out, &mut err, &CancellationToken::new()).await;
    assert_eq!(code, EXIT_OK, "{}", text_of(&err));
    assert!(err.is_empty());

    let out = text_of(&out);
    let head = format!("{}\n", text::updating(NEW_VERSION));
    let tail = format!(
        "\n{}\n{}\n",
        text::PROCESSING,
        text::success_message(NEW_VERSION)
    );
    assert!(out.starts_with(&head) && out.ends_with(&tail), "{out:?}");
    let progress = &out[head.len()..out.len() - tail.len()];
    let percents: Vec<&str> = progress.split('\r').skip(1).collect();
    assert!(
        progress.starts_with('\r') && !percents.is_empty(),
        "{progress:?}"
    );
    assert_eq!(
        percents.last().copied(),
        Some(text::downloading(100).as_str())
    );

    assert_eq!(std::fs::read(fixture.binary()).expect("read"), NEW_BINARY);
    let managed = std::fs::read_to_string(fixture.dir().join(MANAGED_FILES_NAME)).expect("state");
    assert_eq!(
        managed.lines().collect::<Vec<_>>(),
        ["arma3sync", "LICENSE.txt"]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(fixture.binary())
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert!(fixture.staging_is_empty());

    let session = fixture.session(NEW_VERSION);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_console_update(&session, &mut out, &mut err, &CancellationToken::new()).await;
    assert_eq!(code, EXIT_NO_UPDATE);
    assert_eq!(text_of(&out), format!("{}\n", text::NO_UPDATE));
}

#[tokio::test]
async fn check_only_exit_codes() {
    let fixture = Fixture::new(Os::Linux).await;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_check_only(
        &fixture.session(OLD_VERSION),
        &mut out,
        &mut err,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(code, EXIT_OK);
    assert_eq!(
        text_of(&out),
        format!("{}\n", text::update_available(NEW_VERSION))
    );

    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_check_only(
        &fixture.session(NEW_VERSION),
        &mut out,
        &mut err,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(code, EXIT_NO_UPDATE);

    fixture.server.route(MANIFEST_PATH, Reply::status(500));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_check_only(
        &fixture.session(OLD_VERSION),
        &mut out,
        &mut err,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(code, EXIT_FAILURE);
    assert!(out.is_empty());
    assert!(text_of(&err).starts_with(&text::check_failed(&"")));
    assert_eq!(fixture.downloads(), 0);
}

#[tokio::test]
async fn legacy_update_command_always_exits_zero_and_writes_to_out() {
    let fixture = Fixture::new(Os::Linux).await;
    let cancel = CancellationToken::new();
    let mut out = Vec::new();
    assert_eq!(
        run_legacy_update_command(&fixture.session(OLD_VERSION), &mut out, &cancel).await,
        EXIT_OK
    );
    assert!(text_of(&out).ends_with(&format!("{}\n", text::success_message(NEW_VERSION))));

    let mut out = Vec::new();
    assert_eq!(
        run_legacy_update_command(&fixture.session(NEW_VERSION), &mut out, &cancel).await,
        EXIT_OK
    );
    assert_eq!(text_of(&out), format!("{}\n", text::NO_UPDATE));

    fixture.server.route(MANIFEST_PATH, Reply::status(500));
    let mut out = Vec::new();
    assert_eq!(
        run_legacy_update_command(&fixture.session(OLD_VERSION), &mut out, &cancel).await,
        EXIT_OK
    );
    assert!(text_of(&out).starts_with(&text::check_failed(&"")));

    let failing = Fixture::new(Os::Linux).await;
    failing.serve(failing_archive(), &"0".repeat(64), 0);
    let mut out = Vec::new();
    assert_eq!(
        run_legacy_update_command(&failing.session(OLD_VERSION), &mut out, &cancel).await,
        EXIT_OK
    );
    assert!(text_of(&out).ends_with(&format!("{}\n", text::ABORTED)));
}

fn failing_archive() -> Vec<u8> {
    zip_bytes(&[Entry::File("arma3sync", b"never installed")])
}

#[cfg(unix)]
#[tokio::test]
async fn read_only_installation_fails_before_download_without_uac() {
    let fixture = Fixture::new(Os::Linux).await;
    let _read_only = crate::apply_fixture::ReadOnly::new(fixture.dir());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_console_update(
        &fixture.session(OLD_VERSION),
        &mut out,
        &mut err,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(code, EXIT_FAILURE);
    assert_eq!(
        text_of(&err),
        format!(
            "{}\n{}\n",
            text::ADMIN_REQUIRED,
            ApplyError::ElevationUnavailable
        )
    );
    assert_eq!(fixture.downloads(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn read_only_installation_on_windows_requests_uac_after_the_download() {
    use crate::apply_fixture::FakeLauncher;
    use a3s_updater::elevation::LaunchExit;

    let fixture = Fixture::new(Os::Windows).await;
    let _read_only = crate::apply_fixture::ReadOnly::new(fixture.dir());
    let launcher = Arc::new(FakeLauncher::new(|_| Ok(LaunchExit::Exited(0))));
    let session = fixture.session(OLD_VERSION).with_launcher(launcher.clone());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_console_update(&session, &mut out, &mut err, &CancellationToken::new()).await;

    assert_eq!(code, EXIT_OK, "{}", text_of(&err));
    let calls = launcher.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].archive_existed);
    assert_eq!(calls[0].value("--version"), NEW_VERSION);
    assert!(!calls[0].launch.args.iter().any(|a| a.contains("sha")));
    assert_eq!(fixture.downloads(), 1);
}

#[tokio::test]
async fn checksum_mismatch_leaves_the_installation_unchanged() {
    let fixture = Fixture::new(Os::Linux).await;
    fixture.serve(failing_archive(), &"0".repeat(64), 0);
    let before = fixture.snapshot();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_console_update(
        &fixture.session(OLD_VERSION),
        &mut out,
        &mut err,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(code, EXIT_FAILURE);
    assert_eq!(
        text_of(&err),
        format!(
            "{}\n{}\n",
            text::error_occurred(&UpdateError::Integrity),
            text::ABORTED
        )
    );
    assert_eq!(fixture.snapshot(), before);
    assert!(fixture.staging_is_empty());
}

#[tokio::test]
async fn unverified_release_and_wrong_layout_are_refused() {
    let fixture = Fixture::new(Os::Linux).await;
    let session = fixture.session(OLD_VERSION);
    let before = fixture.snapshot();
    let archive = failing_archive();
    let release = fixture.release(&archive, None);
    let err = session
        .apply(&release, &|_| {}, &CancellationToken::new())
        .await
        .expect_err("unverified");
    assert!(matches!(err, ApplyError::Unverified { .. }), "{err:?}");
    assert_eq!(fixture.downloads(), 0);

    let archive = zip_bytes(&[Entry::File("Arma3Sync-9.0.0/arma3sync", b"nested")]);
    let sha256 = sha256_hex(&archive);
    fixture.serve(archive.clone(), &sha256, 0);
    let err = session
        .apply(
            &fixture.release(&archive, Some(sha256)),
            &|_| {},
            &CancellationToken::new(),
        )
        .await
        .expect_err("layout");
    assert!(matches!(err, ApplyError::Layout(_)), "{err:?}");
    assert_eq!(fixture.snapshot(), before);
    assert!(fixture.staging_is_empty());
}

#[tokio::test]
async fn cancel_during_download() {
    let fixture = Fixture::new(Os::Linux).await;
    fixture.server.route(
        ARCHIVE_PATH,
        Reply::Slow {
            chunk: vec![1; 1024],
            interval: Duration::from_millis(100),
            count: 100,
        },
    );
    let session = fixture.session(OLD_VERSION);
    let before = fixture.snapshot();
    let mut release = fixture.release(&[], Some("0".repeat(64)));
    release.size = 100 * 1024;
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    let cancelled_at = Arc::new(parking_lot::Mutex::new(None));
    let stamp = Arc::clone(&cancelled_at);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        *stamp.lock() = Some(Instant::now());
        canceller.cancel();
    });

    let err = session
        .apply(&release, &|_| {}, &cancel)
        .await
        .expect_err("cancelled");
    let elapsed = cancelled_at.lock().expect("cancelled").elapsed();
    assert!(matches!(err, ApplyError::Cancelled), "{err:?}");
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    assert!(err.dialog().is_none());
    assert!(fixture.staging_is_empty());
    assert_eq!(fixture.snapshot(), before);
}

#[tokio::test]
async fn unknown_size_proceeds_and_a_missing_file_fails() {
    let fixture = Fixture::new(Os::Linux).await;
    let archive = crate::apply_fixture::release_zip(Os::Linux);
    let sha256 = sha256_hex(&archive);
    fixture.server.route(
        ARCHIVE_PATH,
        Reply::Chunked(archive.chunks(100).map(<[u8]>::to_vec).collect()),
    );
    let mut release = fixture.release(&archive, Some(sha256));
    release.size = 0;
    let events = Events::default();
    fixture
        .session(OLD_VERSION)
        .apply(&release, &|e| events.record(e), &CancellationToken::new())
        .await
        .expect("apply");
    let all = events.all();
    assert_eq!(all[0], UpdateEvent::Preparing);
    assert!(
        all.iter()
            .any(|e| matches!(e, UpdateEvent::Downloading { total: None, .. }))
    );
    assert_eq!(all.last(), Some(&UpdateEvent::Done));

    let mut missing = release.clone();
    missing.download_url = fixture.server.url("/dl/missing.zip");
    let err = fixture
        .session(OLD_VERSION)
        .apply(&missing, &|_| {}, &CancellationToken::new())
        .await
        .expect_err("missing");
    assert!(matches!(err, ApplyError::SizeUnknown(_)), "{err:?}");
}

#[tokio::test]
async fn one_apply_at_a_time_and_cached_checks() {
    let fixture = Fixture::new(Os::Linux).await;
    let session = fixture.session(OLD_VERSION);
    let cancel = CancellationToken::new();
    for _ in 0..2 {
        assert!(matches!(
            session.check(&cancel).await,
            Ok(CheckOutcome::Available { .. })
        ));
    }
    assert_eq!(fixture.server.hits(MANIFEST_PATH), 1);

    fixture.server.route(
        ARCHIVE_PATH,
        Reply::Slow {
            chunk: vec![1; 1024],
            interval: Duration::from_millis(100),
            count: 100,
        },
    );
    let mut release = fixture.release(&[], Some("0".repeat(64)));
    release.size = 100 * 1024;
    let first = session.apply(&release, &|_| {}, &cancel);
    tokio::pin!(first);
    tokio::select! {
        result = &mut first => panic!("finished early: {result:?}"),
        () = tokio::time::sleep(Duration::from_millis(200)) => {}
    }
    let second = session.apply(&release, &|_| {}, &cancel).await;
    assert!(matches!(second, Err(ApplyError::Busy)), "{second:?}");
    cancel.cancel();
    assert!(matches!(first.await, Err(ApplyError::Cancelled)));
}
