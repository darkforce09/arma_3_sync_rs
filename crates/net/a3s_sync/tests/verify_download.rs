//! Adversarial verification of the download executor (resume, zsync, run policy, cancel,
//! throttle, zip hand-off, path safety, sizes, names).

mod download_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_foundation::error::ErrorKind;
use a3s_sync::download::{
    CollectZips, DownloadEnd, DownloadReport, DownloadRequest, DownloadedZip, NoZipSink, ZipSink,
    download,
};
use a3s_sync::{DownloadStatus, DownloadTask, EventSender, Outcome, SyncEvent, SyncOptions};
use a3s_transport::{CancellationToken, Throttle, TransportOptions};
use a3s_types::{Protocol, ProtocolType, ServerInfo};
use a3s_zsync::{ControlFile, Plan};
use download_support::{
    FtpServer, HttpServer, Rule, compressed_task, dir_task, file_task, part_files, random_bytes,
    request, with_complete,
};
use parking_lot::Mutex;

async fn run(
    request: &DownloadRequest,
    tasks: &mut [DownloadTask],
    zips: Arc<dyn ZipSink>,
    cancel: &CancellationToken,
) -> (DownloadReport, Vec<SyncEvent>) {
    let (events, rx) = EventSender::channel();
    let report = download(request, tasks, zips, cancel, &events).await;
    (report, rx.try_iter().collect())
}

fn count<F: Fn(&SyncEvent) -> bool>(events: &[SyncEvent], f: F) -> usize {
    events.iter().filter(|e| f(e)).count()
}

fn cancel_after(token: &CancellationToken, delay: Duration) -> tokio::task::JoinHandle<Instant> {
    let token = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        token.cancel();
        Instant::now()
    })
}

fn range_of(seen: &download_support::Seen) -> Option<String> {
    seen.range.clone()
}

// ---------------------------------------------------------------------------------------------
// Resume and .part correctness

/// The server ignores `Range` on a resume: the download must restart from 0 (truncate), never
/// append a full body after the partial bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_resume_when_server_ignores_range_restarts() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(200 * 1024, 31);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            no_range: true,
            ..Rule::default()
        },
    );
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), &content[..50 * 1024]).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
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
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(range_of(&seen[0]).as_deref(), Some("bytes=51200-"));
    let got = std::fs::read(dest.join("@a/x.pbo")).expect("final");
    assert_eq!(got.len(), content.len(), "appended instead of restarting");
    assert_eq!(got, content);
    assert!(part_files(dest).is_empty());
    assert_eq!(tasks[0].status, DownloadStatus::Done);
    assert_eq!(report.bytes_received, content.len() as u64);
}

/// Range ignored and the restarted body dropped mid-way: the next attempt resumes from the new
/// (restarted) partial length and the final file is still byte-identical.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_resume_range_ignored_then_drop_then_resume() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(200 * 1024, 32);
    server.put("@a/x.pbo", &content);
    server.rule(
        "@a/x.pbo",
        Rule {
            no_range: true,
            drop_after: Some(30 * 1024),
            drop_first_only: true,
            ..Rule::default()
        },
    );
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), &content[..150 * 1024]).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
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
        report.end
    );
    let got = std::fs::read(dest.join("@a/x.pbo")).expect("final");
    assert_eq!(got.len(), content.len());
    assert_eq!(got, content);
    let seen = server.seen();
    assert_eq!(seen.len(), 2, "{seen:#?}");
    assert_eq!(range_of(&seen[0]).as_deref(), Some("bytes=153600-"));
    // The second attempt resumes from what the restarted first attempt wrote (<= 30 KiB).
    let second: u64 = range_of(&seen[1])
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|r| r.parse().ok())
        .expect("open range");
    assert!(second > 0 && second <= 30 * 1024, "{second}");
}

/// A stale `.part` that is as large as or larger than the expected size is discarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_stale_part_not_smaller_than_expected_is_discarded() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(2000, 33);
    server.put("@a/x.pbo", &content);
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), random_bytes(3000, 34)).expect("part");
    let mut task = file_task(dest, "@a/x.pbo", content.len() as u64);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(server.seen()[0].range, None);
    assert_eq!(
        std::fs::read(dest.join("@a/x.pbo")).expect("final"),
        content
    );
    assert!(part_files(dest).is_empty());
}

/// `Done` tasks are never touched, even with a leftover `.part`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_done_task_with_leftover_part_untouched() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo.part"), b"leftover").expect("part");
    let mut task = file_task(dest, "@a/x.pbo", 100);
    task.status = DownloadStatus::Done;
    let mut tasks = vec![task];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert!(server.seen().is_empty());
    assert_eq!(
        std::fs::read(dest.join("@a/x.pbo.part")).expect("part"),
        b"leftover"
    );
    assert_eq!(report.files_done, 0);
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::FileStarted { .. })),
        0
    );
    assert_eq!(report.outcome(vec![]), Some(Outcome::Success));
}

