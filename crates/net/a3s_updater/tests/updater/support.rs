//! In-process HTTP server and ZIP builders for the updater tests.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s_updater::{ConfigOverrides, UpdateConfig};
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
    pub async fn start() -> Self {
        let state = Arc::new(ServerState::default());
        let app = axum::Router::new()
            .fallback(handle)
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        Self {
            base: Url::parse(&format!("http://{addr}/")).expect("base url"),
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
