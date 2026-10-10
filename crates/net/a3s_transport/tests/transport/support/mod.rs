//! In-process test servers and shared helpers.

pub mod ftp;
pub mod sftp;
pub mod strict_ftp;
pub mod webdav;

use std::path::{Path, PathBuf};

use a3s_transport::{Transport, TransportOptions};
use a3s_types::Protocol;

/// A running server whose files live in `<outer>/root`.
pub struct Fixture {
    pub protocol: Protocol,

    /// Server-side directory of the repository base.
    pub base: PathBuf,

    /// Tempdir containing the server root; nothing else may appear in it.
    pub outer: tempfile::TempDir,

    /// Number of requests the server received (WebDAV only; `None` elsewhere).
    pub request_count: Option<Box<dyn Fn() -> usize + Send + Sync>>,

    /// Keeps the server alive.
    pub _server: Box<dyn std::any::Any + Send + Sync>,
}

impl Fixture {
    pub fn root(&self) -> PathBuf {
        self.outer.path().join("root")
    }

    pub async fn connect(&self) -> Transport {
        let options = TransportOptions::from_protocol(&self.protocol);
        Transport::connect(&self.protocol, &options)
            .await
            .expect("connect to test server")
    }

    /// Writes a file below the base directory on the server side.
    pub fn put(&self, relative: &str, bytes: &[u8]) {
        let path = self.base.join(relative);
        std::fs::create_dir_all(path.parent().expect("file has a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }
}

/// Creates `<tempdir>/root` and returns both.
pub fn outer_with_root() -> (tempfile::TempDir, PathBuf) {
    let outer = tempfile::tempdir().expect("tempdir");
    let root = outer.path().join("root");
    std::fs::create_dir(&root).expect("create root");
    (outer, root)
}

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

/// Sorted list of `(relative path, contents)` below `dir`; directories have no contents.
pub fn snapshot(dir: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Option<Vec<u8>>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").path())
            .collect();
        entries.sort();
        for path in entries {
            let rel = path
                .strip_prefix(base)
                .expect("below base")
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                out.push((rel, None));
                walk(base, &path, out);
            } else {
                out.push((rel, Some(std::fs::read(&path).expect("read"))));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

fn self_signed() -> rcgen::CertifiedKey<rcgen::KeyPair> {
    rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])
        .expect("self-signed certificate")
}

/// Self-signed certificate and key for `127.0.0.1` written as PEM files into a new tempdir.
pub fn tls_pem_files() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let certified = self_signed();
    let dir = tempfile::tempdir().expect("tempdir");
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    std::fs::write(&cert, certified.cert.pem()).expect("write certificate");
    std::fs::write(&key, certified.signing_key.serialize_pem()).expect("write key");
    (dir, cert, key)
}