/// Empty remote files leave 0-byte final files (HTTP), also with a stale non-empty `.part`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_empty_files_http() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/empty.pbo", b"");
    server.put("@a/empty2.pbo", b"");
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/empty2.pbo.part"), b"stale").expect("part");
    std::fs::write(dest.join("@a/empty.pbo"), b"old content").expect("old");
    let mut stale = file_task(dest, "@a/empty2.pbo", 0);
    stale.status = DownloadStatus::Running;
    let mut tasks = vec![file_task(dest, "@a/empty.pbo", 0), stale];
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
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for name in ["@a/empty.pbo", "@a/empty2.pbo"] {
        let meta = std::fs::metadata(dest.join(name)).expect(name);
        assert!(meta.is_file());
        assert_eq!(meta.len(), 0, "{name}");
    }
    assert!(part_files(dest).is_empty());
    assert!(tasks.iter().all(|t| t.status == DownloadStatus::Done));
}

/// Empty remote files over FTP.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_empty_file_ftp() {
    let server = FtpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/empty.pbo", b"");
    let mut tasks = vec![file_task(dest, "@a/empty.pbo", 0)];
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
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        std::fs::metadata(dest.join("@a/empty.pbo"))
            .expect("file")
            .len(),
        0
    );
    assert_eq!(tasks[0].status, DownloadStatus::Done);
}

/// Resume offsets beyond 4 GiB are sent verbatim (no 32-bit truncation) and a failed attempt
/// keeps the sparse `.part` intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_resume_offset_beyond_4_gib() {
    const OFFSET: u64 = 5 * 1024 * 1024 * 1024;
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    let part = dest.join("@a/huge.pbo.part");
    std::fs::File::create(&part)
        .expect("part")
        .set_len(OFFSET)
        .expect("sparse");
    server.rule(
        "@a/huge.pbo",
        Rule {
            status: Some(404),
            ..Rule::default()
        },
    );
    let mut task = file_task(dest, "@a/huge.pbo", OFFSET + 100);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].range.as_deref(),
        Some(format!("bytes={OFFSET}-").as_str())
    );
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].error.kind(), ErrorKind::NotFound);
    assert_eq!(std::fs::metadata(&part).expect("part").len(), OFFSET);
    assert_eq!(tasks[0].status, DownloadStatus::Running);
}

/// Sizes taken from a hostile `.a3s/sync` (`i64::MAX` each) must not panic the executor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_huge_sizes_do_not_panic() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path().to_owned();
    server.put("@a/x.pbo", b"abc");
    let mut tasks: Vec<DownloadTask> = (0..3)
        .map(|i| {
            let mut t = file_task(&dest, &format!("@a/big{i}.pbo"), i64::MAX as u64);
            t.status = DownloadStatus::Done;
            t
        })
        .collect();
    tasks.push(file_task(&dest, "@a/x.pbo", 3));
    let req = request(server.protocol(), 1);
    let handle = tokio::spawn(async move {
        let report = download(
            &req,
            &mut tasks,
            Arc::new(NoZipSink),
            &CancellationToken::new(),
            &EventSender::noop(),
        )
        .await;
        (matches!(report.end, DownloadEnd::Completed), tasks)
    });
    let joined = handle.await;
    assert!(
        joined.is_ok(),
        "download panicked on hostile sizes: {:?}",
        joined.err()
    );
}

// ---------------------------------------------------------------------------------------------
// zsync

fn zsync_contents(seed: u64) -> (Vec<u8>, Vec<u8>) {
    let old = random_bytes(200 * 1024, seed);
    let mut new = old.clone();
    for block in [5usize, 6, 12] {
        let start = block * 8192;
        new[start..start + 8192].copy_from_slice(&random_bytes(8192, seed + 100 + block as u64));
    }
    new.extend(random_bytes(20 * 1024, seed + 1));
    (old, new)
}

fn control_for(content: &[u8]) -> ControlFile {
    let sha1 = {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("f");
        std::fs::write(&p, content).expect("write");
        a3s_repo_files::hash::sha1_file(&p).expect("sha1")
    };
    ControlFile::generate(
        content,
        content.len() as u64,
        "x.pbo",
        "Sat, 01 Jan 2000 00:00:00 GMT",
        "http://x/@a/x.pbo",
        &sha1,
    )
    .expect("generate")
}

struct Zs {
    server: HttpServer,
    _dest_dir: tempfile::TempDir,
    dest: PathBuf,
    old: Vec<u8>,
    new: Vec<u8>,
}

async fn zsync_setup(seed: u64) -> Zs {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path().to_owned();
    let (old, new) = zsync_contents(seed);
    server.put("@a/x.pbo", &new);
    let control = control_for(&new);
    server.put("@a/x.pbo.zsync", &control.to_bytes());
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo"), &old).expect("old");
    Zs {
        server,
        _dest_dir: dest_dir,
        dest,
        old,
        new,
    }
}

