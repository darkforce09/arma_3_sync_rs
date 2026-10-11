//! zsync completion against a local HTTP server.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use a3s_error::{ErrorKind, UserFacing as _};
use a3s_sync::completion::determine_completion;
use a3s_sync::plan::{NoAddons, assign_destinations};
use a3s_sync::{
    CompletionError, CompletionStats, EventSender, Phase, PlanNode, PlanTree, SyncEvent,
};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::{
    Protocol, ProtocolType, ServerInfo, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode,
};
use a3s_zsync::{BLOCK_SIZE, ControlFile};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use parking_lot::Mutex;

// ---------------------------------------------------------------------------------------------
// Local HTTP server

/// How the server answers.
#[derive(Clone, Default)]
struct Config {
    /// Answer `Range: bytes=0-1` on `.a3s/sync` with a 206 (else a full 200).
    ranges: bool,
    /// `.zsync` bodies by request path.
    zsync: HashMap<String, Vec<u8>>,
    /// Delay of every `.zsync` response.
    delay: Duration,
    /// `.zsync` requests that never answer (30 s).
    stall: bool,
}

#[derive(Clone)]
struct ServerState {
    config: Arc<Config>,
    /// `(path, has Range header)`, in arrival order.
    seen: Arc<Mutex<Vec<(String, bool)>>>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

struct TestServer {
    port: u16,
    state: ServerState,
}

impl TestServer {
    async fn start(config: Config) -> Self {
        let state = ServerState {
            config: Arc::new(config),
            seen: Arc::default(),
            in_flight: Arc::default(),
            max_in_flight: Arc::default(),
        };
        let router = axum::Router::new()
            .fallback(handle)
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        Self { port, state }
    }

    fn protocol(&self) -> Protocol {
        Protocol::new(
            ProtocolType::Http,
            "127.0.0.1/repo",
            self.port.to_string(),
            "anonymous",
            "",
            false,
        )
    }

    fn seen(&self) -> Vec<(String, bool)> {
        self.state.seen.lock().clone()
    }

    fn zsync_requests(&self) -> Vec<String> {
        self.seen()
            .into_iter()
            .map(|(p, _)| p)
            .filter(|p| p.ends_with(".zsync"))
            .collect()
    }
}

async fn handle(State(state): State<ServerState>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let range = request.headers().get(header::RANGE).cloned();
    state.seen.lock().push((path.clone(), range.is_some()));
    let config = &state.config;
    let status = |code: u16| {
        Response::builder()
            .status(code)
            .body(Body::empty())
            .expect("response")
    };
    if path == "/repo/.a3s/sync" {
        let body = b"0123456789".to_vec();
        return if config.ranges && range.is_some_and(|r| r == "bytes=0-1") {
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, "bytes 0-1/10")
                .body(Body::from(body[..2].to_vec()))
                .expect("response")
        } else {
            Response::new(Body::from(body))
        };
    }
    let now = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_in_flight.fetch_max(now, Ordering::SeqCst);
    if config.stall {
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
    if !config.delay.is_zero() {
        tokio::time::sleep(config.delay).await;
    }
    let response = match config.zsync.get(&path) {
        Some(body) => Response::new(Body::from(body.clone())),
        None => status(404),
    };
    state.in_flight.fetch_sub(1, Ordering::SeqCst);
    response
}

// ---------------------------------------------------------------------------------------------
// Fixtures

fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as u8
        })
        .collect()
}

fn zsync_of(content: &[u8], name: &str) -> Vec<u8> {
    ControlFile::generate(
        Cursor::new(content),
        content.len() as u64,
        name,
        "Sat, 10 Oct 2026 12:00:00 GMT",
        &format!("http://127.0.0.1/repo/@a/addons/{name}"),
        "sha",
    )
    .expect("generate")
    .to_bytes()
}

/// A remote file below `@a/addons`.
struct Remote {
    name: &'static str,
    updated: bool,
    /// Local content, if the file exists locally.
    local: Option<Vec<u8>>,
}

