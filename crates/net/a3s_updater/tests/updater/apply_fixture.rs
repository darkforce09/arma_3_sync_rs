//! A local update server, a temporary installation and fake elevation for the apply tests.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use a3s_foundation::paths::Os;
use a3s_updater::apply::install::release_binary_name;
use a3s_updater::elevation::{ElevatedLaunch, ElevatedLauncher, LaunchExit};
use a3s_updater::{Release, ReleaseFormat, UpdateContext, UpdateEvent, UpdateSession};
use parking_lot::Mutex;

use crate::support::{Entry, Reply, Server, sha256_hex, zip_bytes};

pub const TARGET: &str = "x86_64-unknown-linux-gnu";
pub const OLD_VERSION: &str = "1.0.0";
pub const NEW_VERSION: &str = "9.0.0";
pub const MANIFEST_PATH: &str = "/a3s.json";
pub const ARCHIVE_PATH: &str = "/dl/Arma3Sync-9.0.0.zip";
pub const OLD_BINARY: &[u8] = b"old binary";
pub const NEW_BINARY: &[u8] = b"new binary";

/// A release archive for `os`: the binary and a licence file.
pub fn release_zip(os: Os) -> Vec<u8> {
    zip_bytes(&[
        Entry::File(release_binary_name(os), NEW_BINARY),
        Entry::File("LICENSE.txt", b"license"),
    ])
}

/// An installation of [`OLD_VERSION`] whose configuration points at a local server.
pub struct Fixture {
    pub server: Server,
    pub install: tempfile::TempDir,
    pub staging: tempfile::TempDir,
    pub os: Os,
}

impl Fixture {
    /// Serves [`release_zip`] with a matching manifest.
    pub async fn new(os: Os) -> Self {
        let fixture = Self {
            server: Server::start().await,
            install: tempfile::tempdir().expect("install"),
            staging: tempfile::tempdir().expect("staging"),
            os,
        };
        std::fs::write(fixture.binary(), OLD_BINARY).expect("old binary");
        std::fs::write(
            fixture.install.path().join("updater.toml"),
            format!(
                "[update]\nmanifest_url = \"{}\"\nlegacy_xml_url = \"\"\nallow_http = true\n\
                 [github]\nenabled = false\n",
                fixture.server.url(MANIFEST_PATH)
            ),
        )
        .expect("updater.toml");
        let archive = release_zip(os);
        let sha256 = sha256_hex(&archive);
        fixture.serve(archive, &sha256, 0);
        fixture
    }

    /// Serves `archive` and a manifest announcing `sha256` and `size`.
    pub fn serve(&self, archive: Vec<u8>, sha256: &str, size: u64) {
        self.server.route(ARCHIVE_PATH, Reply::ok(archive));
        self.server.route(
            MANIFEST_PATH,
            Reply::ok(format!(
                r#"{{"version":"{NEW_VERSION}","file":"Arma3Sync-9.0.0.zip","downloadUrl":"{}","sha256":"{sha256}","size":{size}}}"#,
                self.server.url(ARCHIVE_PATH)
            )),
        );
    }

    pub fn dir(&self) -> &Path {
        self.install.path()
    }

    /// The installed application binary.
    pub fn binary(&self) -> PathBuf {
        self.dir().join(release_binary_name(self.os))
    }

    pub fn context(&self, current_version: &str) -> UpdateContext {
        let mut ctx = UpdateContext::new(
            self.binary(),
            self.dir().to_path_buf(),
            current_version,
            self.os,
        );
        ctx.target = Some(TARGET);
        ctx.staging_parent = Some(self.staging.path().to_path_buf());
        ctx
    }

    pub fn session(&self, current_version: &str) -> UpdateSession {
        UpdateSession::new(self.context(current_version))
    }

    /// The release the manifest announces for `archive`.
    pub fn release(&self, archive: &[u8], sha256: Option<String>) -> Release {
        Release {
            version: NEW_VERSION.to_owned(),
            tag: None,
            file_name: "Arma3Sync-9.0.0.zip".to_owned(),
            download_url: self.server.url(ARCHIVE_PATH),
            sha256,
            size: archive.len() as u64,
            format: ReleaseFormat::Json,
            source: self.server.url(MANIFEST_PATH).to_string(),
        }
    }