fn zsync_task(zs: &Zs) -> DownloadTask {
    with_complete(file_task(&zs.dest, "@a/x.pbo", zs.new.len() as u64), 50.0)
}

/// An unparsable `.zsync` falls back to a full download.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zsync_garbage_control_falls_back() {
    let zs = zsync_setup(40).await;
    zs.server.put(
        "@a/x.pbo.zsync",
        b"this is not a zsync control file\n\n\x00\x01",
    );
    let mut tasks = vec![zsync_task(&zs)];
    let (report, _) = run(
        &request(zs.server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let last = zs.server.seen().pop().expect("request");
    assert_eq!(
        (last.path.as_str(), last.range.as_deref()),
        ("@a/x.pbo", None)
    );
    assert_eq!(
        std::fs::read(zs.dest.join("@a/x.pbo")).expect("file"),
        zs.new
    );
    assert!(part_files(&zs.dest).is_empty());
}

/// A `.zsync` whose length differs from the task size falls back to a full download.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zsync_length_mismatch_falls_back() {
    let zs = zsync_setup(41).await;
    let shorter = control_for(&zs.new[..zs.new.len() - 1]);
    zs.server.put("@a/x.pbo.zsync", &shorter.to_bytes());
    let mut tasks = vec![zsync_task(&zs)];
    let (report, _) = run(
        &request(zs.server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let seen = zs.server.seen();
    assert!(
        seen.iter().all(|s| s.range.is_none()),
        "no range request: {seen:#?}"
    );
    assert_eq!(
        std::fs::read(zs.dest.join("@a/x.pbo")).expect("file"),
        zs.new
    );
}

/// Server file shorter than the control file says (a range past EOF answers 416): the task must
/// not leave a `.part`, must not corrupt the old local file, and must not be "connection lost".
/// The retry policy does not list 416, so a file error is the specified outcome.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zsync_range_416_is_a_clean_file_error() {
    let zs = zsync_setup(42).await;
    // Truncate the server file before the appended tail: the tail range starts past EOF.
    zs.server.put("@a/x.pbo", &zs.new[..190 * 1024]);
    let mut tasks = vec![zsync_task(&zs)];
    let (report, events) = run(
        &request(zs.server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );
    assert!(part_files(&zs.dest).is_empty());
    let local = std::fs::read(zs.dest.join("@a/x.pbo")).expect("local");
    assert!(local == zs.old, "old local file must stay intact");
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert_eq!(tasks[0].status, DownloadStatus::Running);
}

/// A connection drop in the middle of a range body: no resume for zsync (connection
/// lost), `.part` deleted, old local file intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zsync_drop_mid_range_cleans_part() {
    let zs = zsync_setup(43).await;
    zs.server.rule(
        "@a/x.pbo",
        Rule {
            drop_after: Some(1000),
            ..Rule::default()
        },
    );
    let mut tasks = vec![zsync_task(&zs)];
    let (report, events) = run(
        &request(zs.server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
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
    assert!(part_files(&zs.dest).is_empty());
    assert_eq!(
        std::fs::read(zs.dest.join("@a/x.pbo")).expect("local"),
        zs.old
    );
    assert_eq!(tasks[0].status, DownloadStatus::Running);
    assert!(report.errors.is_empty());
}

/// Cancel while a range body is stalled: returns within 1 s, no `.part`, old file intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zsync_cancel_during_range_stall() {
    let zs = zsync_setup(44).await;
    zs.server.rule(
        "@a/x.pbo",
        Rule {
            stall_after: Some(1000),
            ..Rule::default()
        },
    );
    let mut tasks = vec![zsync_task(&zs)];
    let cancel = CancellationToken::new();
    let at = cancel_after(&cancel, Duration::from_millis(300));
    let (report, events) = run(
        &request(zs.server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    let returned = Instant::now();
    let at = at.await.expect("cancel");
    assert!(returned.duration_since(at) < Duration::from_secs(1));
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    assert!(part_files(&zs.dest).is_empty());
    assert_eq!(
        std::fs::read(zs.dest.join("@a/x.pbo")).expect("local"),
        zs.old
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::FileFailed { .. })),
        0
    );
    assert_eq!(tasks[0].status, DownloadStatus::Running);
}

/// The plan's ranges are at most 100 blocks each and sent in ascending order (large change).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zsync_ranges_capped_at_100_blocks() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let old = random_bytes(300 * 8192, 50);
    let mut new = old.clone();
    // 250 consecutive changed blocks.
    new[10 * 8192..260 * 8192].copy_from_slice(&random_bytes(250 * 8192, 51));
    server.put("@a/x.pbo", &new);
    let control = control_for(&new);
    server.put("@a/x.pbo.zsync", &control.to_bytes());
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo"), &old).expect("old");
    let mut tasks = vec![with_complete(
        file_task(dest, "@a/x.pbo", new.len() as u64),
        20.0,
    )];
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
        report.end
    );
    assert_eq!(std::fs::read(dest.join("@a/x.pbo")).expect("file"), new);
    let matches = a3s_zsync::match_blocks(&control, &old[..], None).expect("match");
    let plan = Plan::new(&control, &matches, Plan::MAX_BLOCKS_PER_RANGE);
    let ranges: Vec<(u64, u64)> = server
        .seen()
        .iter()
        .filter_map(|s| s.range.as_deref().map(str::to_owned))
        .map(|r| {
            let (a, b) = r
                .strip_prefix("bytes=")
                .and_then(|r| r.split_once('-'))
                .expect("range");
            (a.parse().expect("a"), b.parse().expect("b"))
        })
        .collect();
    assert_eq!(ranges.len(), plan.ranges.len());
    assert!(ranges.len() >= 3, "{ranges:?}");
    for &(a, b) in &ranges {
        assert!(b + 1 - a <= 100 * 8192, "range {a}-{b} exceeds 100 blocks");
    }
    assert!(ranges.windows(2).all(|w| w[0].1 < w[1].0), "{ranges:?}");
    assert_eq!(report.bytes_received, plan.download_size());
}

