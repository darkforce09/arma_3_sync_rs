//! Adversarial verification of the remote model: status table edge cases, favorite-server merge quirks,
//! plan-tree invariants on the WOG fixture, event helpers, and remote failures on HTTP and a
//! scripted FTP server.

#![allow(clippy::result_large_err)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use a3s_foundation::error::ErrorKind;
use a3s_repo_files::{A3sFile, decode_a3s};
use a3s_sync::events::{PercentCounter, RateLimiter};
use a3s_sync::remote::{CheckJob, check_repositories, check_repository, import_autoconfig};
use a3s_sync::status::{determine_sync_status, merge_favorite_servers, update_repository};
use a3s_sync::{
    EventSender, PlanNode, PlanTree, RemoteError, RemoteMetadata, SyncEvent, SyncOptions,
    TransferProgress,
};
use a3s_transport::TransferCtx;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Configuration, Repository};
use a3s_types::{
    AutoConfig, Changelog, Changelogs, Events, FavoriteServer, Protocol, ProtocolType, ServerInfo,
    SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode,
};
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};

// ---------------------------------------------------------------------------------------------
// Helpers

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_repos/wog_ftp")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(name)).expect("fixture exists")
}

fn decoded<T: a3s_types::JavaCodec>(name: &str) -> T {
    decode_a3s(&fixture(name)).expect("fixture decodes")
}

fn server(revision: i32, content_updated: bool) -> ServerInfo {
    ServerInfo {
        revision,
        repository_content_updated: content_updated,
        ..Default::default()
    }
}

fn changelogs(entries: &[(i32, bool)]) -> Changelogs {
    Changelogs {
        list: entries
            .iter()
            .map(|&(revision, content_updated)| Changelog {
                revision,
                content_updated,
                ..Default::default()
            })
            .collect(),
    }
}

fn repo() -> Repository {
    Repository::new(
        "WOG",
        Protocol::new(ProtocolType::Ftp, "host/path", "21", "anonymous", "", false),
    )
}

fn fav(name: Option<&str>, ip: Option<&str>, port: i32) -> FavoriteServer {
    FavoriteServer {
        name: name.map(str::to_owned),
        ip_address: ip.map(str::to_owned),
        port,
        ..Default::default()
    }
}

fn previous_metadata() -> RemoteMetadata {
    RemoteMetadata {
        server_info: Some(Arc::new(server(1, false))),
        sync: Some(Arc::new(SyncTreeDirectory::new_root())),
        changelogs: Some(Arc::new(Changelogs::default())),
        events: Some(Arc::new(Events::default())),
        auto_config: Some(Arc::new(AutoConfig {
            repository_name: Some("previous".to_owned()),
            protocol: None,
            favorite_servers: vec![],
        })),
    }
}

fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}

// ---------------------------------------------------------------------------------------------
// determine_sync_status edge cases (RepositoryService.java:1475-1521)

#[test]
fn verify_status_equal_revisions_ignore_content_updated() {
    // Equal revision is Ok even if the server says content changed.
    assert_eq!(
        determine_sync_status(10, Some(&server(10, true)), None),
        RepositoryStatus::Ok
    );
}

#[test]
fn verify_status_regression_is_indeterminated_even_with_content_updated() {
    assert_eq!(
        determine_sync_status(
            12,
            Some(&server(10, true)),
            Some(&changelogs(&[(10, true), (12, true)]))
        ),
        RepositoryStatus::Indeterminated
    );
}

#[test]
fn verify_status_duplicate_false_then_true_last_wins() {
    assert_eq!(
        determine_sync_status(
            8,
            Some(&server(10, false)),
            Some(&changelogs(&[
                (8, false),
                (9, false),
                (9, true),
                (10, false)
            ]))
        ),
        RepositoryStatus::Updated
    );
    // Duplicate of the local revision itself: still "contained".
    assert_eq!(
        determine_sync_status(
            8,
            Some(&server(10, false)),
            Some(&changelogs(&[(8, true), (8, false), (10, false)]))
        ),
        RepositoryStatus::Ok
    );
}

#[test]
fn verify_status_unsorted_changelogs_and_entries_beyond_server_revision() {
    // TreeMap sorts; entries beyond the server revision are still looked at.
    assert_eq!(
        determine_sync_status(
            8,
            Some(&server(10, false)),
            Some(&changelogs(&[
                (12, true),
                (10, false),
                (8, false),
                (9, false)
            ]))
        ),
        RepositoryStatus::Updated
    );
    // Revisions missing between local and server are simply absent: all present ones false.
    assert_eq!(
        determine_sync_status(
            2,
            Some(&server(10, false)),
            Some(&changelogs(&[(2, false), (7, false)]))
        ),
        RepositoryStatus::Ok
    );
}

