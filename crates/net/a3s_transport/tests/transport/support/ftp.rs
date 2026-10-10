//! libunftp FTP / FTPS server over a tempdir.

use std::path::PathBuf;

use a3s_types::{Protocol, ProtocolType};
use libunftp::ServerBuilder;
use unftp_sbe_fs::Filesystem;

use super::{Fixture, outer_with_root, tls_pem_files};

/// Serves `root` on `127.0.0.1:<port>`; returns the port and the accept task.
pub async fn serve(root: PathBuf, ftps: bool) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind FTP");
    let port = listener.local_addr().expect("local addr").port();
    // Certificate files live outside the served tree, for as long as the accept loop runs.
    let tls = ftps.then(tls_pem_files);
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let root = root.clone();
            let mut builder = ServerBuilder::new(Box::new(move || {
                Filesystem::new(root.clone()).expect("filesystem back-end")
            }))
            .greeting("a3s test server")
            .passive_ports(40000..=60000);
            if let Some((_dir, cert, key)) = &tls {
                builder = builder.ftps(cert.clone(), key.clone());
            }
            let server = builder.build().expect("FTP server");
            tokio::spawn(async move {
                let _ignored = server.service(stream).await;
            });
        }
    });
    (port, task)
}

/// FTP (or FTPS) fixture; `remote_path` is appended to the protocol URL (e.g. `/repo`).
pub async fn fixture_with_path(ftps: bool, remote_path: &str) -> Fixture {
    let (outer, root) = outer_with_root();
    let (port, task) = serve(root.clone(), ftps).await;
    let protocol_type = if ftps {
        ProtocolType::Ftps
    } else {
        ProtocolType::Ftp
    };
    let protocol = Protocol::new(
        protocol_type,
        format!("127.0.0.1{remote_path}"),
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    let base = root.join(remote_path.trim_start_matches('/'));
    Fixture {
        protocol,
        base,
        outer,
        request_count: None,
        _server: Box::new(AbortOnDrop(task)),
    }
}

pub async fn fixture(ftps: bool) -> Fixture {
    fixture_with_path(ftps, "").await
}

/// Aborts a server task when the fixture is dropped.
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
