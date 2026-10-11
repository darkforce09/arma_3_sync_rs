//! End-to-end synchronization against a local HTTP server serving Java-built and hand-built
//! repositories.

mod download_support;
mod pipeline_support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s_foundation::error::ErrorKind;
use a3s_sync::pipeline::{
    ReconnectPolicy, RunEnd, SyncEnd, SyncJob, UserconfigOutcome, UserconfigPolicy,
};
use a3s_sync::report::ReportStatistics;
use a3s_sync::{
    DownloadStatus, EventSender, MemoryStore, Outcome, Phase, PlanNode, SyncEvent, SyncSession,
    SyncStore, sync_repository,
};
use a3s_transport::CancellationToken;
use a3s_types::constants::RepositoryStatus;
use download_support::{HttpServer, Rule, drain, random_bytes, sha1};
use pipeline_support::{
    REPOSITORY, RepoFile, check_request, count, files_below, finished, fixture, job, leaves,
    read_server_info, read_sync, repository, serve, store, write_repository,
};

/// Runs `sync_repository` and returns its report and events.
async fn sync(job: SyncJob, store: &Arc<MemoryStore>) -> (a3s_sync::SyncRunReport, Vec<SyncEvent>) {
    let (events, rx) = EventSender::channel();
    let report = sync_repository(job, store.clone(), &CancellationToken::new(), &events).await;
    (report, drain(&rx))
}

