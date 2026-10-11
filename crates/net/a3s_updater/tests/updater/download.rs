//! Archive download: progress, SHA-256 verification, size checks and cancellation.

use std::time::{Duration, Instant};

use a3s_updater::{
    CancellationToken, DownloadOptions, HttpClient, Release, ReleaseFormat, StagingDir,
    UpdateError, download_release,
};
use parking_lot::Mutex;

use crate::support::{Reply, Server, local_config, sha256_hex};

fn release(server: &Server, path: &str, sha256: Option<String>, size: u64) -> Release {
    Release {
        version: "2026.1.1".to_owned(),
        tag: None,
        file_name: "Arma3Sync-2026.1.1.zip".to_owned(),
        download_url: server.url(path),
        sha256,
        size,
        format: ReleaseFormat::Json,
        source: server.url("/a3s.json").to_string(),
    }
}

fn payload() -> Vec<Vec<u8>> {
    (0u8..8)
        .map(|i| vec![i; 64 * 1024 + usize::from(i)])
        .collect()
}

#[tokio::test]
async fn verified_download_with_progress() {
    let server = Server::start().await;
    let chunks = payload();
    let all: Vec<u8> = chunks.concat();
    server.route("/a.zip", Reply::Chunked(chunks));
    let http = HttpClient::new(&local_config("")).expect("client");
    let staging = StagingDir::new().expect("staging");
    let seen = Mutex::new(Vec::new());
    let progress = |bytes: u64| seen.lock().push(bytes);

    let archive = download_release(
        &http,
        &release(
            &server,
            "/a.zip",
            Some(sha256_hex(&all).to_ascii_uppercase()),
            0,
        ),
        &staging,
        DownloadOptions::default(),
        &progress,
        &CancellationToken::new(),
    )
    .await
    .expect("download");
    assert!(archive.verified);
    assert_eq!(archive.bytes, all.len() as u64);
    assert_eq!(archive.path, staging.path().join("Arma3Sync-2026.1.1.zip"));
    assert_eq!(std::fs::read(&archive.path).expect("read"), all);

    let seen = seen.lock();
    assert!(!seen.is_empty());
    assert!(
        seen.windows(2).all(|w| w[0] < w[1]),
        "strictly increasing: {seen:?}"
    );
    assert_eq!(seen.last().copied(), Some(all.len() as u64));
}

#[tokio::test]
async fn checksum_and_size_mismatches() {
    let server = Server::start().await;
    let body = vec![7u8; 1000];
    server.route("/a.zip", Reply::ok(body.clone()));
    server.route(
        "/chunked.zip",
        Reply::Chunked(vec![body[..500].to_vec(), body[500..].to_vec()]),
    );
    let http = HttpClient::new(&local_config("")).expect("client");
    let cancel = CancellationToken::new();
    let staging = StagingDir::new().expect("staging");
    let download = |release: Release| {
        let http = &http;
        let staging = &staging;
        let cancel = &cancel;
        async move {
            download_release(
                http,
                &release,
                staging,
                DownloadOptions::default(),
                &|_| {},
                cancel,
            )
            .await
        }
    };

    let wrong = "0".repeat(64);
    let err = download(release(&server, "/a.zip", Some(wrong), 0))
        .await
        .expect_err("sha");
    assert!(matches!(err, UpdateError::Integrity), "{err:?}");
    // A failed download never appears under the final name.
    assert!(!staging.path().join("Arma3Sync-2026.1.1.zip").exists());
    assert!(staging.path().join("Arma3Sync-2026.1.1.zip.part").is_file());

    let hash = Some(sha256_hex(&body));
    // Larger than announced: refused from `Content-Length` before any byte is written, and
    // while streaming when there is no `Content-Length`.
    let early = StagingDir::new().expect("staging");
    let err = download_release(
        &http,
        &release(&server, "/a.zip", hash.clone(), 999),
        &early,
        DownloadOptions::default(),
        &|_| {},
        &cancel,
    )
    .await
    .expect_err("larger");
    assert!(matches!(err, UpdateError::Manifest(_)), "{err:?}");
    assert_eq!(std::fs::read_dir(early.path()).expect("list").count(), 0);
    let err = download(release(&server, "/chunked.zip", hash.clone(), 999))
        .await
        .expect_err("larger, streamed");
    assert!(matches!(err, UpdateError::Manifest(_)), "{err:?}");
    let err = download(release(&server, "/a.zip", hash.clone(), 1001))
        .await
        .expect_err("smaller");
    assert!(matches!(err, UpdateError::Manifest(_)), "{err:?}");
    assert!(
        download(release(&server, "/a.zip", hash, 1000))
            .await
            .is_ok()
    );

    // No checksum: refused unless explicitly allowed.
    let err = download(release(&server, "/a.zip", None, 0))
        .await
        .expect_err("unverified");
    assert!(matches!(err, UpdateError::Unverified), "{err:?}");
    let archive = download_release(
        &http,
        &release(&server, "/a.zip", None, 0),
        &staging,
        DownloadOptions {
            allow_unverified: true,
        },
        &|_| {},
        &cancel,
    )
    .await
    .expect("allowed unverified");
    assert!(!archive.verified);

    let mut unsafe_name = release(&server, "/a.zip", Some(sha256_hex(&body)), 0);
    unsafe_name.file_name = "../a.zip".to_owned();
    assert!(download(unsafe_name).await.is_err());
}

#[tokio::test]
async fn cancellation_is_prompt_and_keeps_the_staging_directory() {
    let server = Server::start().await;
    server.route(
        "/slow.zip",
        Reply::Slow {
            chunk: vec![1u8; 1024],
            interval: Duration::from_millis(100),
            count: 600,
        },
    );
    let http = HttpClient::new(&local_config("")).expect("client");
    let staging = StagingDir::new().expect("staging");
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let cancelled_at = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        trigger.cancel();
        Instant::now()
    });
    let result = download_release(
        &http,
        &release(&server, "/slow.zip", Some("0".repeat(64)), 0),
        &staging,
        DownloadOptions::default(),
        &|_| {},
        &cancel,
    )
    .await;
    let returned = Instant::now();
    let cancelled_at = cancelled_at.await.expect("join");
    assert!(matches!(result, Err(UpdateError::Cancelled)), "{result:?}");
    assert!(returned.duration_since(cancelled_at) < Duration::from_secs(1));

    let dir = staging.path().to_path_buf();
    assert!(dir.join("Arma3Sync-2026.1.1.zip.part").is_file());
    assert!(!dir.join("Arma3Sync-2026.1.1.zip").exists());
    drop(staging);
    assert!(!dir.exists());
}
