//! Download executor against a local libunftp server.

use crate::download_support;

use std::sync::Arc;

use a3s_foundation::error::ErrorKind;
use a3s_sync::download::{CollectZips, DownloadEnd, NoZipSink, download};
use a3s_sync::{DownloadStatus, EventSender, SyncEvent};
use a3s_transport::CancellationToken;
use download_support::{
    FtpServer, compressed_task, drain, file_task, part_files, random_bytes, request, sha1,
    with_complete,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_downloads_and_errors() {
    let server = FtpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let cancel = CancellationToken::new();

    // Three files over two connections.
    let mut tasks = Vec::new();
    for i in 0..3u64 {
        let relative = format!("@a/addons/f{i}.pbo");
        let size = 20_000 + i * 7_000;
        server.put(&relative, &random_bytes(size as usize, i));
        tasks.push(file_task(dest, &relative, size));
    }
    let (events, _rx) = EventSender::channel();
    let report = download(
        &request(server.protocol(), 2),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
        &events,
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for task in &tasks {
        assert_eq!(task.status, DownloadStatus::Done);
        assert_eq!(
            sha1(&task.local_path()),
            sha1(&server.path(&task.relative_path))
        );
    }

    // Expected size larger than the server file: one retry, then an incomplete-transfer error.
    server.put("@b/short.pbo", &random_bytes(5_000, 9));
    let mut tasks = vec![file_task(dest, "@b/short.pbo", 8_000)];
    let (events, rx) = EventSender::channel();
    let report = download(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
        &events,
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(report.errors.len(), 1);
    let error = &report.errors[0].error;
    assert_eq!(error.kind(), ErrorKind::IncompleteTransfer);
    let target = std::path::absolute(dest.join("@b/short.pbo")).expect("absolute");
    assert_eq!(
        error.to_string(),
        a3s_foundation::error::messages::incomplete_transfer(
            &target.to_string_lossy(),
            5_000,
            8_000
        )
    );
    let failed: Vec<_> = drain(&rx)
        .into_iter()
        .filter(|e| matches!(e, SyncEvent::FileFailed { .. }))
        .collect();
    assert_eq!(failed.len(), 1);
    assert_eq!(tasks[0].status, DownloadStatus::Running);
    // The first attempt received the whole server file; the retry resumed at its end.
    assert_eq!(report.bytes_received, 5_000);

    // Resume a `Running` task from a half-length `.part`.
    let content = random_bytes(40_000, 10);
    server.put("@c/resume.pbo", &content);
    std::fs::create_dir_all(dest.join("@c")).expect("mkdir");
    std::fs::write(dest.join("@c/resume.pbo.part"), &content[..20_000]).expect("part");
    let mut task = file_task(dest, "@c/resume.pbo", content.len() as u64);
    task.status = DownloadStatus::Running;
    let mut tasks = vec![task];
    let report = download(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
        &EventSender::noop(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.errors
    );
    assert_eq!(
        std::fs::read(dest.join("@c/resume.pbo")).expect("file"),
        content
    );
    assert_eq!(report.bytes_received, 20_000);
    assert!(
        part_files(dest)
            .iter()
            .all(|p| !p.ends_with("resume.pbo.part"))
    );

    // A missing file: not found, with the FTP connection core.
    let mut tasks = vec![file_task(dest, "@d/missing.pbo", 10)];
    let report = download(
        &request(server.protocol(), 1),
        &mut tasks,
        Arc::new(NoZipSink),
        &cancel,
        &EventSender::noop(),
    )
    .await;
    assert!(matches!(report.end, DownloadEnd::Completed));
    assert_eq!(report.errors.len(), 1);
    let error = &report.errors[0].error;
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ftp_with_completion_falls_back_to_full_download() {
    let server = FtpServer::start().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let zip = random_bytes(3_000, 21);
    server.put("@a/x.pbo.zip", &zip);
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    std::fs::write(dest.join("@a/x.pbo"), random_bytes(9_000, 22)).expect("old");
    let mut tasks = vec![with_complete(
        compressed_task(dest, "@a/x.pbo", 10_000, zip.len() as u64, "s"),
        40.0,
    )];
    let zips = Arc::new(CollectZips::new());
    let report = download(
        &request(server.protocol(), 1),
        &mut tasks,
        zips.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.errors
    );
    assert_eq!(std::fs::read(dest.join("@a/x.pbo.zip")).expect("zip"), zip);
    assert_eq!(zips.take().len(), 1);
    assert_eq!(tasks[0].status, DownloadStatus::Done);
}