// ---------------------------------------------------------------------------------------------
// Workers, LIFO, error limit, connection lost

/// `connections = 0` is clamped to one worker, which takes tasks last-first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zero_connections_clamped_to_one() {
    let server = HttpServer::start().await;
    server.ok_delay(Duration::from_millis(30));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..4 {
        let rel = format!("@a/f{i}.pbo");
        server.put(&rel, &random_bytes(100, i));
        tasks.push(file_task(dest, &rel, 100));
    }
    let (report, _) = run(
        &request(server.protocol(), 0),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(server.max_in_flight(), 1);
    let expected: Vec<String> = (0..4).rev().map(|i| format!("@a/f{i}.pbo")).collect();
    assert_eq!(server.paths(), expected);
}

/// K = min(client, server connections): client 8, server 2 → never more than 2 transfers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_k_bounded_by_server_connections() {
    let server = HttpServer::start().await;
    server.ok_delay(Duration::from_millis(150));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..6 {
        let rel = format!("@a/f{i}.pbo");
        server.put(&rel, &random_bytes(100, i));
        tasks.push(file_task(dest, &rel, 100));
    }
    let options = SyncOptions {
        repository_name: "t".to_owned(),
        event_name: None,
        destination: dest.to_owned(),
        auto_discover: false,
        exact_match: false,
        hidden_folder_paths: std::collections::BTreeSet::new(),
        client_connections: 8,
        max_download_speed: 0,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
    };
    let info = ServerInfo {
        number_of_connections: 2,
        ..Default::default()
    };
    let protocol = server.protocol();
    let topts = TransportOptions::from_protocol(&protocol);
    let mut req = DownloadRequest::new(protocol, topts, &options, Some(&info));
    req.disk_usage_path = None;
    assert_eq!(req.connections, 2);
    let (report, _) = run(
        &req,
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(server.max_in_flight(), 2);
    assert_eq!(report.max_active_connections, 2);
    let first_two: std::collections::BTreeSet<String> =
        server.paths().into_iter().take(2).collect();
    let expected: std::collections::BTreeSet<String> =
        ["@a/f5.pbo".to_owned(), "@a/f4.pbo".to_owned()].into();
    assert_eq!(first_two, expected, "LIFO: the last tasks go first");
}

/// `max_errors = 0` is clamped to 1; the outcome carries the Java "at least" headline.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_error_limit_clamp_and_at_least_message() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let tasks: Vec<DownloadTask> = (0..12)
        .map(|i| file_task(dest, &format!("@m/f{i:02}.pbo"), 10))
        .collect();

    let mut req = request(server.protocol(), 1);
    req.max_errors = 0;
    let mut t = tasks.clone();
    let (report, _) = run(&req, &mut t, Arc::new(NoZipSink), &CancellationToken::new()).await;
    assert!(
        matches!(report.end, DownloadEnd::TooManyErrors { limit: 1 }),
        "{:?}",
        report.end
    );
    assert_eq!(report.errors.len(), 1);
    assert_eq!(server.seen().len(), 1);

    server.clear_seen();
    let mut t = tasks.clone();
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut t,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    let outcome = report.outcome(vec![]).expect("outcome");
    assert_eq!(
        outcome.download_report_message().as_deref(),
        Some("Download has been canceled due to too many errors (at least 10)")
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::FileFailed { .. })),
        10
    );
    assert_eq!(count(&events, |e| matches!(e, SyncEvent::Finished(_))), 0);
    // LIFO: the 10 failures are the last 10 tasks.
    assert!(
        t[..2].iter().all(|t| t.status == DownloadStatus::Pending),
        "{t:#?}"
    );
}

