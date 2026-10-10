//! Remote metadata access against local servers serving the WOG golden fixtures.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use a3s_error::ErrorKind;
use a3s_repo_files::{A3sFile, decode_a3s};
use a3s_sync::remote::{
    CheckJob, MAX_CONCURRENT_REPOSITORY_CHECKS, check_error_message, check_repositories,
    check_repository, import_autoconfig,
};
use a3s_sync::{
    EventSender, PartialTransferSupport, Phase, RemoteConnection, RemoteError, RemoteMetadata,
    SyncEvent,
};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::{AutoConfig, Changelogs, Event, Events, Protocol, ProtocolType, ServerInfo};
use a3s_types::{FavoriteServer, SyncTreeDirectory};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use parking_lot::Mutex;

// ---------------------------------------------------------------------------------------------
// Local HTTP server

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_repos/wog_ftp")
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(name)).expect("fixture exists")
}

/// One request seen by the server.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: HeaderMap,
}

/// What the server answers.
#[derive(Clone, Debug)]
enum Reply {
    /// The fixture of `/repo/.a3s/<file>`, honouring `Range: bytes=0-1` with a 206.
    Fixture,
    /// The fixture, ignoring any `Range` header.
    FixtureNoRange,
    Status(u16),
    Body(Vec<u8>),
    Stall,
}

type Rule = Arc<dyn Fn(&str) -> Reply + Send + Sync>;

