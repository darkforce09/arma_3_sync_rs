//! A private data folder per test, the real binary, and a static HTTP server.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use a3s_foundation::paths::AppDirs;
use a3s_repo_files::local::LocalStore;
use a3s_repo_files::services::{ClientData, ProtocolInput};
use a3s_types::local::Repository;
use a3s_types::{Protocol, ProtocolType, ServerInfo};
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::Response;

/// Environment variables of the developer's shell that would change the binary's behaviour.
const CLEARED_VARS: [&str; 13] = [
    "A3S_LOG",
    "A3S_DEBUG",
    "A3S_PORTABLE",
    "A3S_UPDATER_USER_CONFIG_PATH",
    "A3S_UPDATER_URL",
    "A3S_UPDATER_MANIFEST_URL",
    "A3S_UPDATER_LEGACY_XML_URL",
    "A3S_UPDATER_MANIFEST_URL_DEV",
    "A3S_UPDATER_LEGACY_XML_URL_DEV",
    "A3S_UPDATER_GITHUB_API_URL",
    "A3S_UPDATER_GITHUB_API_URL_DEV",
    "A3S_UPDATER_ALLOW_HTTP",
    "A3S_UPDATER_GITHUB_ENABLED",
];

/// A test's own configuration, data and cache folders, plus room for repositories.
pub struct Env {
    tmp: tempfile::TempDir,
    pub dirs: AppDirs,
}

impl Env {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dirs = AppDirs::from_roots(
            tmp.path().join("config"),
            tmp.path().join("data"),
            tmp.path().join("cache"),
        );
        Self { tmp, dirs }
    }

    /// A path below the test's folder.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.tmp.path().join(relative)
    }

    /// The binary, pointed at this test's folders.
    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_arma3sync"));
        for var in CLEARED_VARS {
            command.env_remove(var);
        }
        command
            .env("A3S_CONFIG_DIR", self.dirs.config_dir())
            .env("A3S_DATA_DIR", self.dirs.data_dir())
            .env("A3S_CACHE_DIR", self.dirs.cache_dir())
            .stdin(Stdio::null());
        command
    }

    /// Runs the binary with `args`.
    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run arma3sync")
    }

    /// Runs the console with `input` piped to standard input.
    pub fn console(&self, input: &str) -> Output {
        let mut child = self
            .command()
            .arg("console")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn arma3sync");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(input.as_bytes())
            .expect("write input");
        child.wait_with_output().expect("wait")
    }

    /// The client data as stored now.
    pub fn data(&self) -> ClientData {
        std::fs::create_dir_all(self.dirs.repositories_dir()).expect("mkdir");
        ClientData::load(Arc::new(LocalStore::new(&self.dirs))).data
    }

    /// The stored repository `name`.
    pub fn repository(&self, name: &str) -> Repository {
        self.data()
            .repository(name)
            .expect("stored repository")
            .clone()
    }

    /// Stores a repository using `protocol`, then applies `edit`.
    pub fn add_repository(
        &self,
        name: &str,
        protocol: &Protocol,
        edit: impl FnOnce(&mut Repository),
    ) {
        let mut data = self.data();
        let input = ProtocolInput {
            protocol_type: protocol.protocol_type,
            url: protocol.url.clone(),
            port: protocol.port.clone(),
            login: "anonymous".to_owned(),
            password: String::new(),
            validate_ssl_certificate: false,
        };
        data.create_repository(name, &input)
            .expect("create repository");
        data.update_repository(name, |repository| {
            edit(repository);
            Ok(true)
        })
        .expect("edit repository");
    }
}

/// Asserts the exit code, showing the output otherwise.
#[track_caller]
pub fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Writes `contents` to `path`, creating its folders.
pub fn write(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

/// Content of a repository's addon file: 40 KiB that zsync splits into several blocks.
pub fn pbo_content() -> Vec<u8> {
    (0..40 * 1024u32).map(|i| (i * 7 % 251) as u8).collect()
}

/// A repository folder with one addon and a top-level file.
pub fn write_repository_content(root: &Path) {
    write(&root.join("@mod/addons/mod.pbo"), &pbo_content());
    write(&root.join("@mod/mod.cpp"), b"name = \"Mod\";\n");
    write(&root.join("readme.txt"), b"read me\n");
}

/// The `.a3s/serverinfo` of a built repository.
pub fn server_info(root: &Path) -> ServerInfo {
    a3s_repo_files::read_a3s(&root.join(".a3s/serverinfo")).expect("serverinfo")
}

/// An HTTP protocol for `127.0.0.1:port`.
pub fn http_protocol(port: u16) -> Protocol {
    Protocol::new(
        ProtocolType::Http,
        "127.0.0.1",
        port.to_string(),
        "anonymous",
        "",
        false,
    )
}

/// A local port nothing listens on.
pub fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}

/// A static HTTP server over a folder (`GET` with single ranges, `HEAD`), stopped on drop.
pub struct HttpServer {
    pub port: u16,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl HttpServer {
    pub fn serve(root: &Path) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("addr").port();
        let (shutdown, stopped) = tokio::sync::oneshot::channel::<()>();
        let root = root.to_path_buf();
        std::thread::Builder::new()
            .name("test-http".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
                    let router = axum::Router::new().fallback(move |request: Request| {
                        let root = root.clone();
                        async move { answer(&root, &request) }
                    });
                    axum::serve(listener, router)
                        .with_graceful_shutdown(async {
                            stopped.await.ok();
                        })
                        .await
                        .ok();
                });
            })
            .expect("spawn server");
        Self {
            port,
            shutdown: Some(shutdown),
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).ok();
        }
    }
}

fn answer(root: &Path, request: &Request) -> Response {
    let status = |status: StatusCode| {
        Response::builder()
            .status(status)
            .body(Body::empty())
            .expect("response")
    };
    let relative = request.uri().path().trim_start_matches('/');
    if relative.split('/').any(|part| part == "..") {
        return status(StatusCode::BAD_REQUEST);
    }
    let Ok(bytes) = std::fs::read(root.join(relative)) else {
        return status(StatusCode::NOT_FOUND);
    };
    let len = bytes.len();
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.split_once('-'))
        .and_then(|(start, end)| {
            let start: usize = start.parse().ok()?;
            let end: usize = if end.is_empty() {
                len.checked_sub(1)?
            } else {
                end.parse().ok()?
            };
            (start <= end && start < len).then_some((start, end.min(len - 1)))
        });
    let builder = Response::builder().header(header::ACCEPT_RANGES, "bytes");
    let (builder, body) = match range {
        Some((start, end)) => (
            builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}")),
            bytes[start..=end].to_vec(),
        ),
        None => (builder.status(StatusCode::OK), bytes),
    };
    let builder = builder.header(header::CONTENT_LENGTH, body.len());
    if request.method() == Method::HEAD {
        builder.body(Body::empty())
    } else {
        builder.body(Body::from(body))
    }
    .expect("response")
}