/// Three workers losing the connection at once: `ConnectionLost` exactly once, terminal (no
/// `Finished`, no `FileFailed`), partial files kept for resume.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_connection_lost_once_with_three_workers() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..6 {
        let rel = format!("@a/f{i}.pbo");
        server.put(&rel, &random_bytes(64 * 1024, i));
        tasks.push(file_task(dest, &rel, 64 * 1024));
    }
    server.default_rule(Rule {
        drop_after: Some(2048),
        ..Rule::default()
    });
    let (report, events) = run(
        &request(server.protocol(), 3),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
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
    assert_eq!(count(&events, |e| matches!(e, SyncEvent::Finished(_))), 0);
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::FileFailed { .. })),
        0
    );
    assert!(report.errors.is_empty());
    assert_eq!(report.outcome(vec![]), None);
    let running = tasks
        .iter()
        .filter(|t| t.status == DownloadStatus::Running)
        .count();
    assert!(running >= 1);
    assert!(
        !part_files(dest).is_empty(),
        "partial files kept for resume"
    );
    assert!(tasks.iter().all(|t| t.status != DownloadStatus::Done));
    // ConnectionLost is the last file-level event: nothing after it except the closing
    // Transfer / ActiveConnections bookkeeping.
    let lost = events
        .iter()
        .position(|e| matches!(e, SyncEvent::ConnectionLost))
        .expect("lost");
    assert!(
        events[lost..].iter().all(|e| !matches!(
            e,
            SyncEvent::FileStarted { .. } | SyncEvent::FileDone { .. }
        )),
        "{:?}",
        &events[lost..]
    );
}

/// Errors recorded before the connection is lost stay in the report.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_errors_before_connection_lost_are_kept() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/drop.pbo", &random_bytes(10 * 1024, 1));
    server.rule(
        "@a/drop.pbo",
        Rule {
            drop_after: Some(1024),
            ..Rule::default()
        },
    );
    // LIFO: the missing file goes first, then the dropping one.
    let mut tasks = vec![
        file_task(dest, "@a/drop.pbo", 10 * 1024),
        file_task(dest, "@a/missing.pbo", 10),
    ];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::ConnectionLost { .. }),
        "{:?}",
        report.end
    );
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].relative_path, "@a/missing.pbo");
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        1
    );
    assert_eq!(report.outcome(vec![]), None);
}

// ---------------------------------------------------------------------------------------------
// Cancellation

/// Cancel while the throttle is sleeping on a long body: still returns within 1 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_cancel_while_throttled() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/x.pbo", &random_bytes(1024 * 1024, 60));
    let mut tasks = vec![file_task(dest, "@a/x.pbo", 1024 * 1024)];
    let mut req = request(server.protocol(), 1);
    req.throttle = Arc::new(Throttle::new(1024));
    let cancel = CancellationToken::new();
    let at = cancel_after(&cancel, Duration::from_millis(400));
    let (report, _) = run(&req, &mut tasks, Arc::new(NoZipSink), &cancel).await;
    let returned = Instant::now();
    let at = at.await.expect("cancel");
    assert!(
        returned.duration_since(at) < Duration::from_secs(1),
        "{:?}",
        returned.duration_since(at)
    );
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    assert_eq!(tasks[0].status, DownloadStatus::Running);
}

/// Cancel while an FTP connect waits for the server greeting: returns within 1 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_cancel_during_stalled_connect() {
    // Accepts TCP (backlog) but never sends the FTP greeting.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let protocol = Protocol::new(
        ProtocolType::Ftp,
        "127.0.0.1/repo",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let mut tasks = vec![file_task(dest_dir.path(), "@a/x.pbo", 10)];
    let cancel = CancellationToken::new();
    let at = cancel_after(&cancel, Duration::from_millis(300));
    let (report, events) = run(
        &request(protocol, 2),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
    )
    .await;
    let returned = Instant::now();
    let at = at.await.expect("cancel");
    drop(listener);
    assert!(
        returned.duration_since(at) < Duration::from_secs(1),
        "{:?}",
        returned.duration_since(at)
    );
    assert!(
        matches!(report.end, DownloadEnd::Cancelled),
        "{:?}",
        report.end
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );
    assert_eq!(tasks[0].status, DownloadStatus::Pending);
}

/// After `download` returns (cancelled mid-transfer), no further events are emitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_no_events_after_return() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/x.pbo", &random_bytes(64 * 1024, 61));
    server.rule(
        "@a/x.pbo",
        Rule {
            stall_after: Some(1024),
            ..Rule::default()
        },
    );
    let mut tasks = vec![file_task(dest, "@a/x.pbo", 64 * 1024)];
    let (events, rx) = EventSender::channel();
    let cancel = CancellationToken::new();
    let _at = cancel_after(&cancel, Duration::from_millis(1300));
    let report = download(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
        &events,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Cancelled));
    let before: Vec<SyncEvent> = rx.try_iter().collect();
    assert!(
        matches!(
            before.last(),
            Some(SyncEvent::ActiveConnections { active: 0 })
        ),
        "{:?}",
        before.last()
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let after: Vec<SyncEvent> = rx.try_iter().collect();
    assert!(after.is_empty(), "events after return: {after:?}");
}

