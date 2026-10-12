//! Uncompressing downloaded archives: Java-built `.pbo.zip`/`.ebo.zip`, hostile archives and
//! the background pool.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_sync::download::{DownloadedZip, ZipSink as _};
use a3s_sync::unzip::{ArchiveProblem, UnzipError, UnzipPool, UnzipStats, unzip_file};
use a3s_sync::{EventSender, Phase, SyncEvent};
use zip::System;
use zip::write::SimpleFileOptions;

fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_built/http_zip/1_initial/repo")
}

fn sha1_of(bytes: &[u8]) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("f");
    std::fs::write(&path, bytes).expect("write");
    a3s_repo_files::hash::sha1_file(&path).expect("sha1")
}

/// An archive with the given entries (`None` content = a directory entry).
fn archive(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in entries {
        match content {
            Some(content) => {
                writer.start_file(*name, options).expect("start");
                writer.write_all(content).expect("write");
            }
            None => writer.add_directory(*name, options).expect("dir"),
        }
    }
    writer.finish().expect("finish").into_inner()
}

/// A downloaded archive `dir/x.pbo.zip` holding `bytes`, for a leaf of `content`.
fn downloaded(dir: &Path, bytes: &[u8], content: &[u8]) -> DownloadedZip {
    let zip_path = dir.join("x.pbo.zip");
    std::fs::write(&zip_path, bytes).expect("write zip");
    DownloadedZip {
        relative_path: "@a/x.pbo".to_owned(),
        zip_path,
        target_path: dir.join("x.pbo"),
        sha1: Some(sha1_of(content)),
        size: content.len() as u64,
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn java_built_archives_extract_to_their_leaf() {
    let repo = fixture_repo();
    for leaf in [
        "@Mod_A/addons/mod_a.pbo",
        "@mod_b/Addons/mod_b.ebo",
        "Collection/@Nested/addons/nested.pbo",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = std::fs::read(repo.join(leaf)).expect("original");
        let name = Path::new(leaf).file_name().expect("name");
        let target = dir.path().join(name);
        let zip_path = dir.path().join(format!("{}.zip", name.to_string_lossy()));
        std::fs::copy(repo.join(format!("{leaf}.zip")), &zip_path).expect("copy");
        // An outdated target is replaced.
        std::fs::write(&target, b"outdated").expect("old");
        let zip = DownloadedZip {
            relative_path: leaf.to_owned(),
            zip_path: zip_path.clone(),
            target_path: target.clone(),
            sha1: Some(sha1_of(&original)),
            size: original.len() as u64,
        };
        let stats = unzip_file(&zip, &AtomicBool::new(false)).expect(leaf);
        assert_eq!(stats.bytes, original.len() as u64);
        assert_eq!(std::fs::read(&target).expect("target"), original, "{leaf}");
        assert!(!zip_path.exists());
        assert_eq!(names_in(dir.path()), [name.to_string_lossy().into_owned()]);
    }
}

#[test]
fn entry_names_are_ignored() {
    for name in ["../evil.pbo", "/abs/evil", "sub/../../evil"] {
        let base = tempfile::tempdir().expect("tempdir");
        let dir = base.path().join("d");
        std::fs::create_dir(&dir).expect("mkdir");
        let content = b"payload";
        let zip = downloaded(&dir, &archive(&[(name, Some(content))]), content);
        unzip_file(&zip, &AtomicBool::new(false)).expect(name);
        assert_eq!(std::fs::read(dir.join("x.pbo")).expect("target"), content);
        assert_eq!(names_in(base.path()), ["d"]);
        assert_eq!(names_in(&dir), ["x.pbo"]);
    }
}

#[test]
fn only_the_first_entry_is_used() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = downloaded(
        dir.path(),
        &archive(&[("a", Some(b"first")), ("b", Some(b"second"))]),
        b"first",
    );
    unzip_file(&zip, &AtomicBool::new(false)).expect("unzip");
    assert_eq!(
        std::fs::read(dir.path().join("x.pbo")).expect("target"),
        b"first"
    );
}

/// Runs a rejected unzip over an existing target and checks what is left.
fn rejected(bytes: &[u8], leaf: &[u8], tweak: impl FnOnce(&mut DownloadedZip)) -> UnzipError {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("x.pbo"), b"old").expect("old");
    let mut zip = downloaded(dir.path(), bytes, leaf);
    tweak(&mut zip);
    let err = unzip_file(&zip, &AtomicBool::new(false)).expect_err("rejected");
    assert_eq!(
        std::fs::read(dir.path().join("x.pbo")).expect("old target"),
        b"old"
    );
    assert!(!dir.path().join("x.pbo.part").exists());
    assert!(
        !zip.zip_path.exists(),
        "the archive is deleted after a failure"
    );
    assert_eq!(names_in(dir.path()), ["x.pbo"]);
    err
}

