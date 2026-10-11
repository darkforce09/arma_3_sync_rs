//! SFTP against the russh test server.

use a3s_transport::{Error, TransferCtx, Transport, TransportOptions};

use crate::common;
use crate::support::sftp::{PASSWORD, fixture, fixture_with};

#[tokio::test]
async fn upload_file_round_trip() {
    common::upload_file_round_trip(&fixture().await).await;
}

#[tokio::test]
async fn upload_bytes_identity() {
    common::upload_bytes_identity(&fixture().await).await;
}

#[tokio::test]
async fn exists() {
    common::exists(&fixture().await).await;
}

#[tokio::test]
async fn mkdirs_idempotent() {
    common::mkdirs_idempotent(&fixture().await).await;
}

#[tokio::test]
async fn delete() {
    common::delete(&fixture().await).await;
}

#[tokio::test]
async fn path_traversal() {
    common::path_traversal(&fixture().await).await;
}

#[tokio::test]
async fn cancellation() {
    common::cancellation(&fixture().await, false).await;
}

#[tokio::test]
async fn a_resume_sftp() {
    common::resume_download(&fixture().await).await;
}

#[tokio::test]
async fn base_directory() {
    let ctx = TransferCtx::new();

    // Empty remote path: base is the home directory (`canonicalize(".")`).
    let fx = fixture_with("/home/a3s", "", PASSWORD, "/home/a3s").await;
    let mut t = fx.connect().await;
    t.upload_bytes(b"home", "x.bin", &ctx).await.unwrap();
    assert_eq!(
        std::fs::read(fx.root().join("home/a3s/x.bin")).unwrap(),
        b"home"
    );
    assert_eq!(t.fetch("x.bin", &ctx).await.unwrap(), b"home");
    t.close().await;

    // A configured base that does not exist is created by the upload (Java SFTP).
    let fx = fixture_with("/", "/new/base", PASSWORD, "/new/base").await;
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("f");
    std::fs::write(&local, b"based").unwrap();
    let mut t = fx.connect().await;
    t.upload_file(&local, "f.bin", &ctx).await.unwrap();
    assert_eq!(
        std::fs::read(fx.root().join("new/base/f.bin")).unwrap(),
        b"based"
    );
    t.close().await;

    // `..` in the configured path is refused at connect.
    let fx = fixture_with("/", "/a/../b", PASSWORD, "").await;
    let options = TransportOptions::from_protocol(&fx.protocol);
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(
        matches!(result, Err(Error::InvalidPath(_))),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn sftp_url_with_scheme_connects() {
    let fx = fixture().await;
    let mut protocol = fx.protocol.clone();
    protocol.url = format!("sftp:///{}", protocol.url);
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    t.upload_bytes(b"s", "s.bin", &TransferCtx::new())
        .await
        .unwrap();
    assert!(fx.base.join("s.bin").is_file());
    t.close().await;
}

#[tokio::test]
async fn authentication() {
    let fx = fixture_with("/", "", "", "").await;
    let options = TransportOptions::from_protocol(&fx.protocol);
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(
        matches!(result, Err(Error::InvalidProtocol(ref m)) if m.contains("password is empty")),
        "{:?}",
        result.err()
    );

    let fx = fixture_with("/", "", "wrong", "").await;
    let options = TransportOptions::from_protocol(&fx.protocol);
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(
        matches!(result, Err(Error::Unauthorized)),
        "{:?}",
        result.err()
    );
}