// ---------------------------------------------------------------------------------------------
// Throttle accuracy

fn within(actual: Duration, expected: Duration, tolerance: f64) -> bool {
    let a = actual.as_secs_f64();
    let e = expected.as_secs_f64();
    a >= e * (1.0 - tolerance) && a <= e * (1.0 + tolerance)
}

/// 1 MiB at 256 KiB/s with one connection takes 4 s ± 20 %.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_throttle_accuracy_single_connection() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/x.pbo", &random_bytes(1024 * 1024, 70));
    let mut tasks = vec![file_task(dest, "@a/x.pbo", 1024 * 1024)];
    let mut req = request(server.protocol(), 1);
    req.throttle = Arc::new(Throttle::new(256 * 1024));
    let started = Instant::now();
    let (report, events) = run(
        &req,
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert!(
        within(elapsed, Duration::from_secs(4), 0.2),
        "elapsed {elapsed:?} not within 4 s +-20 %"
    );
    let speeds: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::Speed { bytes_per_second } => Some(*bytes_per_second),
            _ => None,
        })
        .collect();
    assert!(speeds.len() >= 6, "{speeds:?}");
    // Individual 500 ms samples are bursty (large chunks then a long sleep), and Java's running
    // mean (`report.average_speed`) is dominated by the last samples, so only the arithmetic
    // mean of the samples is checked (+-25 %).
    let mean = speeds.iter().sum::<u64>() as f64 / speeds.len() as f64;
    assert!(
        (256.0 * 1024.0 * 0.75..=256.0 * 1024.0 * 1.25).contains(&mean),
        "mean sample {mean} vs 262144; samples {speeds:?}"
    );
    assert!(report.average_speed > 0);
}

/// The limit is global: two connections share 256 KiB/s (2 × 512 KiB → 4 s ± 20 %).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_throttle_accuracy_is_global_across_workers() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks = Vec::new();
    for i in 0..2 {
        let rel = format!("@a/f{i}.pbo");
        server.put(&rel, &random_bytes(512 * 1024, 80 + i));
        tasks.push(file_task(dest, &rel, 512 * 1024));
    }
    let mut req = request(server.protocol(), 2);
    req.throttle = Arc::new(Throttle::new(256 * 1024));
    let started = Instant::now();
    let (report, _) = run(
        &req,
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(report.max_active_connections, 2);
    assert!(
        within(elapsed, Duration::from_secs(4), 0.2),
        "elapsed {elapsed:?} not within 4 s +-20 %"
    );
}

// ---------------------------------------------------------------------------------------------
// Zip hand-off

/// Records, at submit time, what is on disk.
#[derive(Default)]
struct DiskCheckSink {
    seen: Mutex<Vec<(DownloadedZip, Option<u64>, bool)>>,
}

impl ZipSink for DiskCheckSink {
    fn submit(&self, zip: DownloadedZip) {
        let len = std::fs::metadata(&zip.zip_path).ok().map(|m| m.len());
        let mut part = zip.zip_path.clone().into_os_string();
        part.push(".part");
        let part_exists = Path::new(&part).exists();
        self.seen.lock().push((zip, len, part_exists));
    }
}

/// `.pbo.zip` and `.ebo.zip` are handed over complete (renamed, full length) at submit time,
/// once each, and match `report.downloaded_zips`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_zips_complete_on_disk_when_submitted() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let a = random_bytes(70_000, 90);
    let b = random_bytes(30_000, 91);
    server.put("@a/addons/a.pbo.zip", &a);
    server.put("@a/addons/b.ebo.zip", &b);
    server.put("@a/addons/c.paa", b"plain");
    let mut tasks = vec![
        compressed_task(dest, "@a/addons/a.pbo", 100_000, a.len() as u64, "sa"),
        compressed_task(dest, "@a/addons/b.ebo", 50_000, b.len() as u64, "sb"),
        file_task(dest, "@a/addons/c.paa", 5),
    ];
    let sink = Arc::new(DiskCheckSink::default());
    let (report, _) = run(
        &request(server.protocol(), 2),
        &mut tasks,
        sink.clone(),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    let seen = sink.seen.lock().clone();
    assert_eq!(seen.len(), 2, "{seen:#?}");
    for (zip, len, part_exists) in &seen {
        let expected = if zip.relative_path.ends_with("a.pbo") {
            a.len() as u64
        } else {
            b.len() as u64
        };
        assert_eq!(*len, Some(expected), "{}", zip.relative_path);
        assert!(!part_exists, "{}", zip.relative_path);
    }
    let mut submitted: Vec<DownloadedZip> = seen.into_iter().map(|s| s.0).collect();
    let mut reported = report.downloaded_zips.clone();
    submitted.sort_by(|x, y| x.relative_path.cmp(&y.relative_path));
    reported.sort_by(|x, y| x.relative_path.cmp(&y.relative_path));
    assert_eq!(submitted, reported);
    assert_eq!(submitted[1].zip_path, dest.join("@a/addons/b.ebo.zip"));
    assert_eq!(submitted[1].target_path, dest.join("@a/addons/b.ebo"));
    assert_eq!(submitted[1].size, 50_000);
}