fn problem(err: &UnzipError) -> &ArchiveProblem {
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err:?}");
    match err {
        UnzipError::Archive { problem, .. } => problem,
        other => panic!("{other:?}"),
    }
}

#[test]
fn hostile_and_broken_archives_are_rejected() {
    // A symbolic link entry is refused and never created.
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .add_symlink("x.pbo", "/etc/passwd", SimpleFileOptions::default())
        .expect("symlink");
    let link = writer.finish().expect("finish").into_inner();
    let err = rejected(&link, b"/etc/passwd", |_| {});
    assert_eq!(problem(&err), &ArchiveProblem::Symlink);

    // A raw `S_IFLNK` mode on a regular entry is a link too, whatever system made the entry.
    for system in [System::Unix, System::Dos] {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .system(system)
            .external_attributes(0o120_777 << 16);
        writer.start_file("x", options).expect("start");
        writer.write_all(b"target").expect("write");
        let raw_link = writer.finish().expect("finish").into_inner();
        let err = rejected(&raw_link, b"target", |_| {});
        assert_eq!(problem(&err), &ArchiveProblem::Symlink, "{system:?}");
    }

    let err = rejected(&archive(&[("d/", None)]), b"x", |_| {});
    assert_eq!(problem(&err), &ArchiveProblem::Directory);

    // Larger than the leaf: a zip bomb guard.
    let big = vec![0u8; 2 * 1024 * 1024];
    let err = rejected(&archive(&[("x", Some(&big))]), &big[..1024], |_| {});
    assert_eq!(problem(&err), &ArchiveProblem::TooLarge { expected: 1024 });

    // Smaller than the leaf.
    let err = rejected(&archive(&[("x", Some(b"abc"))]), b"abcd", |_| {});
    assert_eq!(
        problem(&err),
        &ArchiveProblem::SizeMismatch {
            expected: 4,
            actual: 3
        }
    );

    // Same size, other content.
    let err = rejected(&archive(&[("x", Some(b"abc"))]), b"xyz", |_| {});
    assert!(matches!(problem(&err), ArchiveProblem::Sha1Mismatch { .. }));

    // Not an archive, and a truncated one.
    let err = rejected(b"garbage", b"x", |_| {});
    assert!(matches!(problem(&err), ArchiveProblem::Corrupt(_)));
    let whole = archive(&[("x", Some(&big))]);
    let err = rejected(&whole[..whole.len() / 2], &big, |_| {});
    assert!(matches!(problem(&err), ArchiveProblem::Corrupt(_)));

    // An empty archive is an empty file only for an empty leaf.
    let err = rejected(&archive(&[]), b"hello", |_| {});
    assert_eq!(problem(&err), &ArchiveProblem::Empty);
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = downloaded(dir.path(), &archive(&[]), b"");
    assert_eq!(zip.sha1.as_deref(), Some("0"));
    unzip_file(&zip, &AtomicBool::new(false)).expect("empty leaf");
    assert_eq!(
        std::fs::read(dir.path().join("x.pbo")).expect("target"),
        b""
    );
}