#[derive(Clone)]
struct ServerState {
    rule: Rule,
    delay: Duration,
    seen: Arc<Mutex<Vec<Seen>>>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

struct TestServer {
    port: u16,
    state: ServerState,
}

impl TestServer {
    async fn start(delay: Duration, rule: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        let state = ServerState {
            rule: Arc::new(rule),
            delay,
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

    async fn fixtures() -> Self {
        Self::start(Duration::ZERO, |_| Reply::Fixture).await
    }

    fn protocol(&self, path: &str) -> Protocol {
        Protocol::new(
            ProtocolType::Http,
            format!("127.0.0.1{path}"),
            self.port.to_string(),
            "anonymous",
            "",
            false,
        )
    }

    fn paths(&self) -> Vec<String> {
        self.state
            .seen
            .lock()
            .iter()
            .map(|s| s.path.clone())
            .collect()
    }

    fn seen(&self) -> Vec<Seen> {
        self.state.seen.lock().clone()
    }
}

async fn handle(State(state): State<ServerState>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    state.seen.lock().push(Seen {
        path: path.clone(),
        headers: request.headers().clone(),
    });
    let now = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_in_flight.fetch_max(now, Ordering::SeqCst);
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    let response = respond(&state, &path, request.headers()).await;
    state.in_flight.fetch_sub(1, Ordering::SeqCst);
    response
}

async fn respond(state: &ServerState, path: &str, headers: &HeaderMap) -> Response {
    let status = |code: u16| {
        Response::builder()
            .status(code)
            .body(Body::empty())
            .expect("response")
    };
    let fixture_body = || {
        let name = path.rsplit('/').next().unwrap_or_default();
        let file = fixture_dir().join(name);
        (path.starts_with("/repo/.a3s/") && file.is_file())
            .then(|| std::fs::read(file).expect("fixture readable"))
    };
    match (state.rule)(path) {
        Reply::Fixture => match fixture_body() {
            Some(body) if headers.get(header::RANGE).is_some_and(|r| r == "bytes=0-1") => {
                Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_RANGE, format!("bytes 0-1/{}", body.len()))
                    .body(Body::from(body[..2].to_vec()))
                    .expect("response")
            }
            Some(body) => Response::new(Body::from(body)),
            None => status(404),
        },
        Reply::FixtureNoRange => match fixture_body() {
            Some(body) => Response::new(Body::from(body)),
            None => status(404),
        },
        Reply::Status(code) => status(code),
        Reply::Body(body) => Response::new(Body::from(body)),
        Reply::Stall => {
            tokio::time::sleep(Duration::from_secs(30)).await;
            status(500)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers

const ORDER: [A3sFile; 5] = [
    A3sFile::ServerInfo,
    A3sFile::Sync,
    A3sFile::Changelogs,
    A3sFile::Events,
    A3sFile::AutoConfig,
];

fn decoded<T: a3s_types::JavaCodec>(name: &str) -> T {
    decode_a3s(&fixture(name)).expect("fixture decodes")
}

/// Metadata distinct from the fixtures, to observe which fields a check replaced.
fn previous_metadata() -> RemoteMetadata {
    RemoteMetadata {
        server_info: Some(Arc::new(ServerInfo {
            revision: 1,
            ..Default::default()
        })),
        sync: Some(Arc::new(SyncTreeDirectory::new_root())),
        changelogs: Some(Arc::new(Changelogs::default())),
        events: Some(Arc::new(Events {
            list: vec![Event {
                name: "previous".to_owned(),
                ..Default::default()
            }],
        })),
        auto_config: Some(Arc::new(AutoConfig {
            repository_name: Some("previous".to_owned()),
            protocol: None,
            favorite_servers: vec![FavoriteServer::default()],
        })),
    }
}

async fn check(protocol: &Protocol, metadata: &mut RemoteMetadata) -> Result<(), RemoteError> {
    check_repository(
        "WOG",
        protocol,
        &TransportOptions::from_protocol(protocol),
        metadata,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
}

fn assert_fixture_fields(m: &RemoteMetadata, files: &[A3sFile]) {
    for file in files {
        match file {
            A3sFile::ServerInfo => assert_eq!(
                m.server_info.as_deref(),
                Some(&decoded::<ServerInfo>("serverinfo"))
            ),
            A3sFile::Sync => assert_eq!(
                m.sync.as_deref(),
                Some(&decoded::<SyncTreeDirectory>("sync"))
            ),
            A3sFile::Changelogs => assert_eq!(
                m.changelogs.as_deref(),
                Some(&decoded::<Changelogs>("changelogs"))
            ),
            A3sFile::Events => {
                assert_eq!(m.events.as_deref(), Some(&decoded::<Events>("events")));
            }
            A3sFile::AutoConfig => assert_eq!(
                m.auto_config.as_deref(),
                Some(&decoded::<AutoConfig>("autoconfig"))
            ),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// A4

#[tokio::test]
async fn a4_check_repository_fetches_all_five_files_in_order() {
    let server = TestServer::fixtures().await;
    let protocol = server.protocol("/repo");
    let (events, rx) = EventSender::channel();
    let mut metadata = RemoteMetadata::default();
    check_repository(
        "WOG",
        &protocol,
        &TransportOptions::from_protocol(&protocol),
        &mut metadata,
        &TransferCtx::new(),
        &events,
    )
    .await
    .expect("check succeeds");
    drop(events);

    assert_fixture_fields(&metadata, &ORDER);
    assert!(metadata.require_for_sync(Some("e")).is_ok());

    let events: Vec<_> = rx.iter().collect();
    let mut expected = vec![SyncEvent::Phase(Phase::CheckingRepository)];
    expected.extend(ORDER.iter().map(|&file| SyncEvent::MetadataFetched {
        file,
        bytes: fixture(file.file_name()).len() as u64,
    }));
    assert_eq!(events, expected);

    let expected_paths: Vec<_> = ORDER
        .iter()
        .map(|f| format!("/repo/.a3s/{}", f.file_name()))
        .collect();
    assert_eq!(server.paths(), expected_paths);
    for seen in server.seen() {
        assert_eq!(
            seen.headers.get(header::USER_AGENT).map(|v| v.as_bytes()),
            Some(&b"ArmA3Sync"[..])
        );
        assert!(seen.headers.get(header::AUTHORIZATION).is_none());
        assert!(seen.headers.get(header::RANGE).is_none());
    }
}

// ---------------------------------------------------------------------------------------------
// A5

#[tokio::test]
async fn a5_a_not_found_keeps_later_fields_stale() {
    let server = TestServer::start(Duration::ZERO, |path| {
        if path.ends_with("/events") {
            Reply::Status(404)
        } else {
            Reply::Fixture
        }
    })
    .await;
    let previous = previous_metadata();
    let mut metadata = previous.clone();
    let err = check(&server.protocol("/repo"), &mut metadata)
        .await
        .expect_err("events missing");

    assert_fixture_fields(
        &metadata,
        &[A3sFile::ServerInfo, A3sFile::Sync, A3sFile::Changelogs],
    );
    assert_eq!(metadata.events, None);
    assert_eq!(metadata.auto_config, previous.auto_config);

    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.is_not_found());
    assert_eq!(
        err.to_string(),
        format!(
            "Failed to connect to the HTTP server on url: http://127.0.0.1:{}/repo/.a3s/events\n\
             Remote file not found: /.a3s/events",
            server.port
        )
    );
    assert_eq!(server.paths().len(), 4, "autoconfig is not fetched");
}

#[tokio::test]
async fn a5_b_unauthorized() {
    let server = TestServer::start(Duration::ZERO, |_| Reply::Status(401)).await;
    let mut metadata = previous_metadata();
    let err = check(&server.protocol("/repo"), &mut metadata)
        .await
        .expect_err("401");
    assert_eq!(err.kind(), ErrorKind::Unauthorized);
    assert!(
        err.to_string()
            .ends_with("\nlogin or password wrong or missing"),
        "{err}"
    );
    assert_eq!(metadata.server_info, None);
    assert_eq!(metadata.sync, previous_metadata().sync);
}

#[tokio::test]
async fn a5_c_corrupt_sync() {
    let server = TestServer::start(Duration::ZERO, |path| {
        if path.ends_with("/sync") {
            Reply::Body(b"definitely not gzip".to_vec())
        } else {
            Reply::Fixture
        }
    })
    .await;
    let mut metadata = previous_metadata();
    let err = check(&server.protocol("/repo"), &mut metadata)
        .await
        .expect_err("corrupt");
    assert!(matches!(
        err,
        RemoteError::Decode {
            file: A3sFile::Sync,
            ..
        }
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    let text = err.to_string();
    assert!(text.starts_with("Failed to read file: sync\n"), "{text}");
    assert!(
        text.ends_with("\nThe file seems to be corrupted."),
        "{text}"
    );
    assert_fixture_fields(&metadata, &[A3sFile::ServerInfo]);
    assert_eq!(metadata.sync, None);
    assert_eq!(metadata.changelogs, previous_metadata().changelogs);
}

#[tokio::test]
async fn a5_d_cancel_while_stalled() {
    let server = TestServer::start(Duration::ZERO, |path| {
        if path.ends_with("/sync") {
            Reply::Stall
        } else {
            Reply::Fixture
        }
    })
    .await;
    let protocol = server.protocol("/repo");
    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let previous = previous_metadata();
    let mut metadata = previous.clone();
    let start = Instant::now();
    let result = check_repository(
        "WOG",
        &protocol,
        &TransportOptions::from_protocol(&protocol),
        &mut metadata,
        &ctx,
        &EventSender::noop(),
    )
    .await;
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
    assert!(matches!(result, Err(RemoteError::Cancelled)), "{result:?}");
    assert_fixture_fields(&metadata, &[A3sFile::ServerInfo]);
    assert_eq!(metadata.sync, None);
    assert_eq!(metadata.changelogs, previous.changelogs);
    assert_eq!(metadata.events, previous.events);
    assert_eq!(metadata.auto_config, previous.auto_config);

    // Cancelled before starting: nothing is touched.
    let mut metadata = previous.clone();
    let result = check_repository(
        "WOG",
        &protocol,
        &TransportOptions::from_protocol(&protocol),
        &mut metadata,
        &ctx,
        &EventSender::noop(),
    )
    .await;
    assert!(matches!(result, Err(RemoteError::Cancelled)));
    assert_eq!(metadata, previous);
}

#[tokio::test]
async fn a5_e_basic_auth() {
    let server = TestServer::fixtures().await;
    let mut protocol = server.protocol("/repo");
    protocol.login = Some("user".to_owned());
    protocol.password = Some("pass".to_owned());
    let mut metadata = RemoteMetadata::default();
    check(&protocol, &mut metadata).await.expect("check");
    let seen = server.seen();
    assert_eq!(seen.len(), 5);
    for s in seen {
        assert_eq!(
            s.headers.get(header::AUTHORIZATION).map(|v| v.as_bytes()),
            Some(&b"Basic dXNlcjpwYXNz"[..])
        );
    }
}

// ---------------------------------------------------------------------------------------------
// A6

async fn probe(server: &TestServer) -> Result<PartialTransferSupport, RemoteError> {
    let protocol = server.protocol("/repo");
    let ctx = TransferCtx::new();
    let mut connection =
        RemoteConnection::connect(&protocol, &TransportOptions::from_protocol(&protocol), &ctx)
            .await
            .expect("connect");
    let result = connection.probe_partial_transfer(&ctx).await;
    connection.close().await;
    result
}

#[tokio::test]
async fn a6_probe_partial_transfer() {
    let server = TestServer::fixtures().await;
    assert_eq!(
        probe(&server).await.expect("probe"),
        PartialTransferSupport::Supported
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/repo/.a3s/sync");
    let ranges: Vec<_> = seen[0].headers.get_all(header::RANGE).iter().collect();
    assert_eq!(ranges, ["bytes=0-1"]);

    let server = TestServer::start(Duration::ZERO, |_| Reply::FixtureNoRange).await;
    assert_eq!(
        probe(&server).await.expect("probe"),
        PartialTransferSupport::Unsupported
    );

    let server = TestServer::start(Duration::ZERO, |_| Reply::Status(404)).await;
    let err = probe(&server).await.expect_err("404");
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

/// A minimal FTP control connection: enough for login, `TYPE I`, `CWD`/`PWD` and `QUIT`.
async fn fake_ftp_server() -> u16 {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 ready\r\n").await.ok();
                while let Ok(Some(line)) = lines.next_line().await {
                    let command = line
                        .split(' ')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    let reply: &[u8] = match command.as_str() {
                        "USER" => b"331 password please\r\n",
                        "PASS" => b"230 logged in\r\n",
                        "TYPE" => b"200 type set\r\n",
                        "CWD" => b"250 directory changed\r\n",
                        "PWD" => b"257 \"/\" is the current directory\r\n",
                        "QUIT" => b"221 bye\r\n",
                        _ => b"502 not implemented\r\n",
                    };
                    write.write_all(reply).await.ok();
                    if command == "QUIT" {
                        break;
                    }
                }
            });
        }
    });
    port
}

#[tokio::test]
async fn a6_probe_is_not_applicable_on_ftp() {
    let port = fake_ftp_server().await;
    let protocol = Protocol::new(
        ProtocolType::Ftp,
        "127.0.0.1/repo",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let ctx = TransferCtx::new();
    let mut connection =
        RemoteConnection::connect(&protocol, &TransportOptions::from_protocol(&protocol), &ctx)
            .await
            .expect("connect");
    assert_eq!(connection.protocol(), &protocol);
    assert_eq!(connection.transport_mut().protocol_name(), "FTP");
    assert_eq!(
        connection
            .probe_partial_transfer(&ctx)
            .await
            .expect("probe"),
        PartialTransferSupport::NotApplicable
    );
    connection.close().await;
}

#[tokio::test]
async fn a6_connect_errors() {
    // Unsupported protocol type.
    let mut protocol = Protocol::new(
        ProtocolType::Socks5,
        "127.0.0.1",
        "1080",
        "anonymous",
        "",
        false,
    );
    let ctx = TransferCtx::new();
    let err =
        RemoteConnection::connect(&protocol, &TransportOptions::from_protocol(&protocol), &ctx)
            .await
            .err()
            .expect("invalid protocol");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(matches!(err, RemoteError::Connect { .. }));

    // Cancelled before connecting.
    protocol.protocol_type = ProtocolType::Http;
    ctx.cancel.cancel();
    let err =
        RemoteConnection::connect(&protocol, &TransportOptions::from_protocol(&protocol), &ctx)
            .await
            .err()
            .expect("cancelled");
    assert!(matches!(err, RemoteError::Cancelled));
}

// ---------------------------------------------------------------------------------------------
// A7

#[tokio::test]
async fn a7_import_autoconfig() {
    let server = TestServer::fixtures().await;
    let protocol = server.protocol("/repo/.a3s");
    let ctx = TransferCtx::new();
    let options = TransportOptions::from_protocol(&protocol);
    let auto_config = import_autoconfig(&protocol, &options, &ctx)
        .await
        .expect("import");
    assert_eq!(auto_config, decoded::<AutoConfig>("autoconfig"));
    assert_eq!(server.paths(), ["/repo/.a3s/autoconfig"]);

    let missing = server.protocol("/missing/.a3s");
    let err = import_autoconfig(&missing, &options, &ctx)
        .await
        .expect_err("404");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(
        err.to_string(),
        format!(
            "Failed to connect to the HTTP server on url: http://127.0.0.1:{}/missing/.a3s/autoconfig\n\
             Remote file not found: /autoconfig",
            server.port
        )
    );
}

// ---------------------------------------------------------------------------------------------
// A8

#[tokio::test]
async fn a8_check_repositories_is_bounded_and_ordered() {
    let server = TestServer::start(Duration::from_millis(100), |path| {
        if path.starts_with("/missing/") {
            Reply::Status(404)
        } else {
            Reply::Fixture
        }
    })
    .await;
    let mut metadata: Vec<RemoteMetadata> = vec![RemoteMetadata::default(); 6];
    let jobs: Vec<CheckJob<'_>> = metadata
        .iter_mut()
        .enumerate()
        .map(|(i, m)| {
            let protocol = server.protocol(if i == 2 { "/missing" } else { "/repo" });
            CheckJob {
                repository_name: format!("job{i}"),
                options: TransportOptions::from_protocol(&protocol),
                protocol,
                metadata: m,
            }
        })
        .collect();
    let ctx = TransferCtx::new();
    let events = EventSender::noop();
    let future = check_repositories(jobs, &ctx, &events);
    fn assert_send<T: Send>(_: &T) {}
    assert_send(&future);
    let results = future.await;

    let names: Vec<_> = results.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["job0", "job1", "job2", "job3", "job4", "job5"]);
    for (i, (name, result)) in results.iter().enumerate() {
        if i == 2 {
            let err = result.as_ref().expect_err("missing repository");
            assert_eq!(
                check_error_message(name, err),
                format!(
                    "Error when checking repository job2:\n\
                     Failed to connect to the HTTP server on url: http://127.0.0.1:{}/missing/.a3s/serverinfo\n\
                     Remote file not found: /.a3s/serverinfo",
                    server.port
                )
            );
        } else {
            assert!(result.is_ok(), "{name}: {result:?}");
        }
    }
    let max = server.state.max_in_flight.load(Ordering::SeqCst);
    assert!(
        max <= MAX_CONCURRENT_REPOSITORY_CHECKS,
        "max in flight {max}"
    );
    assert!(max >= 2, "checks ran concurrently ({max})");
    for (i, m) in metadata.iter().enumerate() {
        assert_eq!(m.server_info.is_some(), i != 2, "job{i}");
    }
}
