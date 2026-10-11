//! In-process HTTP(S) server, ZIP builders and staging helpers for the updater tests.

use std::collections::HashMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s_updater::{ConfigOverrides, StagedEntry, UpdateConfig};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use parking_lot::Mutex;
use url::Url;

/// What the server answers for one path.
#[derive(Clone, Debug)]
pub enum Reply {
    /// A body with `Content-Length`.
    Body { status: u16, body: Vec<u8> },

    /// A 200 body streamed in chunks without `Content-Length`.
    Chunked(Vec<Vec<u8>>),

    /// `302 Found` to `location`.
    Redirect(String),

    /// `count` chunks, one every `interval`, without `Content-Length`.
    Slow {
        chunk: Vec<u8>,
        interval: Duration,
        count: usize,
    },
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::Body {
            status: 200,
            body: body.into(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self::Body {
            status,
            body: Vec::new(),
        }
    }
}

/// One request as received.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: HeaderMap,
}

#[derive(Default)]
struct ServerState {
    routes: Mutex<HashMap<String, Reply>>,
    requests: Mutex<Vec<Recorded>>,
}

/// A local HTTP server; unknown paths answer 404.
pub struct Server {
    base: Url,
    state: Arc<ServerState>,
}

impl Server {
    /// A plain HTTP server.
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        Self::serve(listener, &format!("http://{addr}/"))
    }

    /// An HTTPS server with a new self-signed certificate for `127.0.0.1`, returned as PEM.
    pub async fn start_tls() -> (Self, String) {
        let certified =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("certificate");
        let key =
            rustls::pki_types::PrivateKeyDer::Pkcs8(certified.signing_key.serialize_der().into());
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocols")
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .expect("server config");
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = tcp.local_addr().expect("addr");
        let listener = TlsListener {
            tcp,
            acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        };
        let server = Self::serve(listener, &format!("https://{addr}/"));
        (server, certified.cert.pem())
    }

    fn serve<L>(listener: L, base: &str) -> Self
    where
        L: axum::serve::Listener,
        L::Addr: std::fmt::Debug,
    {
        let state = Arc::new(ServerState::default());
        let app = axum::Router::new()
            .fallback(handle)
            .with_state(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        Self {
            base: Url::parse(base).expect("base url"),
            state,
        }
    }

    pub fn route(&self, path: &str, reply: Reply) {
        self.state.routes.lock().insert(path.to_owned(), reply);
    }

    /// Absolute URL of `path` (`/x`).
    pub fn url(&self, path: &str) -> Url {
        self.base.join(path).expect("join")
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.requests.lock().clone()
    }

    pub fn hits(&self, path: &str) -> usize {
        self.requests().iter().filter(|r| r.path == path).count()
    }
}

/// TCP listener that completes a TLS handshake before handing out a connection.
struct TlsListener {
    tcp: tokio::net::TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
}

impl axum::serve::Listener for TlsListener {
    type Io = tokio_rustls::server::TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let Ok((stream, addr)) = self.tcp.accept().await else {
                continue;
            };
            if let Ok(tls) = self.acceptor.accept(stream).await {
                return (tls, addr);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

async fn handle(State(state): State<Arc<ServerState>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    state.requests.lock().push(Recorded {
        method: request.method().to_string(),
        path: path.clone(),
        headers: request.headers().clone(),
    });
    let reply = state.routes.lock().get(&path).cloned();
    match reply {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(Reply::Body { status, body }) => Response::builder()
            .status(status)
            .header(header::CONTENT_LENGTH, body.len())
            .body(Body::from(body))
            .expect("response"),
        Some(Reply::Chunked(chunks)) => Body::from_stream(futures::stream::iter(
            chunks.into_iter().map(Ok::<_, std::io::Error>),
        ))
        .into_response(),
        Some(Reply::Redirect(location)) => Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, location)
            .body(Body::empty())
            .expect("response"),
        Some(Reply::Slow {
            chunk,
            interval,
            count,
        }) => {
            let stream = futures::stream::unfold(0usize, move |sent| {
                let chunk = chunk.clone();
                async move {
                    if sent == count {
                        return None;
                    }
                    tokio::time::sleep(interval).await;
                    Some((Ok::<_, std::io::Error>(chunk), sent + 1))
                }
            });
            Body::from_stream(stream).into_response()
        }
    }
}

/// A configuration that accepts the local `http://` server, from TOML text.
pub fn local_config(toml: &str) -> UpdateConfig {
    let overrides = ConfigOverrides {
        allow_http: true,
        ..ConfigOverrides::default()
    };
    UpdateConfig::from_toml_str(toml, &overrides).expect("config")
}

/// One entry of a test archive.
pub enum Entry<'a> {
    File(&'a str, &'a [u8]),
    #[cfg(unix)]
    FileWithMode(&'a str, &'a [u8], u32),
    Directory(&'a str),
    #[cfg(unix)]
    Symlink(&'a str, &'a str),
}

/// Builds a ZIP archive in memory; names are written verbatim (including `\`).
pub fn zip_bytes(entries: &[Entry<'_>]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = || {
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
    };
    for entry in entries {
        match entry {
            Entry::File(name, data) => {
                writer.start_file(*name, options()).expect("start");
                writer.write_all(data).expect("write");
            }
            #[cfg(unix)]
            Entry::FileWithMode(name, data, mode) => {
                writer
                    .start_file(*name, options().unix_permissions(*mode))
                    .expect("start");
                writer.write_all(data).expect("write");
            }
            Entry::Directory(name) => writer.add_directory(*name, options()).expect("dir"),
            #[cfg(unix)]
            Entry::Symlink(name, target) => writer
                .add_symlink(*name, *target, options())
                .expect("symlink"),
        }
    }
    writer.finish().expect("finish").into_inner()
}

/// Writes a ZIP archive to `path`.
pub fn write_zip(path: &Path, entries: &[Entry<'_>]) {
    std::fs::write(path, zip_bytes(entries)).expect("write zip");
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Stages every file below `dir` as it is (sorted, `/`-separated relative paths).
pub fn stage_tree(dir: &Path) -> Vec<StagedEntry> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<StagedEntry>) {
        let mut children: Vec<_> = std::fs::read_dir(dir)
            .expect("list")
            .map(|e| e.expect("entry").path())
            .collect();
        children.sort();
        for path in children {
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).expect("inside");
                let relative = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push(StagedEntry {
                    source: path,
                    relative,
                    unix_mode: None,
                });
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}
