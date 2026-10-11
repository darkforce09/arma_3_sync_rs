//! Download executor against a local range-capable HTTP server.

mod download_support;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_foundation::error::ErrorKind;
use a3s_sync::download::{
    CollectZips, DownloadEnd, DownloadReport, DownloadRequest, MAX_RESUME_RETRIES, NoZipSink,
    ZipSink, download,
};
use a3s_sync::{DownloadStatus, DownloadTask, EventError, EventSender, Outcome, Phase, SyncEvent};
use a3s_transport::CancellationToken;
use a3s_types::{Protocol, ProtocolType};
use a3s_zsync::{ControlFile, Plan};
use download_support::{
    BusySink, HttpServer, Rule, compressed_task, dir_task, drain, file_task, part_files,
    random_bytes, request, sha1, with_complete,
};
use parking_lot::Mutex;

/// Runs `download` and returns the report and every event.
async fn run(
    request: &DownloadRequest,
    tasks: &mut [DownloadTask],
    zips: Arc<dyn ZipSink>,
    cancel: &CancellationToken,
) -> (DownloadReport, Vec<SyncEvent>) {
    let (events, rx) = EventSender::channel();
    let report = download(request, tasks, zips, cancel, &events).await;
    (report, drain(&rx))
}

fn count<F: Fn(&SyncEvent) -> bool>(events: &[SyncEvent], f: F) -> usize {
    events.iter().filter(|e| f(e)).count()
}

fn no_finished(events: &[SyncEvent]) {
    assert_eq!(count(events, |e| matches!(e, SyncEvent::Finished(_))), 0);
}

fn file_failed(events: &[SyncEvent]) -> usize {
    count(events, |e| matches!(e, SyncEvent::FileFailed { .. }))
}

/// Runs `download` and cancels `cancel` as soon as a `Transfer` event reports at least
/// `transferred` bytes. Returns the report, every event and the moment of the cancel.
async fn run_cancelled_after(
    request: &DownloadRequest,
    tasks: &mut [DownloadTask],
    cancel: &CancellationToken,
    transferred: u64,
) -> (DownloadReport, Vec<SyncEvent>, Instant) {
    let events: Arc<Mutex<Vec<SyncEvent>>> = Arc::default();
    let cancelled_at: Arc<Mutex<Option<Instant>>> = Arc::default();
    let sender = {
        let events = Arc::clone(&events);
        let cancelled_at = Arc::clone(&cancelled_at);
        let cancel = cancel.clone();
        EventSender::from_fn(move |event| {
            if let SyncEvent::Transfer(progress) = &event
                && progress.transferred >= transferred
            {
                cancelled_at.lock().get_or_insert_with(|| {
                    cancel.cancel();
                    Instant::now()
                });
            }
            events.lock().push(event);
        })
    };
    let download = download(request, tasks, Arc::new(NoZipSink), cancel, &sender);
    let report = tokio::time::timeout(Duration::from_secs(30), download)
        .await
        .expect("the transfer reached the expected bytes");
    let cancelled_at = cancelled_at.lock().expect("cancelled");
    let events = std::mem::take(&mut *events.lock());
    (report, events, cancelled_at)
}

/// Waits until `path` has `len` bytes: writes of a cancelled download may still be landing.
async fn wait_for_len(path: &Path, len: u64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) != len {
        assert!(
            Instant::now() < deadline,
            "{} never reached {len} bytes",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn cancel_after(token: &CancellationToken, delay: Duration) -> tokio::task::JoinHandle<Instant> {
    let token = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        token.cancel();
        Instant::now()
    })
}

// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn basic_run_with_four_connections() {
    let server = HttpServer::start().await;
    server.ok_delay(Duration::from_millis(30));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();

    let mut tasks = vec![dir_task(dest, "@a/userconfig")];
    let mut server_bytes = 0u64;
    for i in 0..20u64 {
        let size = i * 300 * 1024 / 19;
        let relative = format!("@a/addons/f{i:02}.pbo");
        server.put(&relative, &random_bytes(size as usize, i));
        server_bytes += size;
        tasks.push(file_task(dest, &relative, size));
    }
    let x_zip = random_bytes(3000, 100);
    server.put("@a/addons/x.pbo.zip", &x_zip);
    tasks.push(compressed_task(
        dest,
        "@a/addons/x.pbo",
        5000,
        3000,
        "sha-x",
    ));
    let y_zip = random_bytes(2000, 101);
    server.put("@a/addons/y.ebo.zip", &y_zip);
    tasks.push(compressed_task(
        dest,
        "@a/addons/y.ebo",
        4000,
        2000,
        "sha-y",
    ));
    server_bytes += 5000;
    let mut done = file_task(dest, "@a/addons/done.pbo", 1234);
    done.status = DownloadStatus::Done;
    tasks.push(done);
    let expected_total: u64 = tasks.iter().map(DownloadTask::download_size).sum();

    let zips = Arc::new(CollectZips::new());
    let cancel = CancellationToken::new();
    let (report, events) = run(
        &request(server.protocol(), 4),
        &mut tasks,
        zips.clone(),
        &cancel,
    )
    .await;

    // Tasks and files.
    assert!(
        tasks.iter().all(|t| t.status == DownloadStatus::Done),
        "{tasks:#?}"
    );
    for i in 0..20u64 {
        let relative = format!("@a/addons/f{i:02}.pbo");
        assert_eq!(
            sha1(&dest.join(&relative)),
            sha1(&server.path(&relative)),
            "{relative}"
        );
    }
    let addons = dest.join("@a/addons");
    assert_eq!(
        std::fs::read(addons.join("x.pbo.zip")).expect("x zip"),
        x_zip
    );
    assert_eq!(
        std::fs::read(addons.join("y.ebo.zip")).expect("y zip"),
        y_zip
    );
    assert!(!addons.join("x.pbo").exists());
    assert!(!addons.join("y.ebo").exists());
    assert!(part_files(dest).is_empty());
    assert!(dest.join("@a/userconfig").is_dir());

    let mut submitted = zips.take();
    submitted.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0].relative_path, "@a/addons/x.pbo");
    assert_eq!(submitted[0].zip_path, addons.join("x.pbo.zip"));
    assert_eq!(submitted[0].target_path, addons.join("x.pbo"));
    assert_eq!(submitted[0].sha1.as_deref(), Some("sha-x"));
    assert_eq!(submitted[0].size, 5000);
    assert_eq!(submitted[1].relative_path, "@a/addons/y.ebo");
    assert_eq!(submitted[1].zip_path, addons.join("y.ebo.zip"));
    assert_eq!(report.downloaded_zips.len(), 2);

    // Requests.
    let seen = server.seen();
    assert!(
        seen.iter()
            .all(|s| s.user_agent.as_deref() == Some("ArmA3Sync"))
    );
    assert!(seen.iter().all(|s| s.authorization.is_none()));
    assert!(!seen.iter().any(|s| s.path.contains("userconfig")));
    assert!(!seen.iter().any(|s| s.path.contains("done.pbo")));
    assert_eq!(seen.len(), 22);
    let max = server.max_in_flight();
    assert!((2..=4).contains(&max), "max in flight {max}");

    // Events.
    assert_eq!(events[0], SyncEvent::Phase(Phase::Downloading));
    assert_eq!(
        events[1],
        SyncEvent::TransferTotal {
            bytes: expected_total,
            files: 23
        }
    );
    for task in &tasks[..tasks.len() - 1] {
        let started = events.iter().position(
            |e| matches!(e, SyncEvent::FileStarted { relative_path } if *relative_path == task.relative_path),
        );
        let finished = events.iter().position(
            |e| matches!(e, SyncEvent::FileDone { relative_path } if *relative_path == task.relative_path),
        );
        assert!(
            started.is_some() && started < finished,
            "{}",
            task.relative_path
        );
    }
    let active: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::ActiveConnections { active } => Some(*active),
            _ => None,
        })
        .collect();
    assert!(active.iter().all(|&a| a <= 4));
    assert_eq!(active.last(), Some(&0));
    let last_transfer = events
        .iter()
        .rev()
        .find_map(|e| match e {
            SyncEvent::Transfer(p) => Some(p.clone()),
            _ => None,
        })
        .expect("transfer");
    assert_eq!(last_transfer.transferred, last_transfer.total);
    assert_eq!(last_transfer.total, expected_total);
    no_finished(&events);
    assert_eq!(file_failed(&events), 0);

    // Report.
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(report.errors.is_empty());
    assert_eq!(report.files_done, 23);
    assert_eq!(report.bytes_received, server_bytes);
    assert!((2..=4).contains(&report.max_active_connections));
    assert_eq!(report.outcome(vec![]), Some(Outcome::Success));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lifo_order_and_worker_count() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..5 {
        let relative = format!("@a/f{i}.pbo");
        server.put(&relative, &random_bytes(1000, i));
        tasks.push(file_task(dest, &relative, 1000));
    }
    let cancel = CancellationToken::new();
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    let expected: Vec<String> = (0..5).rev().map(|i| format!("@a/f{i}.pbo")).collect();
    assert_eq!(server.paths(), expected);

    // connections = 3 with 2 pending files: at most 2 concurrent requests.
    server.clear_seen();
    server.ok_delay(Duration::from_millis(200));
    let mut tasks: Vec<_> = tasks.into_iter().take(2).collect();
    for task in &mut tasks {
        task.status = DownloadStatus::Pending;
    }
    let (report, _) = run(
        &request(server.protocol(), 3),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(server.max_in_flight(), 2);
    assert_eq!(report.max_active_connections, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn resume_within_a_session() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(400 * 1024, 7);
    server.put("@a/big.pbo", &content);
    server.rule(
        "@a/big.pbo",
        Rule {
            stall_after: Some(100 * 1024),
            ..Rule::default()
        },
    );
    let mut tasks = vec![file_task(dest, "@a/big.pbo", content.len() as u64)];
    let part = dest.join("@a/big.pbo.part");
    let target = dest.join("@a/big.pbo");

    // 1. Stalled transfer, cancelled once everything before the stall arrived.
    let cancel = CancellationToken::new();
    let (report, events, cancelled_at) = run_cancelled_after(
        &request(server.protocol(), 1),
        &mut tasks,
        &cancel,
        100 * 1024,
    )
    .await;
    let returned = Instant::now();
    assert!(returned.duration_since(cancelled_at) < Duration::from_secs(1));
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    assert_eq!(tasks[0].status, DownloadStatus::Running);
    wait_for_len(&part, 100 * 1024).await;
    let part_len = std::fs::metadata(&part).expect("part kept").len();
    assert_eq!(file_failed(&events), 0);
    assert_eq!(report.outcome(vec![]), Some(Outcome::Cancelled));

    // 2. Resumed with a normal server.
    server.clear_rules();
    server.clear_seen();
    let cancel = CancellationToken::new();
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].range.as_deref(),
        Some(format!("bytes={part_len}-").as_str())
    );
    assert_eq!(std::fs::read(&target).expect("final"), content);
    assert!(!part.exists());
    assert_eq!(report.bytes_received, content.len() as u64 - part_len);
    let started = events
        .iter()
        .position(|e| matches!(e, SyncEvent::FileStarted { .. }))
        .expect("started");
    let first = events[started..]
        .iter()
        .find_map(|e| match e {
            SyncEvent::Transfer(p) => Some(p.transferred),
            _ => None,
        })
        .expect("transfer");
    assert!(
        first >= part_len,
        "first snapshot {first} < offset {part_len}"
    );

    // 3. A leftover `.part` with a `Pending` task is discarded.
    std::fs::write(&part, &content[..100 * 1024]).expect("write part");
    std::fs::remove_file(&target).expect("remove final");
    tasks[0].status = DownloadStatus::Pending;
    server.clear_seen();
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].range, None);
    assert_eq!(std::fs::read(&target).expect("final"), content);
    assert_eq!(tasks[0].status, DownloadStatus::Done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mid_body_drop_resumes_then_connection_lost() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();

    // One drop: resumed from the partial file.
    let content = random_bytes(256 * 1024, 3);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            drop_after: Some(64 * 1024),
            drop_first_only: true,
            ..Rule::default()
        },
    );
    let mut tasks = vec![file_task(dest, "@a/x.pbo", content.len() as u64)];
    let cancel = CancellationToken::new();
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].range, None);
    let offset: u64 = seen[1]
        .range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|r| r.parse().ok())
        .expect("open-ended range");
    assert!(offset > 0 && offset <= 64 * 1024, "offset {offset}");
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), content);
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );

    // A drop on every response: 1 + MAX_RESUME_RETRIES requests, then connection lost.
    server.clear_seen();
    let small = random_bytes(10 * 1024, 4);
    server.put("@a/y.pbo", &small);
    server.rule(
        "@a/y.pbo",
        Rule {
            drop_after: Some(1024),
            ..Rule::default()
        },
    );
    let mut tasks = vec![file_task(dest, "@a/y.pbo", small.len() as u64)];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    let seen = server.seen();
    assert_eq!(seen.len(), 1 + MAX_RESUME_RETRIES as usize, "{seen:#?}");
    let offsets: Vec<u64> = seen
        .iter()
        .map(|s| {
            s.range
                .as_deref()
                .and_then(|r| r.strip_prefix("bytes="))
                .and_then(|r| r.strip_suffix('-'))
                .map_or(0, |r| r.parse().expect("offset"))
        })
        .collect();
    assert!(offsets.windows(2).all(|w| w[0] < w[1]), "{offsets:?}");
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        1
    );
    assert!(
        matches!(report.end, DownloadEnd::ConnectionLost { .. }),
        "{:?}",
        report.end
    );
    assert_eq!(tasks[0].status, DownloadStatus::Running);
    assert!(report.errors.is_empty());
    assert_eq!(file_failed(&events), 0);
    assert_eq!(report.outcome(vec![]), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn file_errors_and_too_many_errors() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let cancel = CancellationToken::new();

    // 3 of 6 files missing.
    let mut tasks = Vec::new();
    for i in 0..6 {
        let relative = format!("@a/f{i}.pbo");
        if i % 2 == 0 {
            server.put(&relative, &random_bytes(500, i));
        }
        tasks.push(file_task(dest, &relative, 500));
    }
    let (report, events) = run(
        &request(server.protocol(), 2),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(report.errors.len(), 3);
    for error in &report.errors {
        assert_eq!(error.error.kind(), ErrorKind::NotFound);
    }
    let failed: Vec<EventError> = events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::FileFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(failed, report.event_errors());
    match report.outcome(vec![]) {
        Some(Outcome::CompletedWithErrors { errors }) => assert_eq!(errors.len(), 3),
        other => panic!("{other:?}"),
    }
    let unzip = EventError::new(ErrorKind::InvalidData, "unzip failed");
    match report.outcome(vec![unzip]) {
        Some(Outcome::CompletedWithErrors { errors }) => assert_eq!(errors.len(), 4),
        other => panic!("{other:?}"),
    }

    // 15 missing files, K = 1: stop after exactly 10.
    let missing: Vec<DownloadTask> = (0..15)
        .map(|i| file_task(dest, &format!("@m/f{i:02}.pbo"), 10))
        .collect();
    server.clear_seen();
    let mut tasks = missing.clone();
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::TooManyErrors { limit: 10 }),
        "{:?}",
        report.end
    );
    assert_eq!(report.errors.len(), 10);
    assert_eq!(server.seen().len(), 10);
    assert_eq!(
        tasks
            .iter()
            .filter(|t| t.status == DownloadStatus::Pending)
            .count(),
        5
    );
    match report.outcome(vec![]) {
        Some(Outcome::TooManyErrors { limit, errors }) => {
            assert_eq!((limit, errors.len()), (10, 10));
        }
        other => panic!("{other:?}"),
    }

    // K = 3: between 10 and 12 errors, nothing starts after the end.
    server.clear_seen();
    server.ok_delay(Duration::ZERO);
    server.default_rule(Rule {
        delay: Some(Duration::from_millis(20)),
        ..Rule::default()
    });
    let mut tasks = missing.clone();
    let (report, _) = run(
        &request(server.protocol(), 3),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(
        report.end,
        DownloadEnd::TooManyErrors { limit: 10 }
    ));
    assert!(
        (10..=12).contains(&report.errors.len()),
        "{}",
        report.errors.len()
    );
    let after_end = server.seen().len();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(server.seen().len(), after_end);
    server.clear_rules();

    // A compressed task whose `.zip` is missing names the `.zip`.
    let mut tasks = vec![compressed_task(dest, "@z/x.pbo", 10, 5, "s")];
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert_eq!(report.errors.len(), 1);
    let message = report.errors[0].error.to_string();
    assert!(
        message.ends_with("\nRemote file not found: /@z/x.pbo.zip"),
        "{message}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_failures() {
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let cancel = CancellationToken::new();

    // Closed port.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let protocol = Protocol::new(
        ProtocolType::Http,
        "127.0.0.1/repo",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let mut tasks = vec![
        file_task(dest, "@a/x.pbo", 10),
        file_task(dest, "@a/y.pbo", 10),
    ];
    let (report, events) = run(
        &request(protocol, 2),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::ConnectionLost { .. }),
        "{:?}",
        report.end
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        1
    );
    assert!(report.errors.is_empty());
    assert_eq!(report.outcome(vec![]), None);
    no_finished(&events);

    // Insecure TLS to a non-local host is refused before any I/O.
    let protocol = Protocol::new(
        ProtocolType::Https,
        "a3s-test.invalid/repo",
        "443",
        "anonymous",
        "",
        false,
    );
    let mut tasks = vec![file_task(dest, "@a/x.pbo", 10)];
    let (report, events) = run(
        &request(protocol, 2),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    match &report.end {
        DownloadEnd::Failed { error } => assert_eq!(error.kind(), ErrorKind::TlsValidation),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );
    assert!(matches!(
        report.outcome(vec![]),
        Some(Outcome::Failed { .. })
    ));
    assert_eq!(tasks[0].status, DownloadStatus::Pending);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_latency() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..6 {
        let relative = format!("@a/f{i}.pbo");
        server.put(&relative, &random_bytes(10 * 1024, i));
        tasks.push(file_task(dest, &relative, 10 * 1024));
    }
    server.default_rule(Rule {
        stall_after: Some(1024),
        ..Rule::default()
    });
    // Cancelled once the three connections each received the bytes before their stall.
    let cancel = CancellationToken::new();
    let (report, events, cancelled_at) = run_cancelled_after(
        &request(server.protocol(), 3),
        &mut tasks,
        &cancel,
        3 * 1024,
    )
    .await;
    let returned = Instant::now();
    assert!(returned.duration_since(cancelled_at) < Duration::from_secs(1));
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    let running = tasks
        .iter()
        .filter(|t| t.status == DownloadStatus::Running)
        .count();
    let pending = tasks
        .iter()
        .filter(|t| t.status == DownloadStatus::Pending)
        .count();
    assert_eq!((running, pending), (3, 3));
    assert_eq!(file_failed(&events), 0);

    // A fresh token works.
    server.clear_rules();
    let fresh = CancellationToken::new();
    let (report, _) = run(
        &request(server.protocol(), 3),
        &mut tasks,
        Arc::new(NoZipSink),
        &fresh,
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(tasks.iter().all(|t| t.status == DownloadStatus::Done));

    // Cancelled before the call: no request.
    server.clear_seen();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let mut tasks = vec![file_task(dest, "@a/f9.pbo", 10)];
    let (report, _) = run(
        &request(server.protocol(), 3),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancelled,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    assert!(server.seen().is_empty());
    assert_eq!(tasks[0].status, DownloadStatus::Pending);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_during_zsync_match() {
    const LEN: usize = 64 * 1024 * 1024;
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let new = random_bytes(LEN, 1);
    let control =
        ControlFile::generate(&new[..], LEN as u64, "big.pbo", "x", "u", "").expect("generate");
    drop(new);
    server.put("@a/big.pbo.zsync", &control.to_bytes());
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/big.pbo"), random_bytes(LEN, 2)).expect("local");
    let mut tasks = vec![with_complete(
        file_task(dest, "@a/big.pbo", LEN as u64),
        10.0,
    )];

    let cancel = CancellationToken::new();
    let cancelled_at = cancel_after(&cancel, Duration::from_millis(300));
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    let returned = Instant::now();
    let cancelled_at = cancelled_at.await.expect("cancel task");
    assert!(returned.duration_since(cancelled_at) < Duration::from_secs(1));
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    assert!(part_files(dest).is_empty());
    assert_eq!(server.paths(), vec!["@a/big.pbo.zsync".to_owned()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn throttle_and_speed() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..2 {
        let relative = format!("@a/f{i}.pbo");
        server.put(&relative, &random_bytes(512 * 1024, i));
        tasks.push(file_task(dest, &relative, 512 * 1024));
    }
    let mut req = request(server.protocol(), 2);
    req.throttle = Arc::new(a3s_transport::Throttle::new(512 * 1024));
    let cancel = CancellationToken::new();
    let started = Instant::now();
    let (report, events) = run(&req, &mut tasks, Arc::new(NoZipSink), &cancel).await;
    let elapsed = started.elapsed();
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert!(elapsed >= Duration::from_millis(1500), "{elapsed:?}");
    let speeds: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::Speed { bytes_per_second } => Some(*bytes_per_second),
            _ => None,
        })
        .collect();
    assert!(speeds.len() >= 2, "{speeds:?}");
    assert!(speeds[1..].iter().all(|&s| s <= 900 * 1024), "{speeds:?}");
    assert!(
        report.average_speed > 0 && report.average_speed <= 900 * 1024,
        "{}",
        report.average_speed
    );

    // Lifting the limit mid-run speeds up a 2 MiB file.
    server.put("@a/big.pbo", &random_bytes(2 * 1024 * 1024, 9));
    let mut tasks = vec![file_task(dest, "@a/big.pbo", 2 * 1024 * 1024)];
    let throttle = Arc::clone(&req.throttle);
    let lift = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        throttle.set_limit(0);
    });
    let started = Instant::now();
    let (report, _) = run(&req, &mut tasks, Arc::new(NoZipSink), &cancel).await;
    lift.await.expect("lift");
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

/// `old` with 3 changed blocks in the middle and 20 KiB appended.
fn zsync_contents() -> (Vec<u8>, Vec<u8>) {
    let old = random_bytes(580 * 1024, 11);
    let mut new = old.clone();
    for block in [30usize, 31, 40] {
        let start = block * 8192;
        new[start..start + 8192].copy_from_slice(&random_bytes(8192, 100 + block as u64));
    }
    new.extend(random_bytes(20 * 1024, 12));
    (old, new)
}

fn control_for(content: &[u8], sha1_of: &Path) -> ControlFile {
    ControlFile::generate(
        content,
        content.len() as u64,
        "x.pbo",
        "Sat, 01 Jan 2000 00:00:00 GMT",
        "http://x/@a/x.pbo",
        &sha1(sha1_of),
    )
    .expect("generate")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn zsync_patch_and_fallbacks() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let (old, new) = zsync_contents();
    server.put("@a/x.pbo", &new);
    let control = control_for(&new, &server.path("@a/x.pbo"));
    server.put("@a/x.pbo.zsync", &control.to_bytes());
    let zip = random_bytes(4000, 13);
    server.put("@a/x.pbo.zip", &zip);
    let local = dest.join("@a/x.pbo");
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(&local, &old).expect("old");
    let task = with_complete(
        compressed_task(dest, "@a/x.pbo", new.len() as u64, zip.len() as u64, "s"),
        50.0,
    );

    let matches = a3s_zsync::match_blocks(&control, &old[..], None).expect("match");
    let plan = Plan::new(&control, &matches, Plan::MAX_BLOCKS_PER_RANGE);
    assert!(!plan.ranges.is_empty());

    let zips = Arc::new(CollectZips::new());
    let cancel = CancellationToken::new();
    let mut tasks = vec![task.clone()];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        zips.clone(),
        &cancel,
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    let seen = server.seen();
    assert_eq!(seen[0].path, "@a/x.pbo.zsync");
    assert_eq!(seen[0].range, None);
    let ranges: Vec<Option<String>> = seen[1..].iter().map(|s| s.range.clone()).collect();
    let expected: Vec<Option<String>> = plan
        .ranges
        .iter()
        .map(|r| Some(format!("bytes={}-{}", r.start, r.end - 1)))
        .collect();
    assert_eq!(ranges, expected);
    assert!(seen[1..].iter().all(|s| s.path == "@a/x.pbo"));
    assert_eq!(std::fs::read(&local).expect("patched"), new);
    assert!(part_files(dest).is_empty());
    assert_eq!(report.bytes_received, plan.download_size());
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::FileDone { .. })),
        1
    );
    assert!(zips.take().is_empty());
    assert!(report.downloaded_zips.is_empty());
    assert_eq!(tasks[0].status, DownloadStatus::Done);

    // Fallbacks: each ends with a full GET of the `.zip`.
    let fallback = |label: &'static str| {
        let server = &server;
        let task = task.clone();
        let zip = zip.clone();
        let old = old.clone();
        let local = local.clone();
        async move {
            std::fs::remove_file(local.with_extension("pbo.zip")).ok();
            if label != "missing local" {
                std::fs::write(&local, &old).expect("old");
            }
            server.clear_seen();
            let mut tasks = vec![task];
            let cancel = CancellationToken::new();
            let (report, _) = run(
                &request(server.protocol(), 1),
                &mut tasks,
                Arc::new(NoZipSink),
                &cancel,
            )
            .await;
            assert!(
                matches!(report.end, DownloadEnd::Completed),
                "{label}: {:?}",
                report.end
            );
            assert!(report.errors.is_empty(), "{label}: {:?}", report.errors);
            let last = server.seen().pop().expect("request");
            assert_eq!(last.path, "@a/x.pbo.zip", "{label}");
            assert_eq!(last.range, None, "{label}");
            assert_eq!(
                std::fs::read(local.with_extension("pbo.zip")).expect("zip"),
                zip,
                "{label}"
            );
            assert_eq!(report.downloaded_zips.len(), 1, "{label}");
            assert!(
                part_files(local.parent().expect("parent")).is_empty(),
                "{label}"
            );
        }
    };

    // (a) `.zsync` missing.
    std::fs::remove_file(server.path("@a/x.pbo.zsync")).expect("rm zsync");
    fallback("zsync 404").await;
    // (b) control file of other content with the same length: checksum mismatch.
    let other = random_bytes(new.len(), 77);
    server.put("@a/other.bin", &other);
    let wrong = control_for(&other, &server.path("@a/other.bin"));
    server.put("@a/x.pbo.zsync", &wrong.to_bytes());
    fallback("checksum mismatch").await;
    assert_eq!(
        std::fs::read(&local).expect("local"),
        old,
        "old file left untouched"
    );
    // (c) the server ignores `Range`.
    server.put("@a/x.pbo.zsync", &control.to_bytes());
    server.rule(
        "@a/x.pbo",
        Rule {
            no_range: true,
            ..Rule::default()
        },
    );
    fallback("no range").await;
    server.clear_rules();
    // (d) local file missing.
    std::fs::remove_file(&local).expect("rm local");
    fallback("missing local").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_waiting_counter() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(20 * 1024, 5);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            stall_after: Some(1024),
            stall_for: Some(Duration::from_millis(4500)),
            ..Rule::default()
        },
    );
    // Taken second (LIFO): keeps the run going for a tick after the stall ends.
    server.put("@a/slow.pbo", b"slow");
    server.rule(
        "@a/slow.pbo",
        Rule {
            delay: Some(Duration::from_millis(1500)),
            ..Rule::default()
        },
    );
    let mut tasks = vec![
        file_task(dest, "@a/slow.pbo", 4),
        file_task(dest, "@a/x.pbo", content.len() as u64),
    ];
    let cancel = CancellationToken::new();
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), content);
    let waiting: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::ConnectionWaiting { seconds } => Some(*seconds),
            _ => None,
        })
        .collect();
    let one = waiting.iter().position(|&s| s == 1).expect("1");
    assert_eq!(&waiting[one..one + 3], &[1, 2, 3], "{waiting:?}");
    assert!(waiting[..one].iter().all(|&s| s == 0), "{waiting:?}");
    assert_eq!(waiting.get(one + 3), Some(&0), "{waiting:?}");
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::DiskUsage { .. })),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_unzip_and_disk_usage() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(20 * 1024, 6);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            stall_after: Some(1024),
            stall_for: Some(Duration::from_millis(2600)),
            ..Rule::default()
        },
    );
    let mut tasks = vec![file_task(dest, "@a/x.pbo", content.len() as u64)];
    let mut req = request(server.protocol(), 1);
    req.disk_usage_path = Some(dest.to_owned());
    let cancel = CancellationToken::new();
    let (report, events) = run(&req, &mut tasks, Arc::new(BusySink::default()), &cancel).await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    let waiting: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::ConnectionWaiting { seconds } => Some(*seconds),
            _ => None,
        })
        .collect();
    assert!(waiting.len() >= 2, "{waiting:?}");
    assert!(waiting.iter().all(|&s| s == 0), "{waiting:?}");
    let disk = count(&events, |e| matches!(e, SyncEvent::DiskUsage { .. }));
    assert!((3..=7).contains(&disk), "{disk} disk usage events");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_future_is_send() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    server.put("x", b"abc");
    let mut tasks = vec![file_task(dest_dir.path(), "x", 3)];
    let req = request(server.protocol(), 1);
    let handle = tokio::spawn(async move {
        let events = EventSender::noop();
        let cancel = CancellationToken::new();
        let report = download(&req, &mut tasks, Arc::new(NoZipSink), &cancel, &events).await;
        (report.files_done, tasks)
    });
    let (done, tasks) = handle.await.expect("join");
    assert_eq!(done, 1);
    assert_eq!(tasks[0].status, DownloadStatus::Done);
    assert_eq!(std::fs::read(dest_dir.path().join("x")).expect("x"), b"abc");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d15_hostile_tasks_never_write_outside() {
    let server = HttpServer::start().await;
    let outer = tempfile::tempdir().expect("tempdir");
    let dest = outer.path().join("dest");
    std::fs::create_dir_all(&dest).expect("mkdir");
    server.put("evil", b"evil");
    server.put("@a/x.pbo", b"good");

    let mut escape = file_task(&dest, "@a/x.pbo", 4);
    escape.relative_path = "../evil".to_owned();
    escape.destination_dir = dest.clone();
    let mut nested = file_task(&dest, "@a/x.pbo", 4);
    nested.relative_path = "@a/../../evil".to_owned();
    let mut absolute = file_task(&dest, "@a/x.pbo", 4);
    absolute.relative_path = "/evil".to_owned();
    let mut backslash = file_task(&dest, "@a/x.pbo", 4);
    backslash.relative_path = "@a/..\\..\\evil".to_owned();
    let mut dir = dir_task(&dest, "@a");
    dir.relative_path = "..".to_owned();
    let mut tasks = vec![dir, escape, nested, absolute, backslash];
    let n = tasks.len();
    let cancel = CancellationToken::new();
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(report.errors.len(), n);
    assert!(
        report
            .errors
            .iter()
            .all(|e| e.error.kind() == ErrorKind::InvalidData)
    );
    assert_eq!(file_failed(&events), n);
    assert!(server.seen().is_empty(), "no request for hostile tasks");
    assert!(!outer.path().join("evil").exists());
    assert!(!outer.path().join("evil.part").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_during_zsync_assembly() {
    const LEN: usize = 64 * 1024 * 1024;
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    // The new file differs from the local one in its last block only: assembly copies 64 MiB
    // of local blocks.
    let mut new = random_bytes(LEN, 31);
    let old = new.clone();
    let last = LEN - 8192;
    new[last..].copy_from_slice(&random_bytes(8192, 32));
    server.put("@a/big.pbo", &new);
    let control = control_for(&new, &server.path("@a/big.pbo"));
    drop(new);
    server.put("@a/big.pbo.zsync", &control.to_bytes());
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    let local = dest.join("@a/big.pbo");
    std::fs::write(&local, &old).expect("local");
    let mut tasks = vec![with_complete(
        file_task(dest, "@a/big.pbo", LEN as u64),
        99.0,
    )];

    let cancel = CancellationToken::new();
    let part = dest.join("@a/big.pbo.part");
    let watcher = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            while !part.exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            cancel.cancel();
            Instant::now()
        })
    };
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    let returned = Instant::now();
    let cancelled_at = watcher.await.expect("watcher");
    assert!(returned.duration_since(cancelled_at) < Duration::from_secs(1));
    assert!(
        matches!(report.end, DownloadEnd::Cancelled),
        "{:?}",
        report.end
    );
    assert!(part_files(dest).is_empty(), "zsync .part must be deleted");
    assert_eq!(file_failed(&events), 0);
    assert_eq!(tasks[0].status, DownloadStatus::Running);
    assert!(
        std::fs::read(&local).expect("local") == old,
        "local file intact"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zsync_range_416_or_short_body_falls_back() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let (old, new) = zsync_contents();
    server.put("@a/x.pbo", &new);
    let control = control_for(&new, &server.path("@a/x.pbo"));
    server.put("@a/x.pbo.zsync", &control.to_bytes());
    let zip = random_bytes(4000, 33);
    server.put("@a/x.pbo.zip", &zip);
    let local = dest.join("@a/x.pbo");
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    let task = with_complete(
        compressed_task(dest, "@a/x.pbo", new.len() as u64, zip.len() as u64, "s"),
        50.0,
    );

    for (label, rule) in [
        (
            "416",
            Rule {
                status: Some(416),
                ..Rule::default()
            },
        ),
        (
            "short body",
            Rule {
                truncate_body: Some(100),
                ..Rule::default()
            },
        ),
    ] {
        std::fs::write(&local, &old).expect("old");
        std::fs::remove_file(dest.join("@a/x.pbo.zip")).ok();
        server.clear_seen();
        server.rule("@a/x.pbo", rule);
        let mut tasks = vec![task.clone()];
        let (report, events) = run(
            &request(server.protocol(), 1),
            &mut tasks,
            Arc::new(NoZipSink),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(report.end, DownloadEnd::Completed), "{label}");
        assert!(report.errors.is_empty(), "{label}: {:?}", report.errors);
        assert_eq!(
            count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
            0
        );
        assert_eq!(tasks[0].status, DownloadStatus::Done, "{label}");
        let paths = server.paths();
        assert_eq!(paths.first().map(String::as_str), Some("@a/x.pbo.zsync"));
        assert_eq!(
            paths.last().map(String::as_str),
            Some("@a/x.pbo.zip"),
            "{label}"
        );
        assert_eq!(std::fs::read(dest.join("@a/x.pbo.zip")).expect("zip"), zip);
        assert_eq!(
            std::fs::read(&local).expect("local"),
            old,
            "{label}: old file intact"
        );
        assert!(part_files(dest).is_empty(), "{label}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_against_a_server_ignoring_range() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(300 * 1024, 41);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            no_range: true,
            ..Rule::default()
        },
    );
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), &content[..100 * 1024]).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.errors
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].range.as_deref(), Some("bytes=102400-"));
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), content);
    assert_eq!(report.bytes_received, content.len() as u64);
    let last = events
        .iter()
        .rev()
        .find_map(|e| match e {
            SyncEvent::Transfer(p) => Some(p.transferred),
            _ => None,
        })
        .expect("transfer");
    assert_eq!(last, content.len() as u64);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumable_full_download_wins_over_zsync() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let (old, new) = zsync_contents();
    server.put("@a/x.pbo", &new);
    let control = control_for(&new, &server.path("@a/x.pbo"));
    server.put("@a/x.pbo.zsync", &control.to_bytes());
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo"), &old).expect("old");
    // A full download (after an earlier zsync fallback) left a resumable `.part`.
    std::fs::write(dest.join("@a/x.pbo.part"), &new[..50_000]).expect("part");
    let mut task = with_complete(file_task(dest, "@a/x.pbo", new.len() as u64), 50.0);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.errors
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].path, "@a/x.pbo");
    assert_eq!(seen[0].range.as_deref(), Some("bytes=50000-"));
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), new);
    assert!(part_files(dest).is_empty());
}

