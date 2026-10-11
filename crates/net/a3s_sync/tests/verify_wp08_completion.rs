//! WP08 verification: zsync completion against real `.zsync` files and a local HTTP server
//! (oracle: `a3s_zsync::match_blocks` + `completion_percent`), LIFO order, first-error stop,
//! empty local files, cancellation while matching a > 2 GiB local file.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_sync::completion::{completion_percent, determine_completion, local_completion};
use a3s_sync::plan::{NoAddons, assign_destinations};
use a3s_sync::{CompletionError, EventSender, Phase, PlanNode, PlanTree, SyncEvent};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::{
    Protocol, ProtocolType, ServerInfo, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode,
};
use a3s_zsync::{BLOCK_SIZE, BlockMatches, ControlFile};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use parking_lot::Mutex;

// ---------------------------------------------------------------------------------------------
// Server

#[derive(Clone)]
struct ServerState {
    zsync: Arc<HashMap<String, Vec<u8>>>,
    seen: Arc<Mutex<Vec<String>>>,
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn handle(State(state): State<ServerState>, request: Request) -> Response {
    let path = percent_decode(request.uri().path());
    state.seen.lock().push(path.clone());
    if path == "/repo/.a3s/sync" {
        return Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, "bytes 0-1/10")
            .body(Body::from(b"01".to_vec()))
            .expect("response");
    }
    match state.zsync.get(&path) {
        Some(body) => Response::new(Body::from(body.clone())),
        None => Response::builder()
            .status(404)
            .body(Body::empty())
            .expect("response"),
    }
}

async fn start(zsync: HashMap<String, Vec<u8>>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let state = ServerState {
        zsync: Arc::new(zsync),
        seen: Arc::default(),
    };
    let seen = Arc::clone(&state.seen);
    let router = axum::Router::new().fallback(handle).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });
    (port, seen)
}

fn protocol(port: u16) -> Protocol {
    Protocol::new(
        ProtocolType::Http,
        "127.0.0.1/repo",
        port.to_string(),
        "anonymous",
        "",
        false,
    )
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

fn control_of(content: &[u8], name: &str) -> ControlFile {
    ControlFile::generate(
        Cursor::new(content),
        content.len() as u64,
        name,
        "Sat, 10 Oct 2026 12:00:00 GMT",
        &format!("http://127.0.0.1/repo/@a/{name}"),
        "sha",
    )
    .expect("generate")
}

/// Oracle: B41 over `match_blocks`.
fn oracle(control: &ControlFile, local: &[u8]) -> f64 {
    if local.is_empty() {
        return 0.0;
    }
    let m = a3s_zsync::match_blocks(control, Cursor::new(local), None).expect("match");
    completion_percent(&m, control.header.hash_lengths.seq_matches)
}

/// Different partial local versions of `content`.
fn variants(content: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let bs = BLOCK_SIZE;
    let mut shifted = b"INSERTED-PREFIX-17".to_vec();
    shifted.extend_from_slice(content);
    let mut scattered = content.to_vec();
    for i in (0..scattered.len()).step_by(5 * bs) {
        scattered[i] ^= 0xFF;
    }
    let mut longer = content.to_vec();
    longer.extend(pseudo_random(10 * bs, 99));
    let mut middle_hole = content.to_vec();
    let mid = content.len() / 2;
    for b in &mut middle_hole[mid..mid + 3 * bs] {
        *b = 0x5A;
    }
    vec![
        ("identical", content.to_vec()),
        ("truncated", content[..content.len() / 3].to_vec()),
        ("shifted", shifted),
        ("scattered", scattered),
        ("longer", longer),
        ("middle_hole", middle_hole),
        ("unrelated", pseudo_random(content.len(), 4242)),
        ("one_byte", vec![1]),
    ]
}

fn tree_with(base: &Path, names: &[String]) -> PlanTree {
    let mut sync = SyncTreeDirectory::new_root();
    let mut a = SyncTreeDirectory::new("@a");
    a.mark_as_addon = true;
    for name in names {
        a.add_child(SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.clone(),
            sha1: Some("remote".to_owned()),
            size: 1,
            updated: true,
            ..Default::default()
        }));
    }
    sync.add_child(SyncTreeNode::Directory(a));
    let mut tree = PlanTree::from_sync_tree(&sync);
    assign_destinations(&mut tree, base, false, &NoAddons).expect("safe names");
    tree
}

fn complete(tree: &PlanTree, name: &str) -> f64 {
    match tree.get(&format!("@a/{name}")) {
        Some(PlanNode::Leaf(l)) => l.complete,
        other => panic!("{name}: {other:?}"),
    }
}