    /// Every file below the installation with its content.
    pub fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).expect("list") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    out.insert(
                        path.strip_prefix(root).expect("inside").to_path_buf(),
                        Vec::new(),
                    );
                    walk(root, &path, out);
                } else {
                    let content = std::fs::read(&path).expect("read");
                    out.insert(
                        path.strip_prefix(root).expect("inside").to_path_buf(),
                        content,
                    );
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(self.dir(), self.dir(), &mut out);
        out
    }

    /// How many times the archive was downloaded (`GET`).
    pub fn downloads(&self) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|r| r.method == "GET" && r.path == ARCHIVE_PATH)
            .count()
    }

    /// Whether every download staging directory is gone.
    pub fn staging_is_empty(&self) -> bool {
        std::fs::read_dir(self.staging.path())
            .expect("list staging")
            .next()
            .is_none()
    }
}

/// Collects the events of an update.
#[derive(Default)]
pub struct Events(Mutex<Vec<UpdateEvent>>);

impl Events {
    pub fn record(&self, event: UpdateEvent) {
        self.0.lock().push(event);
    }

    /// Every event except `Preparing` and `Downloading`.
    pub fn after_download(&self) -> Vec<UpdateEvent> {
        self.0
            .lock()
            .iter()
            .copied()
            .filter(|e| !matches!(e, UpdateEvent::Preparing | UpdateEvent::Downloading { .. }))
            .collect()
    }

    pub fn all(&self) -> Vec<UpdateEvent> {
        self.0.lock().clone()
    }
}

/// One elevated launch as the fake launcher saw it.
#[derive(Clone, Debug)]
pub struct SeenLaunch {
    pub launch: ElevatedLaunch,

    /// Whether the `--archive` file existed when the launcher ran.
    pub archive_existed: bool,
}

impl SeenLaunch {
    /// The value after `option` in the arguments.
    pub fn value(&self, option: &str) -> String {
        let args = &self.launch.args;
        let index = args.iter().position(|a| a == option).expect("option");
        args[index + 1].clone()
    }
}

/// How a [`FakeLauncher`] answers a launch.
type Answer = Box<dyn Fn(&ElevatedLaunch) -> io::Result<LaunchExit> + Send + Sync>;

/// A launcher that records its calls and answers with `answer`.
pub struct FakeLauncher {
    pub calls: Mutex<Vec<SeenLaunch>>,
    answer: Answer,
}

impl FakeLauncher {
    pub fn new(
        answer: impl Fn(&ElevatedLaunch) -> io::Result<LaunchExit> + Send + Sync + 'static,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        }
    }

    pub fn calls(&self) -> Vec<SeenLaunch> {
        self.calls.lock().clone()
    }
}

impl ElevatedLauncher for FakeLauncher {
    fn run(&self, launch: &ElevatedLaunch, _timeout: Duration) -> io::Result<LaunchExit> {
        let seen = SeenLaunch {
            launch: launch.clone(),
            archive_existed: false,
        };
        let archive_existed = Path::new(&seen.value("--archive")).is_file();
        self.calls.lock().push(SeenLaunch {
            archive_existed,
            ..seen
        });
        (self.answer)(launch)
    }
}

/// Makes `dir` read-only (`0o555`) until dropped; `None` when the process can still write it
/// (running as root).
#[cfg(unix)]
pub struct ReadOnly(PathBuf);

#[cfg(unix)]
impl ReadOnly {
    pub fn new(dir: &Path) -> Option<Self> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let guard = Self(dir.to_path_buf());
        let probe = dir.join(".root-probe");
        if std::fs::write(&probe, b"").is_ok() {
            std::fs::remove_file(probe).expect("remove probe");
            return None;
        }
        Some(guard)
    }
}

#[cfg(unix)]
impl Drop for ReadOnly {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ignored: io::Result<()> =
            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}