#[test]
fn verify_status_empty_changelogs_and_extremes() {
    assert_eq!(
        determine_sync_status(1, Some(&server(2, false)), Some(&Changelogs::default())),
        RepositoryStatus::Indeterminated
    );
    assert_eq!(
        determine_sync_status(
            i32::MAX - 1,
            Some(&server(i32::MAX, false)),
            Some(&changelogs(&[(i32::MAX - 1, false), (i32::MAX, true)]))
        ),
        RepositoryStatus::Updated
    );
    assert_eq!(
        determine_sync_status(
            i32::MIN,
            Some(&server(0, false)),
            Some(&changelogs(&[(i32::MIN, false), (0, false)]))
        ),
        RepositoryStatus::Ok
    );
}

// ---------------------------------------------------------------------------------------------
// update_repository / merge_favorite_servers (RepositoryService.java:256-345)

#[test]
fn verify_hidden_union_never_removes_and_reports_change_only_on_growth() {
    let mut r = repo();
    r.hidden_folder_paths = BTreeSet::from(["@a".to_owned(), "@local".to_owned()]);
    r.revision = 5;
    r.number_of_client_connections = 3;
    let mut info = server(5, false);
    info.hidden_folder_paths = BTreeSet::from(["@a".to_owned()]);
    let metadata = RemoteMetadata {
        server_info: Some(Arc::new(info)),
        ..Default::default()
    };
    let mut c = Configuration::default();
    let update = update_repository(&mut r, &mut c, &metadata);
    assert!(!update.repository_changed, "subset adds nothing");
    assert_eq!(
        r.hidden_folder_paths,
        BTreeSet::from(["@a".to_owned(), "@local".to_owned()])
    );
    assert_eq!(update.status, RepositoryStatus::Ok);
}

#[test]
fn verify_client_connections_only_replaced_when_zero() {
    let mut info = server(5, false);
    info.number_of_connections = 6;
    let metadata = RemoteMetadata {
        server_info: Some(Arc::new(info)),
        ..Default::default()
    };
    let mut c = Configuration::default();
    let mut r = repo();
    r.number_of_client_connections = -1;
    update_repository(&mut r, &mut c, &metadata);
    assert_eq!(r.number_of_client_connections, -1, "only 0 is replaced");
}

/// Java's `Configuration.getFavoriteServers()` sorts the list on every read, so the snapshot
/// the merge searches is sorted by name: the first ip+port match is the first in name order.
#[test]
fn verify_merge_matches_first_server_in_sorted_order() {
    // Deliberately unsorted in memory.
    let mut c = Configuration {
        favorite_servers: vec![
            fav(Some("Zulu"), Some("1.1.1.1"), 2302),
            fav(Some("Alpha"), Some("1.1.1.1"), 2302),
        ],
        ..Default::default()
    };
    let mut remote = fav(Some("x"), Some("1.1.1.1"), 2302);
    remote.password = Some("secret".to_owned());
    let metadata = RemoteMetadata {
        auto_config: Some(Arc::new(AutoConfig {
            repository_name: None,
            protocol: None,
            favorite_servers: vec![remote],
        })),
        ..Default::default()
    };
    let mut r = repo();
    update_repository(&mut r, &mut c, &metadata);
    let alpha = c
        .favorite_servers
        .iter()
        .find(|s| s.name.as_deref() == Some("Alpha"))
        .expect("alpha");
    let zulu = c
        .favorite_servers
        .iter()
        .find(|s| s.name.as_deref() == Some("Zulu"))
        .expect("zulu");
    assert_eq!(
        alpha.password.as_deref(),
        Some("secret"),
        "Java updates the first server in sorted order"
    );
    assert_eq!(zulu.password, None);
}

#[test]
fn verify_merge_same_ip_other_port_is_appended_and_empty_remote_is_no_change() {
    let mut local = vec![fav(Some("A"), Some("1.1.1.1"), 1)];
    assert!(!merge_favorite_servers(&mut local, &[], "WOG"));
    let mut remote = fav(None, Some("1.1.1.1"), 2);
    remote.selected = true;
    remote.repository_name = Some("other".to_owned());
    assert!(merge_favorite_servers(&mut local, &[remote], "WOG"));
    assert_eq!(local.len(), 2);
    assert_eq!(local[1].name.as_deref(), Some("WOG"));
    assert!(!local[1].selected);
    assert_eq!(local[1].repository_name.as_deref(), Some("WOG"));
    // Null local ip never matches a non-null remote ip (and vice versa).
    let mut local = vec![fav(Some("A"), None, 1)];
    assert!(merge_favorite_servers(
        &mut local,
        &[fav(Some("B"), Some("1.1.1.1"), 1)],
        "WOG"
    ));
    assert_eq!(local.len(), 2);
}

