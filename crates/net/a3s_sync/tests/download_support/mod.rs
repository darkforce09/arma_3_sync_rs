//! Local servers and helpers for the download executor tests.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use a3s_sync::download::{DownloadRequest, DownloadedZip};
use a3s_sync::{DownloadStatus, DownloadTask, DownloadTaskKind, SyncEvent};
use a3s_transport::{Throttle, TransportOptions};
use a3s_types::{Protocol, ProtocolType};
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use parking_lot::Mutex;

// ---------------------------------------------------------------------------------------------
// Range-capable HTTP server

/// Per-path behaviour of the HTTP server.
#[derive(Clone, Debug, Default)]
pub struct Rule {
    /// Ignore `Range` (always 200 with the whole body).
    pub no_range: bool,

    /// Wait before sending the headers.
    pub delay: Option<Duration>,

    /// Send this many body bytes, then hang (for `stall_for`, or forever).
    pub stall_after: Option<usize>,

    /// How long the stall lasts; `None` = forever.
    pub stall_for: Option<Duration>,

    /// Send this many body bytes, then abort the response with an error.
    pub drop_after: Option<usize>,

    /// Apply `drop_after` to the first response of the path only.
    pub drop_first_only: bool,

    /// Answer this status with an empty body.
    pub status: Option<u16>,

    /// Send at most this many body bytes, with a matching `Content-Length` (a short body).
    pub truncate_body: Option<usize>,
}

/// One request seen by the server.
#[derive(Clone, Debug)]
pub struct Seen {
    /// Path below `/repo/`, e.g. `@a/x.pbo`.
    pub path: String,
    pub range: Option<String>,
    pub user_agent: Option<String>,
    pub authorization: Option<String>,
}