/// A checked session of the test repository with everything selected.
async fn checked_session(server: &HttpServer, store: &Arc<MemoryStore>) -> SyncSession {
    let mut session = SyncSession::check(
        check_request(server.protocol()),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");
    session.select_all();
    session
}

/// Every leaf of the served repository exists in `dest` with its SHA-1, and no archive or
/// partial file is left (a Java-built repository may publish a `.zip` as a leaf).
fn assert_in_sync(server_root: &Path, dest: &Path) {
    let leaves = leaves(&read_sync(server_root));
    for (path, leaf) in &leaves {
        let local = dest.join(path);
        assert!(local.is_file(), "{path} missing");
        assert_eq!(Some(sha1(&local)), leaf.sha1, "{path}");
    }
    let leftovers: Vec<String> = files_below(dest)
        .into_iter()
        .filter(|f| f.ends_with(".zip") || f.ends_with(".part"))
        .filter(|f| !leaves.iter().any(|(path, _)| path == f))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn java_built_compressed_repository_then_nothing_to_do() {
    let server = HttpServer::start().await;
    serve(&server.root, &fixture("http_zip", "1_initial"));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));

    let (report, events) = sync(job(server.protocol()), &store).await;
    assert_eq!(
        report.end,
        SyncEnd::Finished(Outcome::Success),
        "{report:?}"
    );
    assert_in_sync(&server.root, dest);
    let paths = server.paths();
    assert!(
        paths.iter().any(|p| p == "@Mod_A/addons/mod_a.pbo.zip"),
        "{paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == "@mod_b/Addons/mod_b.ebo.zip"),
        "{paths:?}"
    );
    for uncompressed in ["@Mod_A/addons/mod_a.pbo", "@mod_b/Addons/mod_b.ebo"] {
        assert!(
            !paths.iter().any(|p| p == uncompressed),
            "{uncompressed} fetched"
        );
    }

    assert_eq!(
        events.first(),
        Some(&SyncEvent::Phase(Phase::CheckingRepository))
    );
    assert!(events.contains(&SyncEvent::Phase(Phase::Downloading)));
    assert_eq!(finished(&events), [&Outcome::Success]);
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );
    let text = report.report_text.as_deref().expect("report");
    assert!(
        text.contains(REPOSITORY) && text.contains(&server.host_url()),
        "{text}"
    );
    let run = report.download.as_ref().expect("run");
    let tasks = run.download.as_ref().expect("download report");
    assert!(tasks.errors.is_empty());

    let server_revision = read_server_info(&server.root).revision;
    assert_eq!(report.status, Some(RepositoryStatus::Ok));
    assert_eq!(
        store.repository(REPOSITORY).expect("stored").revision,
        server_revision
    );
    assert!(
        !store
            .sync_cache(REPOSITORY)
            .expect("cache")
            .entries
            .is_empty()
    );
    assert_eq!(report.remaining_downloads, Some(0));

    // Second run: nothing to download or delete, no file request.
    let session = checked_session(&server, &store).await;
    assert!(session.summary().download_tasks.is_empty());
    assert!(session.summary().delete_tasks.is_empty());
    server.clear_seen();
    let (report, events) = sync(job(server.protocol()), &store).await;
    assert_eq!(report.end, SyncEnd::Finished(Outcome::Success));
    assert_eq!(finished(&events), [&Outcome::Success]);
    let files: Vec<String> = server
        .paths()
        .into_iter()
        .filter(|p| !p.starts_with(".a3s/"))
        .collect();
    assert!(files.is_empty(), "{files:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_with_exact_match_deletes_extras_without_following_links() {
    let server = HttpServer::start().await;
    serve(&server.root, &fixture("http_zip", "1_initial"));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path().join("dest");
    std::fs::create_dir(&dest).expect("mkdir");
    let outside = dest_dir.path().join("outside");
    std::fs::create_dir(&outside).expect("mkdir");
    std::fs::write(outside.join("keep.txt"), b"keep").expect("write");
    let store = store(repository(server.protocol(), &dest, true));
    let (report, _) = sync(job(server.protocol()), &store).await;
    assert_eq!(report.end, SyncEnd::Finished(Outcome::Success));

    std::fs::write(dest.join("extra.txt"), b"x").expect("extra");
    std::fs::create_dir_all(dest.join("@Old/addons")).expect("mkdir");
    std::fs::write(dest.join("@Old/addons/x.pbo"), b"x").expect("extra");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, dest.join("@Link")).expect("symlink");

    serve(&server.root, &fixture("http_zip", "3_modified"));
    let (report, _) = sync(job(server.protocol()), &store).await;
    assert_eq!(
        report.end,
        SyncEnd::Finished(Outcome::Success),
        "{report:?}"
    );
    assert_in_sync(&server.root, &dest);
    assert!(!dest.join("extra.txt").exists());
    assert!(!dest.join("@Old").exists());
    assert!(std::fs::symlink_metadata(dest.join("@Link")).is_err());
    assert_eq!(
        std::fs::read(outside.join("keep.txt")).expect("kept"),
        b"keep"
    );
    assert!(!dest.join("Collection/@Nested").exists());
    assert_eq!(
        std::fs::read(dest.join("@mod_b/Addons/new_empty.txt")).expect("new"),
        b""
    );
    let run = report.download.as_ref().expect("run");
    assert!(run.cleanup.refused.is_empty() && run.cleanup.failed.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn update_without_exact_match_keeps_root_extras() {
    let server = HttpServer::start().await;
    serve(&server.root, &fixture("http_zip", "1_initial"));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, false));
    let (report, _) = sync(job(server.protocol()), &store).await;
    assert_eq!(report.end, SyncEnd::Finished(Outcome::Success));

    std::fs::write(dest.join("extra.txt"), b"x").expect("extra");
    std::fs::create_dir_all(dest.join("@Old/addons")).expect("mkdir");
    std::fs::write(dest.join("@Old/addons/x.pbo"), b"x").expect("extra");
    std::fs::write(dest.join("@Mod_A/addons/stale.pbo"), b"x").expect("extra");

    serve(&server.root, &fixture("http_zip", "3_modified"));
    let (report, _) = sync(job(server.protocol()), &store).await;
    assert_eq!(
        report.end,
        SyncEnd::Finished(Outcome::Success),
        "{report:?}"
    );
    assert_in_sync(&server.root, dest);
    assert!(dest.join("extra.txt").exists());
    assert!(dest.join("@Old/addons/x.pbo").exists());
    // Remote folders are always scanned (Java quirk).
    assert!(!dest.join("@Mod_A/addons/stale.pbo").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partial_transfer_patches_with_ranges() {
    let server = HttpServer::start().await;
    serve(&server.root, &fixture("http_plain", "1_initial"));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));
    let (report, _) = sync(job(server.protocol()), &store).await;
    assert_eq!(report.end, SyncEnd::Finished(Outcome::Success));

    serve(&server.root, &fixture("http_plain", "3_modified"));
    const PATCHED: &str = "@Mod_A/addons/mod_a.pbo";
    let complete = |session: &SyncSession| match session.tree().get(PATCHED) {
        Some(PlanNode::Leaf(leaf)) => leaf.complete,
        other => panic!("{other:?}"),
    };
    let mut session = checked_session(&server, &store).await;
    if complete(&session) == 0.0 {
        // The versions share no block: start from the new file with its first block changed.
        let mut seeded = std::fs::read(server.root.join(PATCHED)).expect("new");
        seeded[..8192].fill(0xA5);
        std::fs::write(dest.join(PATCHED), &seeded).expect("seed");
        session = checked_session(&server, &store).await;
    }
    assert!(complete(&session) > 0.0);

    server.clear_seen();
    let (events, _rx) = EventSender::channel();
    let run = session.download(&CancellationToken::new(), &events).await;
    assert_eq!(run.end, RunEnd::Finished(Outcome::Success), "{run:?}");
    assert_eq!(sha1(&dest.join(PATCHED)), sha1(&server.root.join(PATCHED)));
    let requests: Vec<_> = server
        .seen()
        .into_iter()
        .filter(|s| s.path.starts_with(PATCHED))
        .collect();
    assert!(
        requests
            .iter()
            .any(|s| s.path == format!("{PATCHED}.zsync"))
    );
    let file_requests: Vec<_> = requests.iter().filter(|s| s.path == PATCHED).collect();
    assert!(!file_requests.is_empty());
    assert!(
        file_requests.iter().all(|s| s.range.is_some()),
        "{file_requests:?}"
    );
    let statistics = ReportStatistics::compute(&session.summary().download_tasks);
    assert_eq!(statistics.partial.files, 1);
}