fn info(connections: i32) -> ServerInfo {
    ServerInfo {
        number_of_connections: connections,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Tests

#[test]
fn verify_local_completion_matches_oracle() {
    // 37.5 blocks: the last block is short.
    let content = pseudo_random(37 * BLOCK_SIZE + BLOCK_SIZE / 2, 3);
    let control = control_of(&content, "x.pbo");
    let dir = tempfile::tempdir().expect("tempdir");
    for (label, local) in variants(&content) {
        let path = dir.path().join(label);
        std::fs::write(&path, &local).expect("write");
        let got = local_completion(&control, &path, None).expect("completion");
        let want = oracle(&control, &local);
        assert!(got == want, "{label}: {got} vs {want}");
        assert!((0.0..=100.0).contains(&got), "{label}: {got}");
    }
    let identical = dir.path().join("identical");
    assert!(local_completion(&control, &identical, None).expect("c") == 100.0);
    let empty = dir.path().join("empty");
    std::fs::write(&empty, b"").expect("write");
    assert!(local_completion(&control, &empty, None).expect("c") == 0.0);
    // Missing file: an I/O error, not a panic.
    assert!(local_completion(&control, &dir.path().join("nope"), None).is_err());
}

#[test]
fn verify_completion_percent_tables() {
    let m = |v: &[bool]| BlockMatches {
        local_offsets: v.iter().map(|&s| s.then_some(0)).collect(),
    };
    let (s, n) = (true, false);
    // Java FileMaker.matchControl hand-evaluated.
    assert!(completion_percent(&m(&[s, n, s, s, n, s]), 2) == 2.0 / 6.0 * 100.0);
    assert!(completion_percent(&m(&[s, s]), 2) == 100.0);
    assert!(completion_percent(&m(&[s, n]), 2) == 0.0);
    assert!(completion_percent(&m(&[n, s]), 2) == 0.0);
    assert!(completion_percent(&m(&[s, n, s]), 2) == 0.0);
    assert!(completion_percent(&m(&[s, n, s]), 1) == 2.0 / 3.0 * 100.0);
    assert!(completion_percent(&m(&[s]), 2) == 100.0);
    assert!(completion_percent(&m(&[n]), 2) == 0.0);
}

#[tokio::test]
async fn verify_determine_completion_real_zsync_matches_oracle() {
    let content = pseudo_random(37 * BLOCK_SIZE + 123, 11);
    let base = tempfile::tempdir().expect("tempdir");
    let mut zsync = HashMap::new();
    let mut expected = std::collections::BTreeMap::new();
    let mut names = Vec::new();
    for (label, local) in variants(&content) {
        // Unicode and a space in the file name.
        let name = format!("é {label}.pbo");
        let control = control_of(&content, &name);
        zsync.insert(format!("/repo/@a/{name}.zsync"), control.to_bytes());
        std::fs::create_dir_all(base.path().join("@a")).expect("mkdir");
        std::fs::write(base.path().join("@a").join(&name), &local).expect("write");
        expected.insert(name.clone(), oracle(&control, &local));
        names.push(name);
    }
    // An empty local file: 0.0 without any request.
    let empty = "empty.pbo".to_owned();
    std::fs::write(base.path().join("@a").join(&empty), b"").expect("write");
    names.push(empty.clone());

    let (port, seen) = start(zsync).await;
    let mut tree = tree_with(base.path(), &names);
    let p = protocol(port);
    let (sender, rx) = EventSender::channel();
    let stats = determine_completion(
        &mut tree,
        &p,
        &TransportOptions::from_protocol(&p),
        &info(3),
        &TransferCtx::new(),
        &sender,
    )
    .await
    .expect("completion");
    for (name, want) in &expected {
        let got = complete(&tree, name);
        assert!(got == *want, "{name}: {got} vs {want}");
    }
    assert!(complete(&tree, &empty) == 0.0);
    assert!(
        !seen.lock().iter().any(|p| p.contains("empty.pbo")),
        "no request for an empty local file"
    );
    assert_eq!(stats.checked, names.len());
    assert_eq!(
        stats.partial,
        expected.values().filter(|c| **c > 0.0).count()
    );
    drop(sender);
    let events: Vec<_> = rx.try_iter().collect();
    assert_eq!(events[0], SyncEvent::Phase(Phase::DeterminingCompletion));
    assert_eq!(
        events.last(),
        Some(&SyncEvent::Progress {
            phase: Phase::DeterminingCompletion,
            percent: 100
        })
    );
}

/// One connection: files are checked last-first (Java `Stack`), and the first error stops the
/// worker (no further `.zsync` request).
#[tokio::test]
async fn verify_lifo_order_and_stop_after_first_error() {
    let content = pseudo_random(4 * BLOCK_SIZE, 5);
    let base = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(base.path().join("@a")).expect("mkdir");
    let names: Vec<String> = ["f1", "f2", "f3"].iter().map(|s| (*s).to_owned()).collect();
    let mut zsync = HashMap::new();
    for n in &names {
        std::fs::write(base.path().join("@a").join(n), &content).expect("write");
        zsync.insert(
            format!("/repo/@a/{n}.zsync"),
            control_of(&content, n).to_bytes(),
        );
    }
    let (port, seen) = start(zsync.clone()).await;
    let p = protocol(port);
    let mut tree = tree_with(base.path(), &names);
    determine_completion(
        &mut tree,
        &p,
        &TransportOptions::from_protocol(&p),
        &info(1),
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect("completion");
    let order: Vec<String> = seen
        .lock()
        .iter()
        .filter(|p| p.ends_with(".zsync"))
        .cloned()
        .collect();
    assert_eq!(
        order,
        [
            "/repo/@a/f3.zsync",
            "/repo/@a/f2.zsync",
            "/repo/@a/f1.zsync"
        ]
    );

    // f3 (checked first) is missing: error, nothing else requested.
    zsync.remove("/repo/@a/f3.zsync");
    let (port, seen) = start(zsync).await;
    let p = protocol(port);
    let mut tree = tree_with(base.path(), &names);
    let err = determine_completion(
        &mut tree,
        &p,
        &TransportOptions::from_protocol(&p),
        &info(1),
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect_err("404");
    assert!(matches!(err, CompletionError::Remote(_)), "{err:?}");
    let zsync_requests = seen.lock().iter().filter(|p| p.ends_with(".zsync")).count();
    assert_eq!(zsync_requests, 1);
}

/// Cancel while matching a > 2 GiB (sparse) local file: `Err(Cancelled)` within 1 s.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_cancel_during_matching_large_file() {
    let content = pseudo_random(64 * BLOCK_SIZE, 21);
    let base = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(base.path().join("@a")).expect("mkdir");
    let local = base.path().join("@a/big.pbo");
    let f = std::fs::File::create(&local).expect("create");
    f.set_len(3 * 1024 * 1024 * 1024).expect("sparse 3 GiB");
    drop(f);
    let zsync: HashMap<String, Vec<u8>> = [(
        "/repo/@a/big.pbo.zsync".to_owned(),
        control_of(&content, "big.pbo").to_bytes(),
    )]
    .into();
    let (port, seen) = start(zsync).await;
    let p = protocol(port);
    let mut tree = tree_with(base.path(), &["big.pbo".to_owned()]);
    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    let seen2 = Arc::clone(&seen);
    let cancelled_at = Arc::new(Mutex::new(None::<Instant>));
    let at = Arc::clone(&cancelled_at);
    tokio::spawn(async move {
        // Wait for the `.zsync` request, then let the matcher run a little.
        loop {
            if seen2.lock().iter().any(|p| p.ends_with(".zsync")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        *at.lock() = Some(Instant::now());
        cancel.cancel();
    });
    let result = determine_completion(
        &mut tree,
        &p,
        &TransportOptions::from_protocol(&p),
        &info(1),
        &ctx,
        &EventSender::noop(),
    )
    .await;
    let end = Instant::now();
    let Some(at) = *cancelled_at.lock() else {
        panic!("matching finished before the cancel: {result:?} (file too small for this test)")
    };
    assert!(
        matches!(result, Err(CompletionError::Cancelled)),
        "{result:?}"
    );
    assert!(end - at < Duration::from_secs(1), "{:?}", end - at);
}

/// The local path of an updated leaf is a directory: a zsync/local read error, not a panic.
#[tokio::test]
async fn verify_local_directory_is_a_read_error() {
    let content = pseudo_random(4 * BLOCK_SIZE, 5);
    let base = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(base.path().join("@a/x.pbo/inner")).expect("mkdir");
    let zsync: HashMap<String, Vec<u8>> = [(
        "/repo/@a/x.pbo.zsync".to_owned(),
        control_of(&content, "x.pbo").to_bytes(),
    )]
    .into();
    let (port, _) = start(zsync).await;
    let p = protocol(port);
    let mut tree = tree_with(base.path(), &["x.pbo".to_owned()]);
    let result = determine_completion(
        &mut tree,
        &p,
        &TransportOptions::from_protocol(&p),
        &info(1),
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await;
    match result {
        Err(CompletionError::Zsync { ref message, .. }) => {
            assert!(
                message.starts_with("Failed to read zsync file: /@a/x.pbo.zsync\n"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
}