struct ServerState {
    root: PathBuf,
    rules: Mutex<HashMap<String, Rule>>,
    default_rule: Mutex<Rule>,
    /// Delay of every 200 answer.
    ok_delay: Mutex<Duration>,
    seen: Mutex<Vec<Seen>>,
    responses: Mutex<HashMap<String, usize>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

/// An axum server serving `root` under `/repo/`.
pub struct HttpServer {
    pub port: u16,
    pub root: PathBuf,
    _dir: tempfile::TempDir,
    state: Arc<ServerState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Decrements the in-flight counter when the response body is dropped.
struct InFlight(Arc<ServerState>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

fn header_string(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Parses `bytes=a-b` / `bytes=a-`.
fn parse_range(value: &str, len: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    if start >= len {
        return Some(Err(()));
    }
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<u64>().ok()?.min(len - 1)
    };
    if end < start {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

async fn handle(State(state): State<Arc<ServerState>>, request: Request) -> Response {
    let uri_path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    let path = percent_decode(uri_path.strip_prefix("/repo/").unwrap_or(&uri_path));
    let range = header_string(&headers, header::RANGE);
    state.seen.lock().push(Seen {
        path: path.clone(),
        range: range.clone(),
        user_agent: header_string(&headers, header::USER_AGENT),
        authorization: header_string(&headers, header::AUTHORIZATION),
    });
    let now = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_in_flight.fetch_max(now, Ordering::SeqCst);
    let guard = InFlight(Arc::clone(&state));

    let rule = state
        .rules
        .lock()
        .get(&path)
        .cloned()
        .unwrap_or_else(|| state.default_rule.lock().clone());
    let response_number = {
        let mut responses = state.responses.lock();
        let n = responses.entry(path.clone()).or_insert(0);
        *n += 1;
        *n
    };
    if let Some(delay) = rule.delay {
        tokio::time::sleep(delay).await;
    }
    if let Some(status) = rule.status {
        return Response::builder()
            .status(status)
            .body(Body::empty())
            .expect("response");
    }
    let Ok(content) = tokio::fs::read(state.root.join(&path)).await else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .expect("response");
    };
    let len = content.len() as u64;
    let mut builder = Response::builder();
    let ranged = range
        .as_deref()
        .filter(|_| !rule.no_range)
        .map(|value| parse_range(value, len));
    let mut body: Vec<u8> = if let Some(Some(parsed)) = ranged {
        let Ok((start, end)) = parsed else {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(Body::empty())
                .expect("response");
        };
        builder = builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
        content[start as usize..=end as usize].to_vec()
    } else {
        let delay = *state.ok_delay.lock();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        builder = builder.status(StatusCode::OK);
        content
    };
    if let Some(limit) = rule.truncate_body {
        body.truncate(limit);
    }
    builder = builder.header(header::CONTENT_LENGTH, body.len());

    let drop_after = rule
        .drop_after
        .filter(|_| !rule.drop_first_only || response_number == 1);
    let split = drop_after.or(rule.stall_after);
    let Some(split) = split.map(|n| n.min(body.len())) else {
        let stream = futures::stream::once(async move {
            let _guard = guard;
            Ok::<_, std::io::Error>(Bytes::from(body))
        });
        return builder.body(Body::from_stream(stream)).expect("response");
    };
    let head = Bytes::copy_from_slice(&body[..split]);
    let tail = Bytes::copy_from_slice(&body[split..]);
    let stall_for = rule.stall_for;
    let dropping = drop_after.is_some();
    let stream = async_stream(head, tail, dropping, stall_for, guard);
    builder.body(Body::from_stream(stream)).expect("response")
}

/// The body: `head`, then an error (drop), or a stall followed by `tail`.
fn async_stream(
    head: Bytes,
    tail: Bytes,
    dropping: bool,
    stall_for: Option<Duration>,
    guard: InFlight,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    use futures::StreamExt as _;
    let first = futures::stream::once(async move { Ok(head) });
    let rest = futures::stream::once(async move {
        let _guard = guard;
        if dropping {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return Err(std::io::Error::other(
                "connection dropped by the test server",
            ));
        }
        match stall_for {
            Some(duration) => tokio::time::sleep(duration).await,
            None => std::future::pending::<()>().await,
        }
        Ok(tail)
    });
    first.chain(rest)
}

fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl HttpServer {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).expect("mkdir");
        let state = Arc::new(ServerState {
            root: root.clone(),
            rules: Mutex::default(),
            default_rule: Mutex::default(),
            ok_delay: Mutex::new(Duration::ZERO),
            seen: Mutex::default(),
            responses: Mutex::default(),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        });
        let router = axum::Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        Self {
            port,
            root,
            _dir: dir,
            state,
            task,
        }
    }

    pub fn protocol(&self) -> Protocol {
        Protocol::new(
            ProtocolType::Http,
            "127.0.0.1/repo",
            self.port.to_string(),
            "anonymous",
            "",
            false,
        )
    }

    pub fn host_url(&self) -> String {
        format!("http://127.0.0.1:{}/repo", self.port)
    }

    /// Writes a server file below the repository root.
    pub fn put(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub fn rule(&self, path: &str, rule: Rule) {
        self.state.rules.lock().insert(path.to_owned(), rule);
    }

    pub fn clear_rules(&self) {
        self.state.rules.lock().clear();
        *self.state.default_rule.lock() = Rule::default();
    }

    pub fn default_rule(&self, rule: Rule) {
        *self.state.default_rule.lock() = rule;
    }

    pub fn ok_delay(&self, delay: Duration) {
        *self.state.ok_delay.lock() = delay;
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.state.seen.lock().clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.seen().into_iter().map(|s| s.path).collect()
    }

    pub fn clear_seen(&self) {
        self.state.seen.lock().clear();
        self.state.responses.lock().clear();
        self.state.max_in_flight.store(0, Ordering::SeqCst);
    }

    pub fn max_in_flight(&self) -> usize {
        self.state.max_in_flight.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------------------------
// libunftp FTP server

/// An anonymous FTP server over a tempdir.
pub struct FtpServer {
    pub port: u16,
    pub root: PathBuf,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FtpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FtpServer {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("repo")).expect("mkdir");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind FTP");
        let port = listener.local_addr().expect("addr").port();
        let served = root.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let served = served.clone();
                let server = libunftp::ServerBuilder::new(Box::new(move || {
                    unftp_sbe_fs::Filesystem::new(served.clone()).expect("filesystem back-end")
                }))
                .greeting("a3s download test server")
                .passive_ports(40000..=60000)
                .build()
                .expect("FTP server");
                tokio::spawn(async move {
                    let _ignored = server.service(stream).await;
                });
            }
        });
        Self {
            port,
            root: root.join("repo"),
            _dir: dir,
            task,
        }
    }