// ---------------------------------------------------------------------------------------------
// Path safety

/// Hostile names (Windows separators, empty, `.`, drive prefixes, NUL, `..` directories) are
/// refused without a request and nothing is created outside the destination.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_hostile_names_are_refused() {
    let server = HttpServer::start().await;
    let outer = tempfile::tempdir().expect("tempdir");
    let dest = outer.path().join("dest");
    std::fs::create_dir_all(&dest).expect("mkdir");
    let mk = |relative: &str, destination: PathBuf| {
        let mut t = file_task(&dest, "@a/x.pbo", 4);
        t.relative_path = relative.to_owned();
        t.destination_dir = destination;
        t
    };
    let mut tasks = vec![
        mk("..\\evil", dest.clone()),
        mk("@a\\..\\..\\evil", dest.clone()),
        mk("", dest.clone()),
        mk("@a/./x.pbo", dest.join("@a/.")),
        mk("C:evil", dest.clone()),
        mk("@a/x\0y", dest.join("@a")),
        mk("@a//x.pbo", dest.join("@a/")),
    ];
    let mut d = dir_task(&dest, "@a");
    d.relative_path = "@a/../../evil_dir".to_owned();
    d.destination_dir = dest.join("@a/../..");
    tasks.push(d);
    let n = tasks.len();
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(report.errors.len(), n, "{:#?}", report.errors);
    assert!(server.seen().is_empty(), "{:?}", server.paths());
    let outside: Vec<_> = std::fs::read_dir(outer.path())
        .expect("outer")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(outside, vec![std::ffi::OsString::from("dest")]);
}

/// Symbolic links at the `.part` path or the target are replaced, never written through.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_symlinked_part_and_target_not_followed() {
    let server = HttpServer::start().await;
    let outer = tempfile::tempdir().expect("tempdir");
    let dest = outer.path().join("dest");
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    let victim1 = outer.path().join("victim1");
    let victim2 = outer.path().join("victim2");
    let victim3 = outer.path().join("victim3");
    for v in [&victim1, &victim2, &victim3] {
        std::fs::write(v, b"precious").expect("victim");
    }
    std::os::unix::fs::symlink(&victim1, dest.join("@a/x.pbo.part")).expect("link");
    std::os::unix::fs::symlink(&victim2, dest.join("@a/x.pbo")).expect("link");
    std::os::unix::fs::symlink(&victim3, dest.join("@a/y.pbo.zip.part")).expect("link");
    server.put("@a/x.pbo", b"new content");
    server.put("@a/y.pbo.zip", b"zipzip");
    let mut running = file_task(&dest, "@a/x.pbo", 11);
    running.status = DownloadStatus::Running;
    let mut z = compressed_task(&dest, "@a/y.pbo", 10, 6, "s");
    z.status = DownloadStatus::Running;
    let mut tasks = vec![running, z];
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(CollectZips::new()),
        &CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for v in [&victim1, &victim2, &victim3] {
        assert_eq!(
            std::fs::read(v).expect("victim"),
            b"precious",
            "{}",
            v.display()
        );
    }
    let meta = std::fs::symlink_metadata(dest.join("@a/x.pbo")).expect("x");
    assert!(meta.file_type().is_file());
    assert_eq!(
        std::fs::read(dest.join("@a/x.pbo")).expect("x"),
        b"new content"
    );
    assert_eq!(
        std::fs::read(dest.join("@a/y.pbo.zip")).expect("y"),
        b"zipzip"
    );
}

/// A user-created symlinked `@mod` folder inside the destination is followed for
/// downloads (only remote names are validated). Pending change at the time of writing.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_d16_user_symlinked_mod_folder_is_followed() {
    let server = HttpServer::start().await;
    let outer = tempfile::tempdir().expect("tempdir");
    let dest = outer.path().join("dest");
    let other_disk = outer.path().join("other_disk/@mod");
    std::fs::create_dir_all(&dest).expect("mkdir");
    std::fs::create_dir_all(other_disk.join("addons")).expect("mkdir");
    std::os::unix::fs::symlink(&other_disk, dest.join("@mod")).expect("link");
    server.put("@mod/addons/x.pbo", b"modfile");
    server.put("@mod/addons/new/y.pbo", b"newdir");
    let mut tasks = vec![
        dir_task(&dest, "@mod/addons/new"),
        file_task(&dest, "@mod/addons/x.pbo", 7),
        file_task(&dest, "@mod/addons/new/y.pbo", 6),
    ];
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert!(tasks.iter().all(|t| t.status == DownloadStatus::Done));
    assert_eq!(
        std::fs::read(other_disk.join("addons/x.pbo")).expect("x"),
        b"modfile"
    );
    assert_eq!(
        std::fs::read(other_disk.join("addons/new/y.pbo")).expect("y"),
        b"newdir"
    );
}

