//! In-process servers and repository fixtures for the check, upload and events tests.
//!
//! The FTP(S), SFTP and WebDAV servers and the TCP relay come from `a3s_test_servers`.

pub use a3s_test_servers as servers;

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use a3s_sync::{EventSender, SyncEvent};
use a3s_types::local::Repository;
use a3s_types::{Protocol, ProtocolType};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse as _, Response};
use parking_lot::Mutex;

pub use a3s_test_servers::AbortOnDrop;

/// The golden fixtures folder.
pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures")
}

/// A Java-built sample repository (`java_built/<variant>/<step>/repo`).
pub fn built_repo(variant: &str, step: &str) -> PathBuf {
    fixtures()
        .join("java_built")
        .join(variant)
        .join(step)
        .join("repo")
}

/// Copies a directory tree.
pub fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Files of the sample repositories that are not in their sync tree.
pub const NOT_IN_SYNC: [&str; 3] = ["excluded.txt", "notes.a3s.txt", "readme.zsync.txt"];

/// `relative path → contents` of every file below `dir` (directories as `None`).
pub fn tree(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    servers::snapshot(dir).into_iter().collect()
}

/// Files below `dir` (no directories), without `.a3s/`.
pub fn data_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    tree(dir)
        .into_iter()
        .filter_map(|(path, contents)| Some((path, contents?)))
        .filter(|(path, _)| !path.starts_with(".a3s"))
        .collect()
}

pub fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .expect("gunzip");
    raw
}

/// A repository whose download protocol is `download` and whose main folder is `path`.
pub fn repository(download: ProtocolType, path: Option<&Path>) -> Repository {
    let protocol = Protocol::new(
        download,
        "repo.example.org/arma3",
        "80",
        "anonymous",
        "",
        false,
    );
    let mut repository = Repository::new("Sample", protocol);
    repository.path = path.map(|p| p.to_str().expect("utf-8 path").to_owned());
    repository
}

/// Collects events.
#[derive(Clone, Default)]
pub struct Events(Arc<Mutex<Vec<SyncEvent>>>);

impl Events {
    pub fn sender(&self) -> EventSender {
        let events = Arc::clone(&self.0);
        EventSender::from_fn(move |event| events.lock().push(event))
    }

    pub fn all(&self) -> Vec<SyncEvent> {
        self.0.lock().clone()
    }

    pub fn count(&self, pred: impl Fn(&SyncEvent) -> bool) -> usize {
        self.0.lock().iter().filter(|e| pred(e)).count()
    }
}

// ---------------------------------------------------------------------------------------------
// HTTP

/// One request seen by [`HttpServer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logged {
    pub method: String,
    pub path: String,
}

/// Behaviour of [`HttpServer`].
#[derive(Default)]
pub struct HttpState {
    pub log: Mutex<Vec<Logged>>,

    /// Delay before every `HEAD` answer.
    pub head_delay: Mutex<Duration>,

    /// Status forced for a URL path (e.g. `/.a3s/changelogs` → 404).
    pub fail: Mutex<BTreeMap<String, u16>>,

    in_flight: AtomicUsize,
    pub max_in_flight: AtomicUsize,
}

/// Static HTTP server over a directory (`GET` and `HEAD`).
pub struct HttpServer {
    pub protocol: Protocol,
    pub state: Arc<HttpState>,
    _task: AbortOnDrop,
}

impl HttpServer {
    pub async fn serve(root: &Path) -> Self {
        let state = Arc::new(HttpState::default());
        let handler_state = Arc::clone(&state);
        let served = root.to_path_buf();
        let router = axum::Router::new().fallback(move |req: Request| {
            let state = Arc::clone(&handler_state);
            let root = served.clone();
            let method = req.method().clone();
            let path = req.uri().path().to_owned();
            async move { answer(&state, &root, &method, path).await }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind HTTP");
        let port = listener.local_addr().expect("addr").port();
        let task = tokio::spawn(async move {
            let _ignored = axum::serve(listener, router).await;
        });
        let protocol = Protocol::new(
            ProtocolType::Http,
            "127.0.0.1",
            port.to_string(),
            "anonymous",
            "",
            false,
        );
        Self {
            protocol,
            state,
            _task: AbortOnDrop(task),
        }
    }

    pub fn requests(&self, method: &str) -> Vec<String> {
        self.state
            .log
            .lock()
            .iter()
            .filter(|l| l.method == method)
            .map(|l| l.path.clone())
            .collect()
    }
}

async fn answer(state: &HttpState, root: &Path, method: &Method, path: String) -> Response {
    state.log.lock().push(Logged {
        method: method.to_string(),
        path: path.clone(),
    });
    let head = method == Method::HEAD;
    if head {
        let now = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        state.max_in_flight.fetch_max(now, Ordering::SeqCst);
        let delay = *state.head_delay.lock();
        tokio::time::sleep(delay).await;
        state.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
    if let Some(status) = state.fail.lock().get(&path) {
        return StatusCode::from_u16(*status)
            .expect("status")
            .into_response();
    }
    let decoded = percent_decode(path.trim_start_matches('/'));
    match std::fs::read(root.join(decoded)) {
        Ok(bytes) if head => {
            [(axum::http::header::CONTENT_LENGTH, bytes.len().to_string())].into_response()
        }
        Ok(bytes) => bytes.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = path.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).expect("utf-8 path")
}

// ---------------------------------------------------------------------------------------------
// WebDAV with a request hook

/// Called with the method and path of every WebDAV request, before it is handled.
pub type DavHook = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// A WebDAV server over `root` (no authentication) that logs every request and calls `hook`.
pub struct DavServer {
    pub protocol: Protocol,
    pub log: Arc<Mutex<Vec<Logged>>>,
    _task: AbortOnDrop,
}

impl DavServer {
    pub async fn serve(root: &Path, hook: Option<DavHook>) -> Self {
        let dav = dav_server::DavHandler::builder()
            .filesystem(dav_server::localfs::LocalFs::new(root, false, false, false))
            .locksystem(dav_server::fakels::FakeLs::new())
            .build_handler();
        let log: Arc<Mutex<Vec<Logged>>> = Arc::default();
        let router_log = Arc::clone(&log);
        let router = axum::Router::new().fallback(move |req: Request| {
            let dav = dav.clone();
            let method = req.method().to_string();
            let path = req.uri().path().to_owned();
            router_log.lock().push(Logged {
                method: method.clone(),
                path: path.clone(),
            });
            if let Some(hook) = &hook {
                hook(&method, &path);
            }
            async move {
                let response = dav.handle(req).await;
                let (parts, body) = response.into_parts();
                Response::from_parts(parts, axum::body::Body::new(body))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind WebDAV");
        let port = listener.local_addr().expect("addr").port();
        let task = tokio::spawn(async move {
            let _ignored = axum::serve(listener, router).await;
        });
        let protocol = Protocol::new(
            ProtocolType::HttpWebdav,
            "127.0.0.1",
            port.to_string(),
            "anonymous",
            "",
            false,
        );
        Self {
            protocol,
            log,
            _task: AbortOnDrop(task),
        }
    }

    /// `METHOD path` of every request except listings and locks.
    pub fn writes(&self) -> Vec<String> {
        self.log
            .lock()
            .iter()
            .filter(|l| matches!(l.method.as_str(), "PUT" | "DELETE" | "MKCOL"))
            .map(|l| format!("{} {}", l.method, l.path))
            .collect()
    }
}