/// A hand-built repository: a big plain file and a small compressed one.
fn big_and_small(root: &Path) -> (Vec<u8>, Vec<u8>) {
    let big = random_bytes(3 * 1024 * 1024, 11);
    let small = random_bytes(40 * 1024, 12);
    write_repository(
        root,
        &[
            RepoFile {
                path: "@a/addons/big.pbo",
                content: &big,
                compressed: false,
            },
            RepoFile {
                path: "@a/addons/small.pbo",
                content: &small,
                compressed: true,
            },
        ],
        7,
    );
    (big, small)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paused_session_resumes() {
    let server = Arc::new(HttpServer::start().await);
    let (big, small) = big_and_small(&server.root);
    server.rule(
        "@a/addons/big.pbo",
        Rule {
            stall_after: Some(256 * 1024),
            ..Rule::default()
        },
    );
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));
    let mut session = SyncSession::check(
        check_request(server.protocol()),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");
    session.select_all();

    // Pause once the small file is downloaded and the big one is stalled.
    let cancel = CancellationToken::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let small_done = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&small_done);
    let events = EventSender::from_fn(move |event| {
        if matches!(&event, SyncEvent::FileDone { relative_path } if relative_path.ends_with("small.pbo"))
        {
            notify.notify_one();
        }
        tx.send(event).ok();
    });
    let pause = async {
        small_done.notified().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    };
    let (run, ()) = tokio::join!(session.download(&cancel, &events), pause);
    let first = drain(&rx);
    assert!(
        std::fs::metadata(dest.join("@a/addons/big.pbo.part")).is_ok_and(|m| m.len() > 0),
        "the big file is partly downloaded"
    );
    assert_eq!(run.end, RunEnd::Finished(Outcome::Cancelled));
    assert_eq!(finished(&first), [&Outcome::Cancelled]);
    assert!(run.report_text.is_none() && session.last_report().is_none());
    assert_eq!(store.repository(REPOSITORY).expect("stored").revision, 0);
    let big_task = session
        .summary()
        .download_tasks
        .iter()
        .find(|t| t.relative_path.ends_with("big.pbo"))
        .expect("big task");
    assert_eq!(big_task.status, DownloadStatus::Running);

    // Resume.
    server.clear_rules();
    server.clear_seen();
    let (events, rx) = EventSender::channel();
    let run = session.download(&CancellationToken::new(), &events).await;
    assert_eq!(run.end, RunEnd::Finished(Outcome::Success), "{run:?}");
    assert_eq!(finished(&drain(&rx)), [&Outcome::Success]);
    let seen = server.seen();
    let big_requests: Vec<_> = seen
        .iter()
        .filter(|s| s.path.ends_with("big.pbo"))
        .collect();
    assert_eq!(big_requests.len(), 1, "{seen:?}");
    assert!(big_requests[0].range.is_some(), "{seen:?}");
    assert!(!seen.iter().any(|s| s.path.contains("small")), "{seen:?}");
    assert_eq!(
        std::fs::read(dest.join("@a/addons/big.pbo")).expect("big"),
        big
    );
    assert_eq!(
        std::fs::read(dest.join("@a/addons/small.pbo")).expect("small"),
        small
    );
    assert!(session.last_report().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn leftover_archive_is_uncompressed_without_a_request() {
    let server = HttpServer::start().await;
    let (_, small) = big_and_small(&server.root);
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));
    let mut session = SyncSession::check(
        check_request(server.protocol()),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");
    // A previous run downloaded the archive but did not uncompress it.
    std::fs::create_dir_all(dest.join("@a/addons")).expect("mkdir");
    std::fs::copy(
        server.root.join("@a/addons/small.pbo.zip"),
        dest.join("@a/addons/small.pbo.zip"),
    )
    .expect("copy");
    session.select_all();
    if let Some(PlanNode::Leaf(leaf)) = session.tree_mut().get_mut("@a/addons/small.pbo") {
        leaf.download_status = DownloadStatus::Done;
    }
    session.refresh_summary();
    server.clear_seen();
    let run = session
        .download(&CancellationToken::new(), &EventSender::noop())
        .await;
    assert_eq!(run.end, RunEnd::Finished(Outcome::Success), "{run:?}");
    assert_eq!(run.unzip.extracted, 1);
    assert!(!server.paths().iter().any(|p| p.contains("small")));
    assert_eq!(
        std::fs::read(dest.join("@a/addons/small.pbo")).expect("small"),
        small
    );
    assert!(!dest.join("@a/addons/small.pbo.zip").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn errors_mark_the_repository_failed() {
    // A missing file.
    let server = HttpServer::start().await;
    let small = random_bytes(10_000, 21);
    let packed = random_bytes(30_000, 22);
    write_repository(
        &server.root,
        &[
            RepoFile {
                path: "@a/addons/gone.pbo",
                content: &small,
                compressed: false,
            },
            RepoFile {
                path: "@a/addons/packed.pbo",
                content: &packed,
                compressed: true,
            },
        ],
        3,
    );
    std::fs::remove_file(server.root.join("@a/addons/gone.pbo")).expect("remove");
    // Garbage of the right size instead of the archive.
    let zip_len = std::fs::metadata(server.root.join("@a/addons/packed.pbo.zip"))
        .expect("zip")
        .len();
    server.put("@a/addons/packed.pbo.zip", &vec![0x42; zip_len as usize]);
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));
    let (report, events) = sync(job(server.protocol()), &store).await;
    let SyncEnd::Finished(Outcome::CompletedWithErrors { errors }) = &report.end else {
        panic!("{report:?}");
    };
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(
        errors[0].relative_path.as_deref(),
        Some("@a/addons/gone.pbo")
    );
    assert_eq!(errors[0].kind, ErrorKind::NotFound);
    assert_eq!(
        errors[1].relative_path.as_deref(),
        Some("@a/addons/packed.pbo")
    );
    assert_eq!(errors[1].kind, ErrorKind::InvalidData);
    let text = report.report_text.as_deref().expect("report");
    assert!(errors.iter().all(|e| text.contains(&e.message)), "{text}");
    assert_eq!(finished(&events).len(), 1);
    assert_eq!(report.status, Some(RepositoryStatus::Error));
    assert_eq!(store.repository(REPOSITORY).expect("stored").revision, -1);
    // No second check.
    let sync_fetches = server
        .seen()
        .iter()
        .filter(|s| s.path == ".a3s/sync" && s.range.is_none())
        .count();
    assert_eq!(sync_fetches, 1);
    assert!(!dest.join("@a/addons/packed.pbo").exists());

    // The destination vanished between the check and the download.
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path().join("dest");
    std::fs::create_dir(&dest).expect("mkdir");
    let store = self::store(repository(server.protocol(), &dest, true));
    let mut session = SyncSession::check(
        check_request(server.protocol()),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");
    session.select_all();
    std::fs::remove_dir(&dest).expect("remove");
    server.clear_seen();
    let (events, rx) = EventSender::channel();
    let run = session.download(&CancellationToken::new(), &events).await;
    match &run.end {
        RunEnd::Finished(Outcome::Failed { error }) => assert_eq!(error.kind, ErrorKind::NotFound),
        other => panic!("{other:?}"),
    }
    assert!(run.report_text.is_some());
    assert_eq!(finished(&drain(&rx)).len(), 1);
    assert!(server.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_lost_without_and_with_reconnect() {
    let server = Arc::new(HttpServer::start().await);
    let (big, _) = big_and_small(&server.root);
    let drop_every_response = Rule {
        drop_after: Some(1024),
        ..Rule::default()
    };

    // No resume left: `ConnectionLost` is the last event.
    server.rule("@a/addons/big.pbo", drop_every_response.clone());
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let store = store(repository(server.protocol(), dest_dir.path(), true));
    let (report, events) = sync(job(server.protocol()), &store).await;
    assert_eq!(report.end, SyncEnd::ConnectionLost, "{report:?}");
    assert_eq!(events.last(), Some(&SyncEvent::ConnectionLost));
    assert!(finished(&events).is_empty());
    assert_eq!(report.status, Some(RepositoryStatus::Error));
    assert_eq!(store.repository(REPOSITORY).expect("stored").revision, -1);

    // One resume, the server recovers meanwhile.
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = self::store(repository(server.protocol(), dest, true));
    let (tx, rx) = std::sync::mpsc::channel();
    let recovering = Arc::clone(&server);
    let events = EventSender::from_fn(move |event| {
        if event == SyncEvent::ConnectionLost {
            recovering.clear_rules();
        }
        tx.send(event).ok();
    });
    let mut job = job(server.protocol());
    job.reconnect = ReconnectPolicy {
        max_attempts: 1,
        delay: Duration::from_millis(100),
    };
    let report = sync_repository(job, store.clone(), &CancellationToken::new(), &events).await;
    let events = drain(&rx);
    assert_eq!(
        report.end,
        SyncEnd::Finished(Outcome::Success),
        "{report:?}"
    );
    assert_eq!(report.reconnects, 1);
    assert_eq!(count(&events, |e| *e == SyncEvent::ConnectionLost), 1);
    assert_eq!(finished(&events), [&Outcome::Success]);
    assert_eq!(
        std::fs::read(dest.join("@a/addons/big.pbo")).expect("big"),
        big
    );
    assert_eq!(report.status, Some(RepositoryStatus::Ok));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn userconfig_is_copied_after_success() {
    let server = HttpServer::start().await;
    write_repository(
        &server.root,
        &[
            RepoFile {
                path: "userconfig/ace/ace_settings.hpp",
                content: b"force ace_setting = 1;",
                compressed: false,
            },
            RepoFile {
                path: "@a/addons/a.pbo",
                content: b"pbo",
                compressed: false,
            },
        ],
        1,
    );
    let arma3 = tempfile::tempdir().expect("tempdir");
    let exe = arma3.path().join("arma3_x64.exe");
    let target = arma3.path().join("userconfig/ace/ace_settings.hpp");
    let policies = [
        (UserconfigPolicy::Skip, false),
        (UserconfigPolicy::Ask(Arc::new(|_| false)), false),
        (UserconfigPolicy::Silent, true),
    ];
    for (policy, copied) in policies {
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let store = store(repository(server.protocol(), dest_dir.path(), true));
        let mut job = job(server.protocol());
        job.arma3_exe_path = Some(exe.clone());
        job.userconfig = policy.clone();
        let (report, events) = sync(job, &store).await;
        assert_eq!(
            report.end,
            SyncEnd::Finished(Outcome::Success),
            "{report:?}"
        );
        let phase = events.contains(&SyncEvent::Phase(Phase::UpdatingUserconfig));
        assert_eq!(phase, copied, "{policy:?}");
        assert_eq!(target.exists(), copied, "{policy:?}");
        match policy {
            UserconfigPolicy::Skip => assert_eq!(report.userconfig, None),
            UserconfigPolicy::Ask(_) => {
                assert_eq!(report.userconfig, Some(UserconfigOutcome::Declined));
            }
            UserconfigPolicy::Silent => {
                assert!(matches!(
                    report.userconfig,
                    Some(UserconfigOutcome::Copied(_))
                ));
                assert_eq!(
                    std::fs::read(&target).expect("copied"),
                    b"force ace_setting = 1;"
                );
            }
        }
    }
}

#[test]
fn futures_and_pool_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<a3s_sync::UnzipPool>();
    assert_send_sync::<MemoryStore>();
    let pool: Arc<dyn a3s_sync::download::ZipSink> = a3s_sync::UnzipPool::with_workers(1);
    drop(pool);

    let store: Arc<dyn SyncStore> = Arc::new(MemoryStore::default());
    let protocol = a3s_types::Protocol::new(
        a3s_types::ProtocolType::Http,
        "127.0.0.1/repo",
        "1",
        "anonymous",
        "",
        false,
    );
    let cancel = CancellationToken::new();
    let events = EventSender::noop();
    let check = SyncSession::check(
        check_request(protocol.clone()),
        store.clone(),
        &cancel,
        &events,
    );
    assert_send(&check);
    let synced = sync_repository(job(protocol), store, &cancel, &events);
    assert_send(&synced);
}

/// Compiles only if a download run is `Send`.
#[allow(dead_code)]
fn download_is_send(session: &mut SyncSession, cancel: &CancellationToken, events: &EventSender) {
    fn assert_send<T: Send>(_: &T) {}
    assert_send(session);
    assert_send(&session.download(cancel, events));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_while_uncompressing_then_resume_at_once() {
    let server = HttpServer::start().await;
    // 128 MiB of zeros: a small archive that takes a while to extract.
    let big = vec![0u8; 128 * 1024 * 1024];
    write_repository(
        &server.root,
        &[RepoFile {
            path: "@a/addons/big.pbo",
            content: &big,
            compressed: true,
        }],
        5,
    );
    let leaf_sha1 = sha1(&server.root.join("@a/addons/big.pbo"));
    drop(big);
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));
    let mut session = checked_session(&server, &store).await;

    // Cancel as soon as the archive is downloaded: its extraction has just started.
    let cancel = CancellationToken::new();
    let pause = cancel.clone();
    let cancelled_at = Arc::new(parking_lot::Mutex::new(None));
    let stamp = Arc::clone(&cancelled_at);
    let events = EventSender::from_fn(move |event| {
        if matches!(&event, SyncEvent::FileDone { relative_path } if relative_path.ends_with("big.pbo"))
        {
            *stamp.lock() = Some(std::time::Instant::now());
            pause.cancel();
        }
    });
    let run = session.download(&cancel, &events).await;
    let returned = std::time::Instant::now();
    assert_eq!(run.end, RunEnd::Finished(Outcome::Cancelled), "{run:?}");
    let cancelled_at = cancelled_at.lock().expect("file done");
    assert!(returned.duration_since(cancelled_at) < Duration::from_secs(1));
    assert!(
        dest.join("@a/addons/big.pbo.zip").exists(),
        "the interrupted archive is kept"
    );
    assert!(!dest.join("@a/addons/big.pbo.part").exists());

    // Resume immediately: the kept archive is extracted, not downloaded again.
    server.clear_seen();
    let run = session
        .download(&CancellationToken::new(), &EventSender::noop())
        .await;
    assert_eq!(run.end, RunEnd::Finished(Outcome::Success), "{run:?}");
    assert_eq!(run.unzip.extracted, 1);
    assert!(server.seen().is_empty(), "{:?}", server.seen());
    assert_eq!(sha1(&dest.join("@a/addons/big.pbo")), leaf_sha1);
    assert!(!dest.join("@a/addons/big.pbo.zip").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_archive_is_downloaded_again_by_the_next_run() {
    let server = HttpServer::start().await;
    let packed = random_bytes(30_000, 31);
    write_repository(
        &server.root,
        &[RepoFile {
            path: "@a/addons/packed.pbo",
            content: &packed,
            compressed: true,
        }],
        2,
    );
    let zip_path = server.root.join("@a/addons/packed.pbo.zip");
    let good = std::fs::read(&zip_path).expect("zip");
    server.put("@a/addons/packed.pbo.zip", &vec![0x42; good.len()]);
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));
    let mut session = checked_session(&server, &store).await;

    let run = session
        .download(&CancellationToken::new(), &EventSender::noop())
        .await;
    assert!(
        matches!(&run.end, RunEnd::Finished(Outcome::CompletedWithErrors { errors }) if errors.len() == 1),
        "{run:?}"
    );
    assert_eq!(run.unzip.errors.len(), 1);
    let packed_task = session
        .summary()
        .download_tasks
        .iter()
        .find(|t| t.relative_path == "@a/addons/packed.pbo")
        .expect("task");
    assert_eq!(packed_task.status, DownloadStatus::Pending);

    server.put("@a/addons/packed.pbo.zip", &good);
    server.clear_seen();
    let run = session
        .download(&CancellationToken::new(), &EventSender::noop())
        .await;
    assert_eq!(run.end, RunEnd::Finished(Outcome::Success), "{run:?}");
    assert!(
        server
            .paths()
            .iter()
            .any(|p| p == "@a/addons/packed.pbo.zip")
    );
    assert_eq!(
        std::fs::read(dest.join("@a/addons/packed.pbo")).expect("file"),
        packed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn synchronization_goes_through_the_configured_proxy() {
    // The repository host does not resolve: only the proxy (the local server, which answers
    // absolute-form requests) can serve it.
    let server = HttpServer::start().await;
    write_repository(
        &server.root,
        &[RepoFile {
            path: "@a/addons/a.pbo",
            content: b"through the proxy",
            compressed: false,
        }],
        4,
    );
    let protocol = a3s_types::Protocol::new(
        a3s_types::ProtocolType::Http,
        "a3s-proxy-test.invalid/repo",
        "80",
        "anonymous",
        "",
        false,
    );
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(protocol.clone(), dest, true));
    let mut configuration = store.configuration();
    configuration.proxy = a3s_types::local::Proxy {
        enable_proxy: true,
        proxy_protocol: Some(a3s_types::Protocol::new(
            a3s_types::ProtocolType::Http,
            "127.0.0.1",
            server.port.to_string(),
            "anonymous",
            "",
            false,
        )),
    };
    store.set_configuration(configuration.clone());

    let (report, _) = sync(job(protocol.clone()), &store).await;
    assert_eq!(
        report.end,
        SyncEnd::Finished(Outcome::Success),
        "{report:?}"
    );
    assert_eq!(
        std::fs::read(dest.join("@a/addons/a.pbo")).expect("file"),
        b"through the proxy"
    );
    assert!(server.paths().iter().any(|p| p == "@a/addons/a.pbo"));

    // An unusable proxy fails the check before any connection.
    if let Some(proxy) = configuration.proxy.proxy_protocol.as_mut() {
        proxy.port = "0".to_owned();
    }
    store.set_configuration(configuration);
    let checked = SyncSession::check(
        check_request(protocol),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await;
    assert!(matches!(checked, Err(a3s_sync::CheckError::Proxy(_))));
}