    pub fn protocol(&self) -> Protocol {
        Protocol::new(
            ProtocolType::Ftp,
            "127.0.0.1/repo",
            self.port.to_string(),
            "anonymous",
            "",
            false,
        )
    }

    pub fn host_url(&self) -> String {
        format!("ftp://127.0.0.1:{}/repo", self.port)
    }

    pub fn put(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

// ---------------------------------------------------------------------------------------------
// Tasks, requests, events

/// Deterministic pseudo-random bytes.
pub fn random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// A request with `connections` workers, no speed limit and no disk-usage sampling.
pub fn request(protocol: Protocol, connections: u32) -> DownloadRequest {
    let mut transport_options = TransportOptions::from_protocol(&protocol);
    transport_options.allow_insecure_tls = false;
    DownloadRequest {
        repository_name: "test".to_owned(),
        protocol,
        transport_options,
        connections,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
        throttle: Arc::new(Throttle::new(0)),
        disk_usage_path: None,
    }
}

/// A file task for the tree path `relative`, below `dest`.
pub fn file_task(dest: &Path, relative: &str, size: u64) -> DownloadTask {
    let parent = a3s_sync::parent_relative_path(relative);
    DownloadTask {
        relative_path: relative.to_owned(),
        destination_dir: if parent.is_empty() {
            dest.to_owned()
        } else {
            dest.join(parent)
        },
        kind: DownloadTaskKind::File {
            size,
            compressed: false,
            compressed_size: 0,
            sha1: None,
            complete: 0.0,
        },
        status: DownloadStatus::Pending,
    }
}

/// A compressed file task.
pub fn compressed_task(
    dest: &Path,
    relative: &str,
    size: u64,
    compressed_size: u64,
    sha1: &str,
) -> DownloadTask {
    let mut task = file_task(dest, relative, size);
    task.kind = DownloadTaskKind::File {
        size,
        compressed: true,
        compressed_size,
        sha1: Some(sha1.to_owned()),
        complete: 0.0,
    };
    task
}

/// A directory task.
pub fn dir_task(dest: &Path, relative: &str) -> DownloadTask {
    let mut task = file_task(dest, relative, 0);
    task.kind = DownloadTaskKind::Directory;
    task
}

/// Sets the zsync completion of a file task.
pub fn with_complete(mut task: DownloadTask, value: f64) -> DownloadTask {
    if let DownloadTaskKind::File { complete, .. } = &mut task.kind {
        *complete = value;
    }
    task
}

pub fn sha1(path: &Path) -> String {
    a3s_hash::sha1_file(path).expect("sha1")
}

/// Every `*.part` file below `dir`.
pub fn part_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.to_string_lossy().ends_with(".part") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}

/// A zip sink that is busy while `busy` is set.
#[derive(Default)]
pub struct BusySink {
    pub zips: Mutex<Vec<DownloadedZip>>,
}

impl a3s_sync::download::ZipSink for BusySink {
    fn submit(&self, zip: DownloadedZip) {
        self.zips.lock().push(zip);
    }

    fn is_idle(&self) -> bool {
        false
    }
}

/// Events received so far.
pub fn drain(rx: &std::sync::mpsc::Receiver<SyncEvent>) -> Vec<SyncEvent> {
    rx.try_iter().collect()
}