#[test]
fn verify_merge_ip_comparison_is_case_and_whitespace_exact() {
    // Java uses String.equals.
    let mut local = vec![fav(Some("A"), Some("host.example"), 1)];
    assert!(merge_favorite_servers(
        &mut local,
        &[fav(Some("B"), Some("HOST.example"), 1)],
        "WOG"
    ));
    assert_eq!(local.len(), 2);
}

// ---------------------------------------------------------------------------------------------
// Plan tree

fn sync_leaf(name: &str, size: i64) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: Some(if size == 0 {
            "0".to_owned()
        } else {
            "ab".to_owned()
        }),
        size,
        ..Default::default()
    })
}

fn sync_dir(name: &str, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    SyncTreeNode::Directory(SyncTreeDirectory {
        name: name.to_owned(),
        children,
        ..Default::default()
    })
}

#[test]
fn verify_wog_plan_counts_sizes_flags() {
    let sync: SyncTreeDirectory = decoded("sync");
    let tree = PlanTree::from_sync_tree(&sync);

    let mut sync_dirs = 0usize;
    let mut sync_hidden = 0usize;
    let mut sync_addons = 0usize;
    let mut sync_size: u64 = 0;
    let mut sync_changed = false;
    sync.walk(&mut |_, node| match node {
        SyncTreeNode::Directory(d) => {
            sync_dirs += 1;
            sync_hidden += usize::from(d.hidden);
            sync_addons += usize::from(d.mark_as_addon);
        }
        SyncTreeNode::Leaf(l) => {
            sync_size += u64::try_from(l.size).unwrap_or(0);
            sync_changed |= l.updated || l.deleted;
        }
    });

    let mut dirs = 0usize;
    let mut hidden = 0usize;
    let mut addons = 0usize;
    let mut size: u64 = 0;
    tree.walk(&mut |_, node| match node {
        PlanNode::Directory(d) => {
            dirs += 1;
            hidden += usize::from(d.hidden);
            addons += usize::from(d.mark_as_addon);
            assert_eq!(d.changed, sync_changed && d.changed, "no spurious changed");
        }
        PlanNode::Leaf(l) => {
            size += l.size;
        }
    });
    assert_eq!(dirs, sync_dirs);
    assert_eq!(hidden, sync_hidden);
    assert_eq!(addons, sync_addons);
    assert_eq!(size, sync_size);
    assert_eq!(tree.root.changed, sync_changed);

    // The server info of the same build agrees with the plan.
    let info: ServerInfo = decoded("serverinfo");
    assert_eq!(tree.leaf_count() as i64, info.number_of_files);
    assert_eq!(size as i64, info.total_files_size);
}

#[test]
fn verify_racine_directly_under_root_and_case_duplicates() {
    let mut sync = SyncTreeDirectory::new_root();
    sync.children = vec![
        sync_leaf("readme", 1),
        sync_dir("racine", vec![sync_leaf("x", 1)]),
        sync_dir("b", vec![sync_leaf("lower", 1)]),
        sync_dir("B", vec![sync_leaf("upper", 1)]),
    ];
    let tree = PlanTree::from_sync_tree(&sync);
    assert_eq!(
        tree.leaf_paths(),
        ["b/lower", "B/upper", "racine/x", "readme"]
    );
    assert_eq!(tree.get("racine/x").map(PlanNode::name), Some("x"));
    assert!(matches!(tree.get("b/lower"), Some(PlanNode::Leaf(_))));
    assert!(matches!(tree.get("B/upper"), Some(PlanNode::Leaf(_))));
    assert!(tree.get("b/upper").is_none());
    assert!(tree.get("B/lower").is_none());
    // Windows separators are not path separators.
    assert!(tree.get("racine\\x").is_none());
}