#[test]
fn checks_before_reading_keep_everything() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = DownloadedZip {
        relative_path: "x.pbo".to_owned(),
        zip_path: dir.path().join("x.pbo.zip"),
        target_path: dir.path().join("x.pbo"),
        sha1: None,
        size: 1,
    };
    let err = unzip_file(&missing, &AtomicBool::new(false)).expect_err("missing");
    assert!(matches!(err, UnzipError::NotFound { .. }), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::NotFound);

    let bytes = archive(&[("x", Some(b"a"))]);
    let wrong_extension = dir.path().join("x.pbo.zap");
    std::fs::write(&wrong_extension, &bytes).expect("write");
    let err = unzip_file(
        &DownloadedZip {
            zip_path: wrong_extension.clone(),
            target_path: dir.path().join("x.pbo"),
            ..missing.clone()
        },
        &AtomicBool::new(false),
    )
    .expect_err("extension");
    assert!(matches!(err, UnzipError::BadExtension { .. }), "{err:?}");
    assert!(wrong_extension.exists());

    let zip = downloaded(dir.path(), &bytes, b"a");
    for target in [dir.path().join("other.pbo"), dir.path().join("../x.pbo")] {
        let err = unzip_file(
            &DownloadedZip {
                target_path: target.clone(),
                ..zip.clone()
            },
            &AtomicBool::new(false),
        )
        .expect_err("target");
        assert!(matches!(err, UnzipError::UnsafeTarget { .. }), "{err:?}");
        assert!(!target.exists());
    }
    assert!(zip.zip_path.exists());
}

