//! HTTP-WebDAV against dav-server.

use std::sync::Arc;

use a3s_transport::{Error, TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse as _;
use parking_lot::Mutex;

use crate::common;
use crate::support::ftp::AbortOnDrop;
use crate::support::webdav::{PASSWORD, USER, fixture, fixture_with};

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
async fn protocol_details() {
    let (fx, log) = fixture_with(USER, PASSWORD, true).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("f");
    std::fs::write(&local, b"0123456789").unwrap();

    {
        let mut session = t.upload_session();
        session
            .upload_file(&local, "m a/b/one.bin", &ctx)
            .await
            .unwrap();
        let mkcols: Vec<_> = log
            .lock()
            .iter()
            .filter(|r| r.method == "MKCOL")
            .map(|r| r.path.clone())
            .collect();
        assert_eq!(mkcols, ["/m%20a/", "/m%20a/b/"]);
        let put = log
            .lock()
            .iter()
            .find(|r| r.method == "PUT")
            .cloned()
            .expect("PUT logged");
        assert_eq!(put.path, "/m%20a/b/one.bin");
        assert_eq!(put.content_length.as_deref(), Some("10"));
        assert_eq!(put.content_encoding, None);
        assert!(put.authorization.is_some_and(|a| a.starts_with("Basic ")));

        let before = log.lock().len();
        session
            .upload_file(&local, "m a/b/two.bin", &ctx)
            .await
            .unwrap();
        let methods: Vec<_> = log.lock()[before..]
            .iter()
            .map(|r| r.method.clone())
            .collect();
        assert_eq!(methods, ["PUT"]);
    }
    assert_eq!(
        std::fs::read(fx.base.join("m a/b/two.bin")).unwrap(),
        b"0123456789"
    );
    t.close().await;

    // Anonymous: no Authorization header.
    let (fx, log) = fixture_with("anonymous", "", false).await;
    let mut t = fx.connect().await;
    t.upload_bytes(b"anon", "anon.bin", &ctx).await.unwrap();
    assert!(log.lock().iter().all(|r| r.authorization.is_none()));
    t.close().await;

    // Wrong password.
    let (fx, _log) = fixture_with(USER, "wrong", true).await;
    let mut t = fx.connect().await;
    let result = t.upload_bytes(b"x", "x.bin", &ctx).await;
    assert!(matches!(result, Err(Error::Unauthorized)), "{result:?}");
    let result = t.exists("x.bin", &ctx).await;
    assert!(matches!(result, Err(Error::Unauthorized)), "{result:?}");
    t.close().await;
}

#[tokio::test]
async fn write_verbs_do_not_follow_redirects() {
    let hits: Arc<Mutex<Vec<String>>> = Arc::default();
    let router_hits = Arc::clone(&hits);
    let router = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let hits = Arc::clone(&router_hits);
        async move {
            hits.lock()
                .push(format!("{} {}", req.method(), req.uri().path()));
            if req.uri().path().starts_with("/target") {
                StatusCode::CREATED.into_response()
            } else {
                (StatusCode::FOUND, [(header::LOCATION, "/target/f.bin")]).into_response()
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = AbortOnDrop(tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    }));
    let protocol = Protocol::new(
        ProtocolType::HttpWebdav,
        "127.0.0.1",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let result = t.upload_bytes(b"x", "f.bin", &TransferCtx::new()).await;
    assert!(
        matches!(result, Err(Error::HttpStatus { status: 302, .. })),
        "{result:?}"
    );
    let result = t
        .delete(
            "f.bin",
            a3s_transport::RemoteKind::File,
            &TransferCtx::new(),
        )
        .await;
    assert!(
        matches!(result, Err(Error::HttpStatus { status: 302, .. })),
        "{result:?}"
    );
    assert_eq!(*hits.lock(), ["PUT /f.bin", "DELETE /f.bin"]);
}

/// 409 Conflict on `MKCOL`/`PUT` is a missing parent (`NotFound`); a 207 answer to `DELETE`
/// is a partial failure, never success.
#[tokio::test]
async fn conflict_and_partial_delete() {
    let router = axum::Router::new().fallback(|req: axum::extract::Request| async move {
        match (req.method().as_str(), req.uri().path()) {
            ("PROPFIND", "/c/") => StatusCode::NOT_FOUND.into_response(),
            ("MKCOL", "/c/") | ("PUT", "/put.bin") => StatusCode::CONFLICT.into_response(),
            ("DELETE", "/tree/") => StatusCode::MULTI_STATUS.into_response(),
            _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = AbortOnDrop(tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    }));
    let protocol = Protocol::new(
        ProtocolType::HttpWebdav,
        "127.0.0.1",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    let result = t.mkdirs("c", &ctx).await;
    assert!(matches!(result, Err(Error::NotFound(_))), "{result:?}");
    let result = t.upload_bytes(b"x", "put.bin", &ctx).await;
    assert!(matches!(result, Err(Error::NotFound(_))), "{result:?}");
    let result = t
        .delete("tree", a3s_transport::RemoteKind::Directory, &ctx)
        .await;
    assert!(matches!(result, Err(Error::Remote { .. })), "{result:?}");
}