#[test]
fn verify_unicode_ordering_matches_java_compare_to_ignore_case() {
    let mut sync = SyncTreeDirectory::new_root();
    sync.children = vec![
        sync_leaf("é.pbo", 1),
        sync_leaf("z.pbo", 1),
        sync_leaf("E.pbo", 1),
        sync_leaf("_a.pbo", 1),
    ];
    let tree = PlanTree::from_sync_tree(&sync);
    // Java: '_'(95) < 'e'(101) < 'z'(122) < 'é'(233) after upper/lower folding.
    assert_eq!(tree.leaf_paths(), ["_a.pbo", "E.pbo", "z.pbo", "é.pbo"]);
    assert!(tree.get("é.pbo").is_some());
}

#[test]
fn verify_large_sizes_empty_sha1_and_negative_sizes() {
    let mut sync = SyncTreeDirectory::new_root();
    let three_gib: i64 = 3 * 1024 * 1024 * 1024;
    sync.children = vec![
        sync_leaf("big", three_gib),
        sync_leaf("huge", i64::MAX),
        sync_leaf("empty", 0),
        sync_leaf("neg", i64::MIN),
    ];
    let tree = PlanTree::from_sync_tree(&sync);
    let leaf = |p: &str| match tree.get(p) {
        Some(PlanNode::Leaf(l)) => l.clone(),
        other => panic!("{p}: {other:?}"),
    };
    assert_eq!(leaf("big").size, three_gib as u64);
    assert_eq!(leaf("huge").size, i64::MAX as u64);
    assert_eq!(leaf("empty").sha1.as_deref(), Some("0"));
    assert_eq!(leaf("empty").download_size(), 0);
    assert_eq!(leaf("neg").size, 0);

    // Completion uses the uncompressed size even for compressed files (Java quirk).
    let mut l = leaf("big");
    l.compressed = true;
    l.compressed_size = 10;
    l.complete = 50.0;
    assert_eq!(l.download_size(), (three_gib / 2) as u64);
    l.complete = 0.0;
    assert_eq!(l.download_size(), 10);
}

#[test]
fn verify_changed_ignores_directory_flags_and_deleted_leaf_propagates() {
    let mut sync = SyncTreeDirectory::new_root();
    sync.children = vec![
        SyncTreeNode::Directory(SyncTreeDirectory {
            name: "@dir_updated".to_owned(),
            updated: true,
            deleted: true,
            children: vec![sync_leaf("x", 1)],
            ..Default::default()
        }),
        sync_dir(
            "@del",
            vec![sync_dir(
                "deep",
                vec![SyncTreeNode::Leaf(SyncTreeLeaf {
                    name: "gone".to_owned(),
                    deleted: true,
                    ..Default::default()
                })],
            )],
        ),
    ];
    let tree = PlanTree::from_sync_tree(&sync);
    let changed = |p: &str| match tree.get(p) {
        Some(PlanNode::Directory(d)) => d.changed,
        other => panic!("{p}: {other:?}"),
    };
    assert!(!changed("@dir_updated"));
    assert!(changed("@del"));
    assert!(changed("@del/deep"));
    assert!(tree.root.changed);
    // Directory flags are copied.
    assert!(tree.get("@dir_updated").is_some_and(PlanNode::updated));
    assert!(tree.get("@dir_updated").is_some_and(PlanNode::deleted));
}

#[test]
fn verify_set_selected_on_leaf_only() {
    let mut sync = SyncTreeDirectory::new_root();
    sync.children = vec![sync_dir("@a", vec![sync_leaf("x", 1), sync_leaf("y", 1)])];
    let mut tree = PlanTree::from_sync_tree(&sync);
    assert!(tree.set_selected_recursive("@a/x", true));
    assert!(tree.get("@a/x").is_some_and(PlanNode::selected));
    assert!(!tree.get("@a/y").is_some_and(PlanNode::selected));
    assert!(!tree.get("@a").is_some_and(PlanNode::selected));
}

// ---------------------------------------------------------------------------------------------
// Events / throttling helpers

#[test]
fn verify_percent_counter_boundaries() {
    let mut c = PercentCounter::new(1000);
    let emitted: Vec<u8> = (0..1000).filter_map(|_| c.increment()).collect();
    assert_eq!(emitted, (0..=100).collect::<Vec<u8>>());

    let mut c = PercentCounter::new(1);
    assert_eq!(c.increment(), Some(100));

    let mut c = PercentCounter::new(200);
    let emitted: Vec<u8> = (0..200).filter_map(|_| c.increment()).collect();
    assert_eq!(emitted, (0..=100).collect::<Vec<u8>>());

    // Totals beyond u32/i32 do not overflow.
    let mut c = PercentCounter::new(u64::MAX);
    assert_eq!(c.increment(), Some(0));
    assert_eq!(c.increment(), None);
}

