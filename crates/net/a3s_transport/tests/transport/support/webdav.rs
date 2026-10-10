//! dav-server WebDAV server in axum, with Basic auth and a request log.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_types::{Protocol, ProtocolType};
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use base64::Engine as _;
use dav_server::DavHandler;
use dav_server::fakels::FakeLs;
use dav_server::localfs::LocalFs;
use parking_lot::Mutex;

use super::ftp::AbortOnDrop;
use super::{Fixture, outer_with_root};

pub const USER: &str = "uploader";
pub const PASSWORD: &str = "secret";

/// One request seen by the server.
#[derive(Clone, Debug)]
pub struct Logged {
    pub method: String,
    pub path: String,
    pub content_length: Option<String>,
    pub content_encoding: Option<String>,
    pub authorization: Option<String>,
}

pub type Log = Arc<Mutex<Vec<Logged>>>;

fn header_value(req: &Request, name: header::HeaderName) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Serves `root`; requests must carry Basic auth for [`USER`]/[`PASSWORD`] unless
/// `require_auth` is false.
pub async fn serve(root: PathBuf, require_auth: bool) -> (u16, Log, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WebDAV");
    let port = listener.local_addr().expect("local addr").port();
    let dav = DavHandler::builder()
        .filesystem(LocalFs::new(root, false, false, false))
        .locksystem(FakeLs::new())
        .build_handler();
    let log: Log = Arc::default();
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASSWORD}"))
    );
    let router_log = Arc::clone(&log);
    let router = axum::Router::new().fallback(move |req: Request| {
        let dav = dav.clone();
        let log = Arc::clone(&router_log);
        let expected = expected.clone();
        async move {
            let authorization = header_value(&req, header::AUTHORIZATION);
            log.lock().push(Logged {
                method: req.method().to_string(),
                path: req.uri().path().to_owned(),
                content_length: header_value(&req, header::CONTENT_LENGTH),
                content_encoding: header_value(&req, header::CONTENT_ENCODING),
                authorization: authorization.clone(),
            });
            if require_auth && authorization.as_deref() != Some(expected.as_str()) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let response = dav.handle(req).await;
            let (parts, body) = response.into_parts();
            Response::from_parts(parts, Body::new(body))
        }
    });
    let task = tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    });
    (port, log, task)
}

/// HTTP-WebDAV fixture with the given credentials (the server requires [`USER`] /
/// [`PASSWORD`] unless `require_auth` is false).
pub async fn fixture_with(login: &str, password: &str, require_auth: bool) -> (Fixture, Log) {
    let (outer, root) = outer_with_root();
    let (port, log, task) = serve(root.clone(), require_auth).await;
    let protocol = Protocol::new(
        ProtocolType::HttpWebdav,
        "127.0.0.1",
        port.to_string(),
        login,
        password,
        false,
    );
    let count_log = Arc::clone(&log);
    let fixture = Fixture {
        protocol,
        base: root,
        outer,
        request_count: Some(Box::new(move || count_log.lock().len())),
        _server: Box::new(AbortOnDrop(task)),
    };
    (fixture, log)
}

pub async fn fixture() -> Fixture {
    fixture_with(USER, PASSWORD, true).await.0
}
