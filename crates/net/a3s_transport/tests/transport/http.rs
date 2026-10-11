//! HTTP `exists` and the unsupported write operations.

use std::path::Path;

use a3s_transport::{Error, RemoteKind, TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse as _;

use crate::support::AbortOnDrop;

#[tokio::test]
async fn http_exists_and_unsupported_writes() {
    let router = axum::Router::new().fallback(|req: axum::extract::Request| async move {
        let path = req.uri().path().to_owned();
        let head = req.method() == Method::HEAD;
        match path.as_str() {
            "/repo/ok.bin" => StatusCode::OK.into_response(),
            "/repo/secret.bin" => StatusCode::UNAUTHORIZED.into_response(),
            "/repo/nohead.bin" if head => StatusCode::METHOD_NOT_ALLOWED.into_response(),
            "/repo/nohead.bin" => (StatusCode::OK, "body").into_response(),
            "/repo/teapot.bin" => StatusCode::IM_A_TEAPOT.into_response(),
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = AbortOnDrop(tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    }));
    let protocol = Protocol::new(
        ProtocolType::Http,
        "127.0.0.1/repo",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();

    assert!(!t.supports_upload());
    assert!(t.exists("ok.bin", &ctx).await.unwrap());
    assert!(!t.exists("missing.bin", &ctx).await.unwrap());
    assert!(t.exists("nohead.bin", &ctx).await.unwrap());
    let result = t.exists("secret.bin", &ctx).await;
    assert!(matches!(result, Err(Error::Unauthorized)), "{result:?}");
    let result = t.exists("teapot.bin", &ctx).await;
    assert!(
        matches!(result, Err(Error::HttpStatus { status: 418, .. })),
        "{result:?}"
    );

    let unsupported = |result: Result<(), Error>, operation: &str| {
        assert!(
            matches!(result, Err(Error::Unsupported { protocol: "HTTP", operation: op }) if op == operation),
            "{result:?}"
        );
    };
    let local = tempfile::NamedTempFile::new().unwrap();
    unsupported(
        t.upload_file(Path::new(local.path()), "f.bin", &ctx)
            .await
            .map(|_| ()),
        "upload",
    );
    unsupported(t.upload_bytes(b"x", "f.bin", &ctx).await, "upload");
    unsupported(t.mkdirs("d", &ctx).await, "mkdir");
    unsupported(t.delete("f.bin", RemoteKind::File, &ctx).await, "delete");
}

/// `probe_range` returns the status and headers of `Range: bytes=0-1`; error statuses fail.
#[tokio::test]
async fn http_probe_range() {
    let router = axum::Router::new().fallback(|req: axum::extract::Request| async move {
        let range = req.headers().get("range").cloned();
        match (req.uri().path(), range) {
            ("/repo/.a3s/sync", Some(range)) if range == "bytes=0-1" => (
                StatusCode::PARTIAL_CONTENT,
                [("content-range", "bytes 0-1/10")],
                "ab",
            )
                .into_response(),
            ("/repo/plain.bin", _) => {
                (StatusCode::OK, [("x-server", "plain")], "whole").into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = AbortOnDrop(tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    }));
    let protocol = Protocol::new(
        ProtocolType::Http,
        "127.0.0.1/repo",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    assert!(t.supports_partial_transfer(&ctx).await);
    let Transport::Http(http) = &mut t else {
        panic!("an HTTP transport");
    };

    let partial = http.probe_range(".a3s/sync", &ctx).await.unwrap();
    assert_eq!(partial.status, 206);
    assert!(partial.supports_ranges());
    assert!(
        partial
            .headers
            .contains(&("content-range".to_owned(), "bytes 0-1/10".to_owned())),
        "{partial:?}"
    );

    let ignored = http.probe_range("plain.bin", &ctx).await.unwrap();
    assert_eq!(ignored.status, 200);
    assert!(!ignored.supports_ranges());
    assert!(
        ignored
            .headers
            .contains(&("x-server".to_owned(), "plain".to_owned())),
        "{ignored:?}"
    );

    let missing = http.probe_range("missing.bin", &ctx).await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
    let unsafe_path = http.probe_range("../x", &ctx).await;
    assert!(
        matches!(unsafe_path, Err(Error::InvalidPath(_))),
        "{unsafe_path:?}"
    );
}