#[test]
fn verify_rate_limiter_boundaries() {
    let base = Instant::now();
    let interval = Duration::from_millis(500);
    let mut r = RateLimiter::new(interval);
    assert!(r.ready(base));
    assert!(!r.ready(base));
    assert!(!r.ready(base + Duration::from_nanos(499_999_999)));
    assert!(r.ready(base + interval), "exactly one interval later");
    // The reference moves to the last `true`, not to the theoretical tick.
    assert!(!r.ready(base + interval + Duration::from_millis(499)));
    assert!(r.ready(base + interval * 2));
    // A clock going backwards never panics and is not ready.
    assert!(!r.ready(base));

    let mut zero = RateLimiter::new(Duration::ZERO);
    assert!(zero.ready(base));
    assert!(zero.ready(base));
}

#[test]
fn verify_transfer_percent_large_values() {
    let p = TransferProgress {
        transferred: u64::MAX / 2,
        total: u64::MAX,
        ..Default::default()
    };
    assert_eq!(p.percent(), 49);
    let p = TransferProgress {
        transferred: 3 * 1024 * 1024 * 1024,
        total: 4 * 1024 * 1024 * 1024,
        ..Default::default()
    };
    assert_eq!(p.percent(), 75);
}

#[test]
fn verify_download_connections_negative_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut r = repo();
    r.default_download_location = Some(dir.path().to_string_lossy().into_owned());
    r.number_of_client_connections = 2;
    let o = SyncOptions::from_repository(&mut r, None).expect("options");
    let mut info = server(1, false);
    info.number_of_connections = -3;
    assert_eq!(o.download_connections(Some(&info)), 1);
    info.number_of_connections = 6;
    assert_eq!(o.download_connections(Some(&info)), 2);

    r.number_of_client_connections = -4;
    r.maximum_client_download_speed = f64::NAN;
    let o = SyncOptions::from_repository(&mut r, None).expect("options");
    assert_eq!(o.client_connections, 0);
    assert_eq!(o.max_download_speed, 0);
    assert_eq!(o.download_connections(Some(&info)), 1);

    r.default_download_location = Some(String::new());
    assert!(SyncOptions::from_repository(&mut r, None).is_err());
}

// ---------------------------------------------------------------------------------------------
// Remote: HTTP failures

/// A raw HTTP server: every request gets `response`, then the socket is closed.
async fn raw_http_server(response: &'static [u8]) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                // Read the request head (best effort).
                let _read: std::io::Result<usize> = stream.read(&mut buf).await;
                stream.write_all(response).await.ok();
                stream.flush().await.ok();
                // Close abruptly.
            });
        }
    });
    port
}

fn http(port: u16, path: &str) -> Protocol {
    Protocol::new(
        ProtocolType::Http,
        format!("127.0.0.1{path}"),
        port.to_string(),
        "anonymous",
        "",
        false,
    )
}