fn remote(name: &'static str, updated: bool, local: Option<Vec<u8>>) -> Remote {
    Remote {
        name,
        updated,
        local,
    }
}

/// A planned tree with `@a/addons/<files>`, the local files written below `base`.
fn planned(base: &Path, files: &[Remote]) -> PlanTree {
    let addons = base.join("@a/addons");
    std::fs::create_dir_all(&addons).expect("mkdir");
    let mut sync = SyncTreeDirectory::new_root();
    let mut a = SyncTreeDirectory::new("@a");
    a.mark_as_addon = true;
    let mut dir = SyncTreeDirectory::new("addons");
    for file in files {
        dir.add_child(SyncTreeNode::Leaf(SyncTreeLeaf {
            name: file.name.to_owned(),
            sha1: Some("remote".to_owned()),
            size: 4 * BLOCK_SIZE as i64,
            updated: file.updated,
            ..Default::default()
        }));
        if let Some(content) = &file.local {
            std::fs::write(addons.join(file.name), content).expect("write");
        }
    }
    a.add_child(SyncTreeNode::Directory(dir));
    sync.add_child(SyncTreeNode::Directory(a));
    let mut tree = PlanTree::from_sync_tree(&sync);
    assign_destinations(&mut tree, base, false, &NoAddons).expect("safe names");
    tree
}

fn complete(tree: &PlanTree, name: &str) -> f64 {
    match tree.get(&format!("@a/addons/{name}")) {
        Some(PlanNode::Leaf(l)) => l.complete,
        other => panic!("{name}: {other:?}"),
    }
}

fn server_info(connections: i32) -> ServerInfo {
    ServerInfo {
        number_of_connections: connections,
        ..Default::default()
    }
}

async fn run(
    tree: &mut PlanTree,
    protocol: &Protocol,
    info: &ServerInfo,
    events: &EventSender,
) -> Result<CompletionStats, CompletionError> {
    determine_completion(
        tree,
        protocol,
        &TransportOptions::from_protocol(protocol),
        info,
        &TransferCtx::new(),
        events,
    )
    .await
}

/// Content of the remote files, and the zsync map for `names`.
fn remote_content(names: &[&str]) -> (Vec<u8>, HashMap<String, Vec<u8>>) {
    let content = pseudo_random(4 * BLOCK_SIZE, 7);
    let zsync = names
        .iter()
        .map(|n| (format!("/repo/@a/addons/{n}.zsync"), zsync_of(&content, n)))
        .collect();
    (content, zsync)
}

fn half_of(content: &[u8]) -> Vec<u8> {
    let mut half = content[..2 * BLOCK_SIZE].to_vec();
    half.extend(std::iter::repeat_n(0xAB_u8, 2 * BLOCK_SIZE));
    half
}

// ---------------------------------------------------------------------------------------------
// Tests