// ---------------------------------------------------------------------------------------------
// Local I/O failures

#[cfg(unix)]
fn running_as_root() -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let probe = dir.path().join("ro");
    std::fs::create_dir(&probe).expect("mkdir");
    set_mode(&probe, 0o555);
    let writable = std::fs::write(probe.join("f"), b"x").is_ok();
    set_mode(&probe, 0o755);
    writable
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// A read-only destination is a file error (permission denied), never "connection lost", and
/// the run continues with the next file.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_permission_denied_is_a_file_error() {
    if running_as_root() {
        eprintln!("skipped: running as root");
        return;
    }
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@ro/x.pbo", b"abc");
    server.put("@ok/y.pbo", b"def");
    std::fs::create_dir_all(dest.join("@ro")).expect("mkdir");
    set_mode(&dest.join("@ro"), 0o555);
    let mut tasks = vec![
        file_task(dest, "@ok/y.pbo", 3),
        file_task(dest, "@ro/x.pbo", 3),
    ];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    set_mode(&dest.join("@ro"), 0o755);
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert_eq!(report.errors[0].relative_path, "@ro/x.pbo");
    assert_eq!(report.errors[0].error.kind(), ErrorKind::PermissionDenied);
    assert_eq!(tasks[0].status, DownloadStatus::Done);
}

/// The final rename onto an existing non-empty directory fails as a file error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_rename_failure_is_a_file_error() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    server.put("@a/x.pbo", b"abc");
    std::fs::create_dir_all(dest.join("@a/x.pbo/inner")).expect("mkdir");
    let mut tasks = vec![file_task(dest, "@a/x.pbo", 3)];
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
        report.end
    );
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::ConnectionLost)),
        0
    );
    assert!(dest.join("@a/x.pbo/inner").is_dir());
    assert_ne!(tasks[0].status, DownloadStatus::Done);
}

/// A directory task blocked by a regular file: "Cannot create directory" file error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_directory_task_blocked_by_file() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/userconfig"), b"file").expect("file");
    let mut tasks = vec![dir_task(dest, "@a/userconfig")];
    let (report, events) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(report.errors.len(), 1);
    let message = report.errors[0].error.to_string();
    let abs = std::path::absolute(dest.join("@a/userconfig")).expect("abs");
    assert_eq!(
        message,
        format!(
            "Cannot create directory {}\nPlease checkout file access permissions.",
            abs.display()
        )
    );
    assert_eq!(
        count(&events, |e| matches!(e, SyncEvent::FileFailed { .. })),
        1
    );
    assert!(server.seen().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Names

/// Unicode, spaces and URL-special characters survive the request path and the local name;
/// names differing only in case are distinct files.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_unicode_and_special_names_http() {
    let server = HttpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let names = [
        "@ünï/ädd ons/文件 #1%20?.pbo",
        "@ünï/ädd ons/X.pbo",
        "@ünï/ädd ons/x.pbo",
    ];
    let mut tasks = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let content = random_bytes(1000 + i, 100 + i as u64);
        server.put(name, &content);
        tasks.push(file_task(dest, name, content.len() as u64));
    }
    let (report, _) = run(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
    )
    .await;
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    let mut paths = server.paths();
    paths.sort();
    let mut expected: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
    expected.sort();
    assert_eq!(paths, expected);
    for name in names {
        assert_eq!(
            std::fs::read(dest.join(name)).expect(name),
            std::fs::read(server.path(name)).expect(name),
            "{name}"
        );
    }
}

/// Unicode names over FTP, and a missing `.zip` reported as not found naming the `.zip`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v_ftp_unicode_and_missing_zip() {
    let server = FtpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let content = random_bytes(5000, 120);
    server.put("@ünï/文件 a.pbo", &content);
    let mut tasks = vec![
        compressed_task(dest, "@z/x.pbo", 10, 5, "s"),
        file_task(dest, "@ünï/文件 a.pbo", content.len() as u64),
    ];
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
        report.end
    );
    assert_eq!(
        std::fs::read(dest.join("@ünï/文件 a.pbo")).expect("unicode"),
        content
    );
    assert_eq!(report.errors.len(), 1, "{:#?}", report.errors);
    let error = &report.errors[0].error;
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error:?}");
    assert!(error.to_string().ends_with("/@z/x.pbo.zip"), "{error}");
}