async fn run_check(protocol: &Protocol, metadata: &mut RemoteMetadata) -> Result<(), RemoteError> {
    check_repository(
        "WOG",
        protocol,
        &a3s_transport::proxy::transport_options(protocol, &a3s_types::local::Proxy::default())
            .expect("no proxy"),
        metadata,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
}

#[tokio::test]
async fn verify_http_connection_lost_mid_body() {
    let port = raw_http_server(
        b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\npartial",
    )
    .await;
    let protocol = http(port, "/repo");
    let previous = previous_metadata();
    let mut metadata = previous.clone();
    let err = run_check(&protocol, &mut metadata)
        .await
        .expect_err("connection lost");
    assert_eq!(metadata.server_info, None);
    assert_eq!(metadata.sync, previous.sync);
    assert!(err.is_connection_lost(), "{err:?}");
    assert!(
        matches!(err, RemoteError::Fetch { ref relative_path, .. } if relative_path == ".a3s/serverinfo")
    );
}

#[tokio::test]
async fn verify_http_500_and_refused() {
    let port =
        raw_http_server(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n").await;
    let protocol = http(port, "/repo");
    let mut metadata = previous_metadata();
    let err = run_check(&protocol, &mut metadata).await.expect_err("500");
    assert_eq!(err.kind(), ErrorKind::Io);
    assert!(!err.is_not_found());
    assert_eq!(metadata.server_info, None);

    // Refused connection: Java stores a null server info (getServerInfo's finally block).
    let port = closed_port();
    let protocol = http(port, "/repo");
    let mut metadata = previous_metadata();
    let err = run_check(&protocol, &mut metadata)
        .await
        .expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::ConnectionFailed, "{err:?}");
    assert!(err.is_connection_lost());
    assert_eq!(metadata.server_info, None);
}

// ---------------------------------------------------------------------------------------------
// Remote: scripted FTP server

#[derive(Clone)]
enum FtpReply {
    File(Vec<u8>),
    Missing,
    /// Never answer RETR.
    Stall,
    /// Send part of the file, close the data connection and answer 426.
    Abort(Vec<u8>),
}

type FtpRule = Arc<dyn Fn(&str) -> FtpReply + Send + Sync>;

struct FtpStub {
    port: u16,
    retrs: Arc<Mutex<Vec<String>>>,
    max_sessions: Arc<AtomicUsize>,
}

fn ftp_fixture_rule(path: &str) -> FtpReply {
    let name = path.rsplit('/').next().unwrap_or_default();
    let file = fixture_dir().join(name);
    if path.starts_with("/repo/.a3s/") && file.is_file() {
        FtpReply::File(std::fs::read(file).expect("fixture"))
    } else {
        FtpReply::Missing
    }
}

async fn ftp_stub(
    retr_delay: Duration,
    rule: impl Fn(&str) -> FtpReply + Send + Sync + 'static,
) -> FtpStub {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let rule: FtpRule = Arc::new(rule);
    let retrs: Arc<Mutex<Vec<String>>> = Arc::default();
    let sessions = Arc::new(AtomicUsize::new(0));
    let max_sessions = Arc::new(AtomicUsize::new(0));
    let stub = FtpStub {
        port,
        retrs: retrs.clone(),
        max_sessions: max_sessions.clone(),
    };
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let rule = rule.clone();
            let retrs = retrs.clone();
            let sessions = sessions.clone();
            let max_sessions = max_sessions.clone();
            tokio::spawn(async move {
                let now = sessions.fetch_add(1, Ordering::SeqCst) + 1;
                max_sessions.fetch_max(now, Ordering::SeqCst);
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 ready\r\n").await.ok();
                let mut data: Option<tokio::net::TcpListener> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    let (command, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                    let command = command.to_ascii_uppercase();
                    match command.as_str() {
                        "USER" => write.write_all(b"331 password please\r\n").await.ok(),
                        "PASS" => write.write_all(b"230 logged in\r\n").await.ok(),
                        "TYPE" => write.write_all(b"200 type set\r\n").await.ok(),
                        "CWD" => write.write_all(b"250 ok\r\n").await.ok(),
                        "PWD" => write.write_all(b"257 \"/\"\r\n").await.ok(),
                        "PASV" => {
                            let l = tokio::net::TcpListener::bind("127.0.0.1:0")
                                .await
                                .expect("bind data");
                            let p = l.local_addr().expect("addr").port();
                            data = Some(l);
                            let reply = format!(
                                "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                                p >> 8,
                                p & 0xff
                            );
                            write.write_all(reply.as_bytes()).await.ok()
                        }
                        "RETR" => {
                            retrs.lock().push(arg.to_owned());
                            if !retr_delay.is_zero() {
                                tokio::time::sleep(retr_delay).await;
                            }
                            match (rule)(arg) {
                                FtpReply::Missing => {
                                    data = None;
                                    write.write_all(b"550 No such file\r\n").await.ok()
                                }
                                FtpReply::Stall => {
                                    tokio::time::sleep(Duration::from_secs(30)).await;
                                    None
                                }
                                FtpReply::File(bytes) | FtpReply::Abort(bytes) => {
                                    let abort = matches!((rule)(arg), FtpReply::Abort(_));
                                    write.write_all(b"150 opening\r\n").await.ok();
                                    if let Some(l) = data.take()
                                        && let Ok((mut d, _)) = l.accept().await
                                    {
                                        d.write_all(&bytes).await.ok();
                                        d.shutdown().await.ok();
                                    }
                                    if abort {
                                        write
                                            .write_all(
                                                b"426 Connection closed; transfer aborted.\r\n",
                                            )
                                            .await
                                            .ok()
                                    } else {
                                        write.write_all(b"226 done\r\n").await.ok()
                                    }
                                }
                            }
                        }
                        "QUIT" => {
                            write.write_all(b"221 bye\r\n").await.ok();
                            break;
                        }
                        _ => write.write_all(b"502 not implemented\r\n").await.ok(),
                    };
                }
                sessions.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    stub
}

fn ftp(port: u16, path: &str) -> Protocol {
    Protocol::new(
        ProtocolType::Ftp,
        format!("127.0.0.1{path}"),
        port.to_string(),
        "anonymous",
        "",
        false,
    )
}

#[tokio::test]
async fn verify_ftp_check_repository_fetches_fixtures_in_order() {
    let stub = ftp_stub(Duration::ZERO, ftp_fixture_rule).await;
    let protocol = ftp(stub.port, "/repo");
    let (events, rx) = EventSender::channel();
    let mut metadata = RemoteMetadata::default();
    check_repository(
        "WOG",
        &protocol,
        &a3s_transport::proxy::transport_options(&protocol, &a3s_types::local::Proxy::default())
            .expect("no proxy"),
        &mut metadata,
        &TransferCtx::new(),
        &events,
    )
    .await
    .expect("check");
    drop(events);
    assert_eq!(
        stub.retrs.lock().clone(),
        [
            "/repo/.a3s/serverinfo",
            "/repo/.a3s/sync",
            "/repo/.a3s/changelogs",
            "/repo/.a3s/events",
            "/repo/.a3s/autoconfig"
        ]
    );
    assert_eq!(
        metadata.server_info.as_deref(),
        Some(&decoded::<ServerInfo>("serverinfo"))
    );
    assert_eq!(
        metadata.sync.as_deref(),
        Some(&decoded::<SyncTreeDirectory>("sync"))
    );
    assert_eq!(
        metadata.auto_config.as_deref(),
        Some(&decoded::<AutoConfig>("autoconfig"))
    );
    let fetched: Vec<_> = rx
        .iter()
        .filter_map(|e| match e {
            SyncEvent::MetadataFetched { file, bytes } => Some((file, bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(fetched.len(), 5);
    assert_eq!(fetched[1], (A3sFile::Sync, fixture("sync").len() as u64));
}

#[tokio::test]
async fn verify_ftp_550_uses_connect_core() {
    let stub = ftp_stub(Duration::ZERO, |path| {
        if path.ends_with("/changelogs") {
            FtpReply::Missing
        } else {
            ftp_fixture_rule(path)
        }
    })
    .await;
    let protocol = ftp(stub.port, "/repo");
    let previous = previous_metadata();
    let mut metadata = previous.clone();
    let err = run_check(&protocol, &mut metadata).await.expect_err("550");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(metadata.sync.is_some() && metadata.sync != previous.sync);
    assert_eq!(metadata.changelogs, None);
    assert_eq!(metadata.events, previous.events);
    assert_eq!(metadata.auto_config, previous.auto_config);
}

#[tokio::test]
async fn verify_ftp_transfer_aborted_uses_retrieve_core() {
    let stub = ftp_stub(Duration::ZERO, |path| {
        if path.ends_with("/sync") {
            FtpReply::Abort(fixture("sync")[..10].to_vec())
        } else {
            ftp_fixture_rule(path)
        }
    })
    .await;
    let protocol = ftp(stub.port, "/repo");
    let previous = previous_metadata();
    let mut metadata = previous.clone();
    let err = run_check(&protocol, &mut metadata).await.expect_err("426");
    assert!(matches!(err, RemoteError::Fetch { .. }), "{err:?}");
    assert_eq!(metadata.sync, None);
    assert_eq!(metadata.changelogs, previous.changelogs);
}

/// Java connects inside `getServerInfo`, so an unreachable server stores a null server info
/// (`ConnectionService.java:100-106`, `AbstractConnexionDAO.java:155`). Without that, the
/// following `update_repository` computes the status from stale metadata.
#[tokio::test]
async fn verify_ftp_connect_failure_clears_server_info_like_java() {
    let port = closed_port();
    let protocol = ftp(port, "/repo");
    let mut metadata = previous_metadata();
    let err = run_check(&protocol, &mut metadata)
        .await
        .expect_err("refused");
    assert!(matches!(err, RemoteError::Connect { .. }), "{err:?}");
    assert_eq!(
        metadata.server_info, None,
        "Java: serverInfo is null after a failed connection"
    );
}

#[tokio::test]
async fn verify_ftp_cancel_while_retr_stalls() {
    let stub = ftp_stub(Duration::ZERO, |path| {
        if path.ends_with("/sync") {
            FtpReply::Stall
        } else {
            ftp_fixture_rule(path)
        }
    })
    .await;
    let protocol = ftp(stub.port, "/repo");
    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let previous = previous_metadata();
    let mut metadata = previous.clone();
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        check_repository(
            "WOG",
            &protocol,
            &a3s_transport::proxy::transport_options(
                &protocol,
                &a3s_types::local::Proxy::default(),
            )
            .expect("no proxy"),
            &mut metadata,
            &ctx,
            &EventSender::noop(),
        ),
    )
    .await;
    let elapsed = start.elapsed();
    let result = result.expect("check returned within 10 s");
    assert!(matches!(result, Err(RemoteError::Cancelled)), "{result:?}");
    assert!(elapsed < Duration::from_secs(1), "cancel took {elapsed:?}");
    assert_eq!(metadata.sync, None);
    assert_eq!(metadata.changelogs, previous.changelogs);
}

#[tokio::test]
async fn verify_ftp_import_autoconfig() {
    let stub = ftp_stub(Duration::ZERO, ftp_fixture_rule).await;
    let protocol = ftp(stub.port, "/repo/.a3s");
    let ctx = TransferCtx::new();
    let options =
        a3s_transport::proxy::transport_options(&protocol, &a3s_types::local::Proxy::default())
            .expect("no proxy");
    let auto_config = import_autoconfig(&protocol, &options, &ctx)
        .await
        .expect("import");
    assert_eq!(auto_config, decoded::<AutoConfig>("autoconfig"));
    assert_eq!(stub.retrs.lock().clone(), ["/repo/.a3s/autoconfig"]);

    let missing = ftp(stub.port, "/nope/.a3s");
    let err = import_autoconfig(&missing, &options, &ctx)
        .await
        .expect_err("550");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn verify_check_repositories_ftp_bound_order_and_empty() {
    let ctx = TransferCtx::new();
    let events = EventSender::noop();
    assert!(
        check_repositories(Vec::new(), &ctx, &events)
            .await
            .is_empty()
    );

    let stub = ftp_stub(Duration::from_millis(50), ftp_fixture_rule).await;
    let mut metadata = vec![RemoteMetadata::default(); 9];
    let jobs: Vec<CheckJob<'_>> = metadata
        .iter_mut()
        .enumerate()
        .map(|(i, m)| {
            let protocol = ftp(stub.port, if i == 4 { "/missing" } else { "/repo" });
            CheckJob {
                repository_name: format!("r{i}"),
                options: a3s_transport::proxy::transport_options(
                    &protocol,
                    &a3s_types::local::Proxy::default(),
                )
                .expect("no proxy"),
                protocol,
                metadata: m,
            }
        })
        .collect();
    let results = check_repositories(jobs, &ctx, &events).await;
    let names: Vec<_> = results.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(names, (0..9).map(|i| format!("r{i}")).collect::<Vec<_>>());
    for (i, (_, r)) in results.iter().enumerate() {
        assert_eq!(r.is_ok(), i != 4, "r{i}: {r:?}");
    }
    let max = stub.max_sessions.load(Ordering::SeqCst);
    assert!(max <= 4, "max concurrent FTP sessions {max}");
    assert!(max >= 2, "ran concurrently ({max})");
    assert!(metadata[4].server_info.is_none());
    assert!(metadata[8].auto_config.is_some());
}

#[tokio::test]
async fn verify_check_repositories_cancel_stops_all_quickly() {
    let stub = ftp_stub(Duration::from_millis(100), ftp_fixture_rule).await;
    let mut metadata = vec![RemoteMetadata::default(); 8];
    let jobs: Vec<CheckJob<'_>> = metadata
        .iter_mut()
        .enumerate()
        .map(|(i, m)| {
            let protocol = ftp(stub.port, "/repo");
            CheckJob {
                repository_name: format!("r{i}"),
                options: a3s_transport::proxy::transport_options(
                    &protocol,
                    &a3s_types::local::Proxy::default(),
                )
                .expect("no proxy"),
                protocol,
                metadata: m,
            }
        })
        .collect();
    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        cancel.cancel();
    });
    let start = Instant::now();
    let results = check_repositories(jobs, &ctx, &EventSender::noop()).await;
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(results.len(), 8);
    assert!(
        results
            .iter()
            .any(|(_, r)| matches!(r, Err(RemoteError::Cancelled)))
    );
    // Jobs that never started are cancelled before connecting and left untouched.
    assert_eq!(metadata[7], RemoteMetadata::default());
}

#[test]
fn verify_public_types_are_send_sync() {
    fn bounds<T: Send + Sync + 'static>() {}
    fn send<T: Send>() {}
    bounds::<RemoteError>();
    bounds::<RemoteMetadata>();
    bounds::<PlanTree>();
    send::<a3s_sync::RemoteConnection>();
}