#[test]
fn cancel_keeps_the_archive_and_the_old_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("x.pbo"), b"old").expect("old");
    let zip = downloaded(dir.path(), &archive(&[("x", Some(b"new"))]), b"new");
    let err = unzip_file(&zip, &AtomicBool::new(true)).expect_err("cancelled");
    assert!(matches!(err, UnzipError::Cancelled));
    assert!(zip.zip_path.exists());
    assert!(!dir.path().join("x.pbo.part").exists());
    assert_eq!(
        std::fs::read(dir.path().join("x.pbo")).expect("old"),
        b"old"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_extracts_reports_and_cancels() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = UnzipPool::with_workers(2);

    // Idle: no event.
    let (events, rx) = EventSender::channel();
    let summary = pool.finish(&events).await;
    assert_eq!(summary.extracted, 0);
    assert!(rx.try_iter().next().is_none());

    // Six good archives and a broken one.
    let mut targets = Vec::new();
    for i in 0..6 {
        let content = format!("content {i}").repeat(50_000);
        let zip_path = dir.path().join(format!("f{i}.pbo.zip"));
        std::fs::write(&zip_path, archive(&[("f", Some(content.as_bytes()))])).expect("zip");
        let target = dir.path().join(format!("f{i}.pbo"));
        pool.submit(DownloadedZip {
            relative_path: format!("@a/f{i}.pbo"),
            zip_path,
            target_path: target.clone(),
            sha1: Some(sha1_of(content.as_bytes())),
            size: content.len() as u64,
        });
        targets.push((target, content));
    }
    std::fs::write(dir.path().join("bad.pbo.zip"), b"garbage").expect("bad");
    pool.submit(DownloadedZip {
        relative_path: "@a/bad.pbo".to_owned(),
        zip_path: dir.path().join("bad.pbo.zip"),
        target_path: dir.path().join("bad.pbo"),
        sha1: None,
        size: 3,
    });
    let summary = pool.finish(&EventSender::noop()).await;
    assert!(pool.is_idle());
    assert_eq!(summary.extracted, 6);
    assert_eq!(summary.errors.len(), 1);
    assert_eq!(
        summary.errors[0].relative_path.as_deref(),
        Some("@a/bad.pbo")
    );
    assert_eq!(summary.errors[0].kind, ErrorKind::InvalidData);
    for (target, content) in &targets {
        assert_eq!(std::fs::read(target).expect("target"), content.as_bytes());
    }

    // Cancel during a large extraction (256 MiB of zeros, a few hundred KiB compressed).
    let big_len: u64 = 256 * 1024 * 1024;
    let zip_path = dir.path().join("big.pbo.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).expect("big zip"));
    writer
        .start_file("big", SimpleFileOptions::default().large_file(true))
        .expect("start");
    let chunk = vec![0u8; 1024 * 1024];
    for _ in 0..big_len / chunk.len() as u64 {
        writer.write_all(&chunk).expect("write");
    }
    writer.finish().expect("finish");
    let pool = UnzipPool::with_workers(1);
    pool.submit(DownloadedZip {
        relative_path: "@a/big.pbo".to_owned(),
        zip_path: zip_path.clone(),
        target_path: dir.path().join("big.pbo"),
        sha1: None,
        size: big_len,
    });
    assert!(!pool.is_idle());
    tokio::time::sleep(Duration::from_millis(20)).await;
    pool.cancel();
    let started = Instant::now();
    let summary = pool.finish(&EventSender::noop()).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(summary.errors.is_empty());
    assert!(zip_path.exists(), "a cancelled archive is kept");
    assert!(!dir.path().join("big.pbo.part").exists());
    assert!(!dir.path().join("big.pbo").exists());

    // Submissions after a cancel are ignored.
    pool.submit(DownloadedZip {
        relative_path: "@a/big.pbo".to_owned(),
        zip_path: zip_path.clone(),
        target_path: dir.path().join("big.pbo"),
        sha1: None,
        size: big_len,
    });
    assert!(pool.is_idle());
}

/// Archives handed to a recording extractor that takes 30 ms each.
struct Recorder {
    started: parking_lot::Mutex<Vec<String>>,
    running: AtomicUsize,
    max_running: AtomicUsize,
}

impl Recorder {
    fn pool(workers: usize) -> (Arc<Self>, Arc<UnzipPool>) {
        let recorder = Arc::new(Self {
            started: parking_lot::Mutex::new(Vec::new()),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
        });
        let seen = Arc::clone(&recorder);
        let pool = UnzipPool::with_extractor(
            workers,
            Arc::new(move |zip: &DownloadedZip, _cancel: &AtomicBool| {
                seen.started.lock().push(zip.relative_path.clone());
                let now = seen.running.fetch_add(1, Ordering::SeqCst) + 1;
                seen.max_running.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                seen.running.fetch_sub(1, Ordering::SeqCst);
                Ok(UnzipStats { bytes: 1 })
            }),
        );
        (recorder, pool)
    }
}

fn submit_numbered(pool: &UnzipPool, count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            let relative_path = format!("@a/f{i}.pbo");
            pool.submit(DownloadedZip {
                relative_path: relative_path.clone(),
                zip_path: PathBuf::from(format!("/nowhere/f{i}.pbo.zip")),
                target_path: PathBuf::from(format!("/nowhere/f{i}.pbo")),
                sha1: None,
                size: 1,
            });
            relative_path
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_limits_workers_and_starts_in_submission_order() {
    // Two workers: never more than two extractions at once, and both are used.
    let (recorder, pool) = Recorder::pool(2);
    submit_numbered(&pool, 6);
    let (events, rx) = EventSender::channel();
    let summary = pool.finish(&events).await;
    assert_eq!(summary.extracted, 6);
    assert_eq!(recorder.max_running.load(Ordering::SeqCst), 2);

    // Work was pending when `finish` started: the phase, then increasing percentages to 100.
    let events: Vec<SyncEvent> = rx.try_iter().collect();
    assert_eq!(
        events.first(),
        Some(&SyncEvent::Phase(Phase::Uncompressing))
    );
    let percents: Vec<u8> = events[1..]
        .iter()
        .map(|e| match e {
            SyncEvent::Progress {
                phase: Phase::Uncompressing,
                percent,
            } => *percent,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(percents.last(), Some(&100), "{percents:?}");
    assert!(percents.windows(2).all(|w| w[0] < w[1]), "{percents:?}");

    // One worker: archives start exactly in submission order.
    let (recorder, pool) = Recorder::pool(1);
    let submitted = submit_numbered(&pool, 5);
    pool.finish(&EventSender::noop()).await;
    assert_eq!(*recorder.started.lock(), submitted);
    assert_eq!(recorder.max_running.load(Ordering::SeqCst), 1);
}