#[tokio::test]
async fn a9_completion_with_partial_transfer() {
    let (content, zsync) = remote_content(&["full.pbo", "half.pbo", "absent.pbo", "same.pbo"]);
    let server = TestServer::start(Config {
        ranges: true,
        zsync,
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(
        base.path(),
        &[
            remote("full.pbo", true, Some(content.clone())),
            remote("half.pbo", true, Some(half_of(&content))),
            remote("absent.pbo", true, None),
            remote("same.pbo", false, Some(content.clone())),
        ],
    );
    let (sender, rx) = EventSender::channel();
    let stats = run(&mut tree, &server.protocol(), &server_info(1), &sender)
        .await
        .expect("completion");
    assert_eq!(
        stats,
        CompletionStats {
            checked: 2,
            partial: 2
        }
    );
    assert!(complete(&tree, "full.pbo") == 100.0);
    assert!(complete(&tree, "half.pbo") == 50.0);
    assert!(complete(&tree, "absent.pbo") == 0.0);
    assert!(complete(&tree, "same.pbo") == 100.0);

    let seen = server.seen();
    assert_eq!(seen[0], ("/repo/.a3s/sync".to_owned(), true), "probe first");
    let mut zsync = server.zsync_requests();
    zsync.sort();
    assert_eq!(
        zsync,
        [
            "/repo/@a/addons/full.pbo.zsync",
            "/repo/@a/addons/half.pbo.zsync"
        ]
    );
    assert_eq!(seen.len(), 3);

    let events: Vec<SyncEvent> = rx.try_iter().collect();
    assert_eq!(
        events,
        [
            SyncEvent::Phase(Phase::DeterminingCompletion),
            SyncEvent::Progress {
                phase: Phase::DeterminingCompletion,
                percent: 50
            },
            SyncEvent::Progress {
                phase: Phase::DeterminingCompletion,
                percent: 100
            },
        ]
    );
}

#[tokio::test]
async fn a9_completion_server_ignores_range() {
    let (content, zsync) = remote_content(&["full.pbo"]);
    let server = TestServer::start(Config {
        ranges: false,
        zsync,
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(
        base.path(),
        &[
            remote("full.pbo", true, Some(content.clone())),
            remote("same.pbo", false, Some(content)),
        ],
    );
    let stats = run(
        &mut tree,
        &server.protocol(),
        &server_info(1),
        &EventSender::noop(),
    )
    .await
    .expect("completion");
    assert_eq!(stats, CompletionStats::default());
    assert!(complete(&tree, "full.pbo") == 0.0);
    assert!(complete(&tree, "same.pbo") == 100.0);
    assert_eq!(server.seen(), [("/repo/.a3s/sync".to_owned(), true)]);
}

#[tokio::test]
async fn a9_completion_no_partial_file_transfer() {
    let (content, zsync) = remote_content(&["full.pbo"]);
    let server = TestServer::start(Config {
        ranges: true,
        zsync,
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(base.path(), &[remote("full.pbo", true, Some(content))]);
    let info = ServerInfo {
        no_partial_file_transfer: true,
        ..server_info(1)
    };
    run(&mut tree, &server.protocol(), &info, &EventSender::noop())
        .await
        .expect("completion");
    assert!(complete(&tree, "full.pbo") == 0.0);
    assert!(server.seen().is_empty(), "{:?}", server.seen());
}

#[tokio::test]
async fn a9_completion_compressed_only() {
    let (content, zsync) = remote_content(&["full.pbo"]);
    let server = TestServer::start(Config {
        ranges: true,
        zsync,
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(base.path(), &[remote("full.pbo", true, Some(content))]);
    let info = ServerInfo {
        compressed_pbo_files_only: true,
        ..server_info(1)
    };
    run(&mut tree, &server.protocol(), &info, &EventSender::noop())
        .await
        .expect("completion");
    assert!(complete(&tree, "full.pbo") == 0.0);
    assert_eq!(server.seen(), [("/repo/.a3s/sync".to_owned(), true)]);
}

#[tokio::test]
async fn a9_completion_zsync_not_found() {
    let content = pseudo_random(4 * BLOCK_SIZE, 7);
    let server = TestServer::start(Config {
        ranges: true,
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(base.path(), &[remote("x.pbo", true, Some(content))]);
    let err = run(
        &mut tree,
        &server.protocol(),
        &server_info(1),
        &EventSender::noop(),
    )
    .await
    .expect_err("404");
    assert!(matches!(err, CompletionError::Remote(_)), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(
        err.to_string(),
        format!(
            "Failed to connect to the HTTP server on url: http://127.0.0.1:{}/repo/@a/addons/x.pbo.zsync\nRemote file not found: /@a/addons/x.pbo.zsync",
            server.port
        )
    );
}

#[tokio::test]
async fn a9_completion_garbage_zsync() {
    let content = pseudo_random(4 * BLOCK_SIZE, 7);
    let server = TestServer::start(Config {
        ranges: true,
        zsync: [(
            "/repo/@a/addons/x.pbo.zsync".to_owned(),
            b"not a zsync file".to_vec(),
        )]
        .into(),
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(base.path(), &[remote("x.pbo", true, Some(content))]);
    let err = run(
        &mut tree,
        &server.protocol(),
        &server_info(1),
        &EventSender::noop(),
    )
    .await
    .expect_err("garbage");
    let CompletionError::Zsync { relative_path, .. } = &err else {
        panic!("{err:?}")
    };
    assert_eq!(relative_path, "@a/addons/x.pbo.zsync");
    assert!(
        err.to_string()
            .starts_with("Failed to read zsync file: /@a/addons/x.pbo.zsync\n"),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(err.user_message(), err.to_string());
}

#[tokio::test]
async fn a9_completion_concurrency_bound() {
    let names = ["f1", "f2", "f3", "f4", "f5", "f6"];
    let (content, zsync) = remote_content(&names);
    let server = TestServer::start(Config {
        ranges: true,
        zsync,
        delay: Duration::from_millis(100),
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let files: Vec<Remote> = names
        .iter()
        .map(|n| remote(n, true, Some(content.clone())))
        .collect();
    let mut tree = planned(base.path(), &files);
    let stats = run(
        &mut tree,
        &server.protocol(),
        &server_info(2),
        &EventSender::noop(),
    )
    .await
    .expect("completion");
    assert_eq!(stats.checked, 6);
    for name in names {
        assert!(complete(&tree, name) == 100.0, "{name}");
    }
    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let max = server.state.max_in_flight.load(Ordering::SeqCst);
    assert!(max >= 1 && max <= 2.min(cpus), "max in flight {max}");
    if cpus >= 2 {
        assert_eq!(max, 2, "both connections work in parallel");
    }
    assert_eq!(server.zsync_requests().len(), 6);
}

#[tokio::test]
async fn a9_completion_cancel_stalled() {
    let (content, zsync) = remote_content(&["x.pbo"]);
    let server = TestServer::start(Config {
        ranges: true,
        zsync,
        stall: true,
        ..Default::default()
    })
    .await;
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(base.path(), &[remote("x.pbo", true, Some(content))]);
    let protocol = server.protocol();
    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let start = Instant::now();
    let err = determine_completion(
        &mut tree,
        &protocol,
        &TransportOptions::from_protocol(&protocol),
        &server_info(1),
        &ctx,
        &EventSender::noop(),
    )
    .await
    .expect_err("cancelled");
    assert!(matches!(err, CompletionError::Cancelled), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn a9_completion_ftp_without_network() {
    let content = pseudo_random(4 * BLOCK_SIZE, 7);
    let base = tempfile::tempdir().expect("tempdir");
    let mut tree = planned(
        base.path(),
        &[
            remote("x.pbo", true, Some(content.clone())),
            remote("same.pbo", false, Some(content)),
        ],
    );
    // Nothing listens there; any connection attempt would fail.
    let protocol = Protocol::new(
        ProtocolType::Ftp,
        "127.0.0.1/repo",
        "1",
        "anonymous",
        "",
        false,
    );
    let start = Instant::now();
    let stats = run(&mut tree, &protocol, &server_info(4), &EventSender::noop())
        .await
        .expect("no network needed");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(stats, CompletionStats::default());
    assert!(complete(&tree, "x.pbo") == 0.0);
    assert!(complete(&tree, "same.pbo") == 100.0);
}

#[test]
fn a9_completion_future_and_errors_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    fn assert_error<T: std::error::Error + Send + Sync + 'static>() {}
    assert_error::<CompletionError>();
    assert_error::<a3s_sync::PlanError>();

    let mut tree = PlanTree::new();
    let protocol = Protocol::new(
        ProtocolType::Http,
        "127.0.0.1/repo",
        "1",
        "anonymous",
        "",
        false,
    );
    let options = TransportOptions::from_protocol(&protocol);
    let info = ServerInfo::default();
    let ctx = TransferCtx::new();
    let events = EventSender::noop();
    let future = determine_completion(&mut tree, &protocol, &options, &info, &ctx, &events);
    assert_send(&future);
}
