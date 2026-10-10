//! Transport tests against in-process FTP/FTPS (libunftp), SFTP (russh), WebDAV (dav-server)
//! and HTTP (axum) servers.

mod common;
mod ftp;
mod http;
mod sftp;
mod support;
mod webdav;

use std::path::PathBuf;

use a3s_transport::{RemoteKind, TransferCtx, Transport};

/// A18: `Transport` and the futures of every write method are `Send` (spawned by the upload
/// pipeline). Compile-time only.
#[test]
fn a18_transport_and_futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}

    #[expect(dead_code, reason = "only has to compile")]
    fn spawn_all(mut t: Transport) {
        tokio::spawn(async move {
            let ctx = TransferCtx::new();
            let _exists = t.exists("a", &ctx).await;
            let _file = t.upload_file(&PathBuf::from("a"), "a", &ctx).await;
            let _bytes = t.upload_bytes(b"a", "a", &ctx).await;
            let _mkdirs = t.mkdirs("a", &ctx).await;
            let _delete = t.delete("a", RemoteKind::Directory, &ctx).await;
            let mut session = t.upload_session();
            let _in_session = session.exists("a", &ctx).await;
            drop(session);
            t.close().await;
        });
    }

    assert_send(&TransferCtx::new());
    let _check: fn(&Transport) = |t| assert_send(t);
}