/// Every `transferred` value of the `Transfer` events.
fn transferred_values(events: &[SyncEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::Transfer(p) => Some(p.transferred),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restarted_resume_corrects_the_progress_offset() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(200 * 1024, 43);
    server.put("@a/x.pbo", &content);
    // The server ignores the range and pauses after more bytes than the requested offset.
    server.rule(
        "@a/x.pbo",
        Rule {
            no_range: true,
            stall_after: Some(50 * 1024),
            stall_for: Some(Duration::from_millis(600)),
            ..Rule::default()
        },
    );
    let offset = 10 * 1024;
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), &content[..offset]).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.errors
    );
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), content);
    let values = transferred_values(&events);
    // During the stall the progress is the bytes of the restarted transfer, not offset + bytes.
    assert!(values.contains(&(50 * 1024)), "{values:?}");
    assert!(!values.contains(&(offset as u64 + 50 * 1024)), "{values:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_answered_416_fails_the_file_and_drops_the_part() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(64 * 1024, 44);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            status: Some(416),
            ..Rule::default()
        },
    );
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), &content[..1000]).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].relative_path, "@a/x.pbo");
    assert_eq!(file_failed(&events), 1);
    // One request: resuming cannot help, so there is no retry.
    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].range.as_deref(), Some("bytes=1000-"));
    assert!(part_files(dest).is_empty());
    assert!(!dest.join("@a/x.pbo").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webdav_never_resumes() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(120 * 1024, 45);
    server.put("@a/x.pbo", &content);
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    // A stale partial file with wrong bytes: resuming would corrupt the result.
    std::fs::write(dest.join("@a/x.pbo.part"), vec![0xEE_u8; 30 * 1024]).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let mut protocol = server.protocol();
    protocol.protocol_type = ProtocolType::HttpWebdav;
    let (report, _) = run(
        &request(protocol, 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.errors
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].range, None);
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), content);
    assert_eq!(report.bytes_received, content.len() as u64);
}

/// A zip sink that panics, which panics the worker handing a zip over.
struct PanickingSink;

impl ZipSink for PanickingSink {
    fn submit(&self, _zip: a3s_sync::download::DownloadedZip) {
        panic!("sink failure");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_panic_fails_the_run() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(4 * 1024, 46);
    server.put("@a/x.pbo.zip", &content);
    let mut tasks = vec![compressed_task(
        dest,
        "@a/x.pbo",
        10_000,
        content.len() as u64,
        "abc",
    )];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(PanickingSink),
        &CancellationToken::new(),
    )
    .await;
    let DownloadEnd::Failed { error } = &report.end else {
        panic!("{:?}", report.end);
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    match report.outcome(vec![]) {
        Some(Outcome::Failed { error }) => assert_eq!(error.kind, ErrorKind::Internal),
        other => panic!("{other:?}"),
    }
    no_finished(&events);
}
