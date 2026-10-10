//! Adversarial verification of the WP06 upload side (exists / upload / mkdirs / delete and
//! upload sessions) against the in-process test servers of `tests/transport/support`.

#[allow(dead_code, unused_imports)]
#[path = "transport/support/mod.rs"]
mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s_transport::{Error, RemoteKind, Throttle, TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;

use support::ftp::AbortOnDrop;
use support::{Fixture, random_bytes, snapshot};

macro_rules! each_protocol {
    ($($name:ident => $check:path;)*) => {
        $(
            mod $name {
                use super::*;

                #[tokio::test]
                async fn ftp() {
                    $check(&support::ftp::fixture(false).await).await;
                }

                #[tokio::test]
                async fn ftps() {
                    $check(&support::ftp::fixture(true).await).await;
                }

                #[tokio::test]
                async fn sftp() {
                    $check(&support::sftp::fixture().await).await;
                }

                #[tokio::test]
                async fn webdav() {
                    $check(&support::webdav::fixture().await).await;
                }
            }
        )*
    };
}

each_protocol! {
    verify_empty_upload => empty_upload;
    verify_overwrite_truncates => overwrite_truncates;
    verify_backslash_separators => backslash_separators;
    verify_extra_traversal_rejected => extra_traversal_rejected;
    verify_unicode_names => unicode_names;
    verify_deep_mkdirs => deep_mkdirs;
    verify_session_delete_then_reupload => session_delete_then_reupload;
    verify_pre_cancelled_ops => pre_cancelled_ops;
    verify_delete_tree_with_spaces => delete_tree_with_spaces;
}

/// Empty file (Java SHA-1 "0" case): 0 bytes uploaded, file present and empty.
async fn empty_upload(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"", ".a3s/events", &ctx).await.unwrap();
    let path = fx.base.join(".a3s/events");
    assert!(path.is_file(), "{path:?}");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    assert!(t.exists(".a3s/events", &ctx).await.unwrap());
    assert!(t.fetch(".a3s/events", &ctx).await.unwrap().is_empty());

    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("empty.pbo");
    std::fs::write(&local, b"").unwrap();
    let sent = t.upload_file(&local, "@m/empty.pbo", &ctx).await.unwrap();
    assert_eq!(sent, 0);
    assert_eq!(ctx.progress.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        std::fs::metadata(fx.base.join("@m/empty.pbo"))
            .unwrap()
            .len(),
        0
    );
    t.close().await;
}

/// Overwriting a large file with a small one leaves exactly the small content.
async fn overwrite_truncates(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let big = random_bytes(300 * 1024 + 7, 5);
    t.upload_bytes(&big, "o/file.bin", &ctx).await.unwrap();
    assert!(std::fs::read(fx.base.join("o/file.bin")).unwrap() == big);
    t.upload_bytes(b"short", "o/file.bin", &ctx).await.unwrap();
    assert_eq!(std::fs::read(fx.base.join("o/file.bin")).unwrap(), b"short");
    assert_eq!(t.fetch("o/file.bin", &ctx).await.unwrap(), b"short");
    t.close().await;
}

/// Windows separators are normalised to `/` (B2).
async fn backslash_separators(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"bs", "@m\\addons\\x.pbo", &ctx)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(fx.base.join("@m/addons/x.pbo")).unwrap(),
        b"bs"
    );
    assert!(t.exists("@m/addons/x.pbo", &ctx).await.unwrap());
    assert!(t.exists("\\@m\\addons\\x.pbo", &ctx).await.unwrap());
    t.delete("@m\\addons\\x.pbo", RemoteKind::File, &ctx)
        .await
        .unwrap();
    assert!(!fx.base.join("@m/addons/x.pbo").exists());
    t.close().await;
}

/// More unsafe shapes than A8; none may reach the server.
async fn extra_traversal_rejected(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let before = snapshot(&fx.root());
    let requests_before = fx.request_count.as_ref().map(|count| count());
    let bad = [
        "..\\escape",
        "\\..\\escape",
        "a/./b",
        ".",
        "./",
        "a/b/..",
        "a/b/../../..",
        "a\0b",
        "x\nDELE y",
        "x\rRMD y",
        "ok/\r\n",
    ];
    for path in bad {
        let results = [
            t.upload_bytes(b"x", path, &ctx).await,
            t.mkdirs(path, &ctx).await,
            t.delete(path, RemoteKind::File, &ctx).await,
            t.delete(path, RemoteKind::Directory, &ctx).await,
            t.exists(path, &ctx).await.map(|_| ()),
            t.fetch(path, &ctx).await.map(|_| ()),
        ];
        for (i, result) in results.into_iter().enumerate() {
            assert!(
                matches!(result, Err(Error::InvalidPath(_))),
                "{path:?} op {i}: {result:?}"
            );
        }
    }
    let outer: Vec<_> = std::fs::read_dir(fx.outer.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(outer, ["root"]);
    assert_eq!(snapshot(&fx.root()), before);
    assert_eq!(
        fx.request_count.as_ref().map(|count| count()),
        requests_before
    );
    // The transport is still usable afterwards.
    t.upload_bytes(b"fine", "after/ok.bin", &ctx).await.unwrap();
    assert_eq!(
        std::fs::read(fx.base.join("after/ok.bin")).unwrap(),
        b"fine"
    );
    t.close().await;
}

/// Non-ASCII names land under their UTF-8 name and read back.
async fn unicode_names(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let relative = "@mod/ünï cødé ✓/ファイル (1).pbo";
    let data = random_bytes(4096, 11);
    t.upload_bytes(&data, relative, &ctx).await.unwrap();
    assert!(std::fs::read(fx.base.join(relative)).unwrap() == data);
    assert!(t.exists(relative, &ctx).await.unwrap());
    assert!(t.fetch(relative, &ctx).await.unwrap() == data);
    {
        let mut session = t.upload_session();
        assert!(session.exists(relative, &ctx).await.unwrap());
        assert!(
            !session
                .exists("@mod/ünï cødé ✓/ファイル (2).pbo", &ctx)
                .await
                .unwrap()
        );
    }
    t.close().await;
}

/// Deep `mkdirs` in and outside a session; a later upload into the deepest level works.
async fn deep_mkdirs(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let deep = "a/b/c/d/e/f/g/h/i/j";
    t.mkdirs(deep, &ctx).await.unwrap();
    assert!(fx.base.join(deep).is_dir());
    {
        let mut session = t.upload_session();
        session.mkdirs("a/b/c/d/e/x/y/z", &ctx).await.unwrap();
        session.mkdirs("a/b/c/d/e/x/y/z", &ctx).await.unwrap();
        session
            .upload_bytes(b"deep", "a/b/c/d/e/x/y/z/f.bin", &ctx)
            .await
            .unwrap();
    }
    assert_eq!(
        std::fs::read(fx.base.join("a/b/c/d/e/x/y/z/f.bin")).unwrap(),
        b"deep"
    );
    t.close().await;
}

/// In a session: a directory deleted with `RemoteKind::Directory` is forgotten by the caches,
/// so a later upload into it recreates it.
async fn session_delete_then_reupload(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    {
        let mut session = t.upload_session();
        session
            .upload_bytes(b"one", "q/r/one.bin", &ctx)
            .await
            .unwrap();
        assert!(session.exists("q/r/one.bin", &ctx).await.unwrap());
        session
            .delete("q", RemoteKind::Directory, &ctx)
            .await
            .unwrap();
        assert!(!fx.base.join("q").exists());
        assert!(!session.exists("q/r/one.bin", &ctx).await.unwrap());
        session
            .upload_bytes(b"two", "q/r/two.bin", &ctx)
            .await
            .unwrap();
        assert!(session.exists("q/r/two.bin", &ctx).await.unwrap());
    }
    assert_eq!(std::fs::read(fx.base.join("q/r/two.bin")).unwrap(), b"two");
    t.close().await;
}

/// A cancelled context is refused before any request by every op.
async fn pre_cancelled_ops(fx: &Fixture) {
    let mut t = fx.connect().await;
    fx.put("present.bin", b"p");
    let before = snapshot(&fx.root());
    let requests_before = fx.request_count.as_ref().map(|count| count());
    let ctx = TransferCtx::new();
    ctx.cancel.cancel();
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("f");
    std::fs::write(&local, b"x").unwrap();
    let results = [
        t.exists("present.bin", &ctx).await.map(|_| ()),
        t.mkdirs("m/n", &ctx).await,
        t.delete("present.bin", RemoteKind::File, &ctx).await,
        t.delete("present.bin", RemoteKind::Directory, &ctx).await,
        t.upload_file(&local, "u/f.bin", &ctx).await.map(|_| ()),
        t.upload_bytes(b"x", "u/g.bin", &ctx).await,
    ];
    for (i, result) in results.into_iter().enumerate() {
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "op {i}: {result:?}"
        );
    }
    assert_eq!(snapshot(&fx.root()), before);
    assert_eq!(
        fx.request_count.as_ref().map(|count| count()),
        requests_before
    );
    t.close().await;
}

/// Recursive delete of a tree whose names contain spaces.
async fn delete_tree_with_spaces(fx: &Fixture) {
    fx.put("sp ace/s u b/f 1.pbo", b"1");
    fx.put("sp ace/f 2.pbo", b"2");
    std::fs::create_dir_all(fx.base.join("sp ace/e m p t y")).unwrap();
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.delete("sp ace", RemoteKind::Directory, &ctx)
        .await
        .unwrap();
    assert!(!fx.base.join("sp ace").exists());
    t.close().await;
}

// ---------------------------------------------------------------------------------------
// FTP specifics
// ---------------------------------------------------------------------------------------

/// After a cancelled upload, the same FTP transport can still upload and download data
/// (not only answer `MLST`): B16.
async fn ftp_cancel_then_transfer(ftps: bool) {
    let fx = support::ftp::fixture(ftps).await;
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("big.bin");
    std::fs::write(&local, random_bytes(4 << 20, 1)).unwrap();
    let mut t = fx.connect().await;
    let ctx = TransferCtx {
        throttle: Some(Arc::new(Throttle::new(256 * 1024))),
        ..TransferCtx::new()
    };
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();
    });
    let result = t.upload_file(&local, "big/big.bin", &ctx).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");

    let fresh = TransferCtx::new();
    let data = random_bytes((1 << 20) + 3, 2);
    t.upload_bytes(&data, "big/after.bin", &fresh)
        .await
        .unwrap();
    assert!(std::fs::read(fx.base.join("big/after.bin")).unwrap() == data);
    assert!(t.fetch("big/after.bin", &fresh).await.unwrap() == data);
    // A second cancel/transfer cycle.
    let ctx2 = TransferCtx {
        throttle: Some(Arc::new(Throttle::new(256 * 1024))),
        ..TransferCtx::new()
    };
    let cancel2 = ctx2.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel2.cancel();
    });
    let result = t.upload_file(&local, "big/big.bin", &ctx2).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    let sent = t.upload_file(&local, "big/full.bin", &fresh).await.unwrap();
    assert_eq!(sent, 4 << 20);
    assert!(std::fs::read(fx.base.join("big/full.bin")).unwrap() == std::fs::read(&local).unwrap());
    t.close().await;
}

#[tokio::test]
async fn verify_ftp_cancel_then_transfer() {
    ftp_cancel_then_transfer(false).await;
}

#[tokio::test]
async fn verify_ftps_cancel_then_transfer() {
    ftp_cancel_then_transfer(true).await;
}

/// Listing cached as "missing" is dropped when the directory is created by `mkdirs`.
#[tokio::test]
async fn verify_ftp_listing_invalidated_by_mkdir() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    assert!(!session.exists("new_dir/f.bin", &ctx).await.unwrap());
    session.mkdirs("new_dir", &ctx).await.unwrap();
    // Another worker uploads into the fresh directory.
    fx.put("new_dir/f.bin", b"other worker");
    assert!(session.exists("new_dir/f.bin", &ctx).await.unwrap());
    drop(session);
    t.close().await;
}

/// The parent's cached listing learns about a directory created by `mkdirs` and about an
/// uploaded file, without a second listing.
#[tokio::test]
async fn verify_ftp_listing_updated_by_mkdir_and_upload() {
    let fx = support::ftp::fixture(false).await;
    fx.put("p/existing.bin", b"e");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    assert!(session.exists("p/existing.bin", &ctx).await.unwrap());
    assert!(!session.exists("p/sub", &ctx).await.unwrap());
    session.mkdirs("p/sub", &ctx).await.unwrap();
    assert!(session.exists("p/sub", &ctx).await.unwrap());
    session.upload_bytes(b"n", "p/new.bin", &ctx).await.unwrap();
    assert!(session.exists("p/new.bin", &ctx).await.unwrap());
    assert_eq!(session.upload_session_stats().directory_listings, 1);
    drop(session);
    // A new session starts with fresh counters.
    let session = t.upload_session();
    assert_eq!(session.upload_session_stats().directory_listings, 0);
    assert_eq!(session.upload_session_stats().cached_file_checks, 0);
    drop(session);
    t.close().await;
}

/// Dot files hidden from `LIST` are confirmed with `MLST` (B12).
#[tokio::test]
async fn verify_ftp_session_dot_file_confirmed() {
    let fx = support::ftp::fixture(false).await;
    fx.put("d/visible.bin", b"v");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    assert!(!session.exists("d/.hidden", &ctx).await.unwrap());
    fx.put("d/.hidden", b"h");
    assert!(session.exists("d/.hidden", &ctx).await.unwrap());
    drop(session);
    t.close().await;
}

/// A known directory removed behind the session's back is recreated once (B14).
#[tokio::test]
async fn verify_ftp_known_dir_vanished_is_recreated() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session.upload_bytes(b"a", "k/l/a.bin", &ctx).await.unwrap();
    std::fs::remove_dir_all(fx.base.join("k")).unwrap();
    session.upload_bytes(b"b", "k/l/b.bin", &ctx).await.unwrap();
    assert_eq!(std::fs::read(fx.base.join("k/l/b.bin")).unwrap(), b"b");
    drop(session);
    t.close().await;
}

/// Recursive delete removes dot files and dot directories too (B8: "whole tree gone").
#[tokio::test]
async fn verify_ftp_delete_dir_with_dot_entries() {
    let fx = support::ftp::fixture(false).await;
    fx.put("h/.a3s/sync", b"s");
    fx.put("h/.hidden", b"h");
    fx.put("h/f.bin", b"f");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.delete("h", RemoteKind::Directory, &ctx).await.unwrap();
    assert!(!fx.base.join("h").exists(), "{:?}", snapshot(&fx.root()));
    t.close().await;
}

/// Case-differing names: the session cache and MLST agree.
#[tokio::test]
async fn verify_ftp_case_sensitivity_consistent() {
    let fx = support::ftp::fixture(false).await;
    fx.put("c/File.PBO", b"x");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let outside = t.exists("c/file.pbo", &ctx).await.unwrap();
    let mut session = t.upload_session();
    let inside = session.exists("c/file.pbo", &ctx).await.unwrap();
    assert_eq!(outside, inside);
    assert!(session.exists("c/File.PBO", &ctx).await.unwrap());
    drop(session);
    t.close().await;
}

// ---------------------------------------------------------------------------------------
// SFTP specifics
// ---------------------------------------------------------------------------------------

/// B21: a configured path of `/` (URL `host/`) is the SFTP root, not the home directory
/// (`SftpDAO.resolveConfiguredDirectory`, `"/".equals(configured)` → `"/"`).
#[tokio::test]
async fn verify_sftp_root_url_path_is_root_not_home() {
    let fx = support::sftp::fixture_with("/home/a3s", "/", support::sftp::PASSWORD, "/").await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"root", "x.bin", &ctx).await.unwrap();
    let at_root = fx.root().join("x.bin").is_file();
    let at_home = fx.root().join("home/a3s/x.bin").is_file();
    assert!(
        at_root && !at_home,
        "uploaded to root: {at_root}, to home: {at_home}"
    );
    t.close().await;
}

/// SFTP: `delete(File)` on a missing file and on a file in a missing directory is `Ok`.
#[tokio::test]
async fn verify_sftp_delete_missing_parent() {
    let fx = support::sftp::fixture().await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.delete("no/such/dir/f.bin", RemoteKind::File, &ctx)
        .await
        .unwrap();
    t.delete("no/such/dir", RemoteKind::Directory, &ctx)
        .await
        .unwrap();
    assert!(!t.exists("no/such/dir/f.bin", &ctx).await.unwrap());
    t.close().await;
}

// ---------------------------------------------------------------------------------------
// WebDAV specifics
// ---------------------------------------------------------------------------------------

/// Percent sequences in names are literal: `%2e%2e` must not become `..` on the server.
#[tokio::test]
async fn verify_webdav_percent_names_do_not_escape() {
    let (fx, log) =
        support::webdav::fixture_with(support::webdav::USER, support::webdav::PASSWORD, true).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"p", "%2e%2e/escape", &ctx).await.unwrap();
    t.upload_bytes(b"q", "a/%2E%2E/%2e%2e/escape2", &ctx)
        .await
        .unwrap();
    t.upload_bytes(b"r", "%2F/s", &ctx).await.unwrap();
    let outer: Vec<_> = std::fs::read_dir(fx.outer.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(outer, ["root"]);
    assert_eq!(std::fs::read(fx.base.join("%2e%2e/escape")).unwrap(), b"p");
    assert_eq!(
        std::fs::read(fx.base.join("a/%2E%2E/%2e%2e/escape2")).unwrap(),
        b"q"
    );
    assert_eq!(std::fs::read(fx.base.join("%2F/s")).unwrap(), b"r");
    assert!(t.exists("%2e%2e/escape", &ctx).await.unwrap());
    assert!(
        log.lock()
            .iter()
            .any(|r| r.method == "PUT" && r.path == "/%252e%252e/escape")
    );
    t.close().await;
}

/// WebDAV directory deletes use a `/`-terminated URL; file deletes do not.
#[tokio::test]
async fn verify_webdav_delete_urls() {
    let (fx, log) =
        support::webdav::fixture_with(support::webdav::USER, support::webdav::PASSWORD, true).await;
    fx.put("d d/f.bin", b"f");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.delete("d d/f.bin", RemoteKind::File, &ctx).await.unwrap();
    t.delete("d d", RemoteKind::Directory, &ctx).await.unwrap();
    let deletes: Vec<_> = log
        .lock()
        .iter()
        .filter(|r| r.method == "DELETE")
        .map(|r| r.path.clone())
        .collect();
    assert_eq!(deletes, ["/d%20d/f.bin", "/d%20d/"]);
    assert!(!fx.base.join("d d").exists());
    t.close().await;
}

async fn stub(statuses: &'static [(&'static str, &'static str, u16)]) -> (Protocol, AbortOnDrop) {
    let router = axum::Router::new().fallback(move |req: axum::extract::Request| async move {
        let method = req.method().as_str().to_owned();
        let path = req.uri().path().to_owned();
        let status = statuses
            .iter()
            .find(|(m, p, _)| *m == method && *p == path)
            .map_or(404, |(_, _, s)| *s);
        StatusCode::from_u16(status).unwrap().into_response()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = AbortOnDrop(tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    }));
    let protocol = Protocol::new(
        ProtocolType::HttpWebdav,
        "127.0.0.1",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    (protocol, task)
}

/// B31 mapping of WebDAV `exists`, B32 MKCOL 405 race and MKCOL errors.
#[tokio::test]
async fn verify_webdav_status_mapping() {
    static STATUSES: &[(&str, &str, u16)] = &[
        ("HEAD", "/ok.bin", 200),
        ("HEAD", "/gone.bin", 410),
        ("HEAD", "/forbidden.bin", 403),
        ("HEAD", "/denied.bin", 401),
        ("HEAD", "/boom.bin", 500),
        ("PROPFIND", "/race/", 404),
        ("MKCOL", "/race/", 405),
        ("PROPFIND", "/conflict/", 404),
        ("MKCOL", "/conflict/", 409),
        ("PROPFIND", "/exists/", 207),
        ("PUT", "/exists/f.bin", 204),
        ("DELETE", "/gone.bin", 410),
    ];
    let (protocol, _server) = stub(STATUSES).await;
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    assert!(t.exists("ok.bin", &ctx).await.unwrap());
    assert!(!t.exists("gone.bin", &ctx).await.unwrap());
    assert!(!t.exists("missing.bin", &ctx).await.unwrap());
    let r = t.exists("forbidden.bin", &ctx).await;
    assert!(matches!(r, Err(Error::Unauthorized)), "{r:?}");
    let r = t.exists("denied.bin", &ctx).await;
    assert!(matches!(r, Err(Error::Unauthorized)), "{r:?}");
    let r = t.exists("boom.bin", &ctx).await;
    assert!(
        matches!(r, Err(Error::HttpStatus { status: 500, .. })),
        "{r:?}"
    );
    t.mkdirs("race", &ctx).await.unwrap();
    let r = t.mkdirs("conflict", &ctx).await;
    assert!(r.is_err(), "{r:?}");
    t.upload_bytes(b"x", "exists/f.bin", &ctx).await.unwrap();
    t.delete("gone.bin", RemoteKind::File, &ctx).await.unwrap();
}

// ---------------------------------------------------------------------------------------
// HTTP specifics
// ---------------------------------------------------------------------------------------

/// B40: only 200 is `true`; other 2xx `false`; 403 is a plain HTTP status error.
#[tokio::test]
async fn verify_http_exists_mapping() {
    static STATUSES: &[(&str, &str, u16)] = &[
        ("HEAD", "/nocontent.bin", 204),
        ("HEAD", "/gone.bin", 410),
        ("HEAD", "/forbidden.bin", 403),
        ("HEAD", "/notimpl.bin", 501),
        ("GET", "/notimpl.bin", 200),
        ("HEAD", "/notimpl404.bin", 501),
        ("GET", "/notimpl404.bin", 404),
    ];
    let (mut protocol, _server) = stub(STATUSES).await;
    protocol.protocol_type = ProtocolType::Http;
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    assert!(!t.exists("nocontent.bin", &ctx).await.unwrap());
    assert!(!t.exists("gone.bin", &ctx).await.unwrap());
    let r = t.exists("forbidden.bin", &ctx).await;
    assert!(
        matches!(r, Err(Error::HttpStatus { status: 403, .. })),
        "{r:?}"
    );
    assert!(t.exists("notimpl.bin", &ctx).await.unwrap());
    assert!(!t.exists("notimpl404.bin", &ctx).await.unwrap());
    let r = t.exists("", &ctx).await;
    assert!(matches!(r, Err(Error::InvalidPath(_))), "{r:?}");
    let local = tempfile::NamedTempFile::new().unwrap();
    let r = t.upload_file(Path::new(local.path()), "../x", &ctx).await;
    assert!(
        matches!(r, Err(Error::InvalidPath(_) | Error::Unsupported { .. })),
        "{r:?}"
    );
}

/// Session `exists` in a directory whose name contains a space: Java lists the current
/// directory after `CWD` (`FtpDAO.java:460-462`, `listFiles()` without argument), so the
/// name never travels as a `LIST` argument; servers that split `LIST` arguments on spaces
/// (libunftp, Pure-FTPd) must still answer `true` for a present file.
#[tokio::test]
async fn verify_ftp_session_exists_in_dir_with_space() {
    let fx = support::ftp::fixture(false).await;
    fx.put("m a/f.bin", b"x");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    assert!(t.exists("m a/f.bin", &ctx).await.unwrap());
    let mut session = t.upload_session();
    assert!(session.exists("m a/f.bin", &ctx).await.unwrap());
    drop(session);
    t.close().await;
}

/// Same as `verify_unicode_names` for FTP but without spaces: isolates the UTF-8 handling.
#[tokio::test]
async fn verify_ftp_session_unicode_without_space() {
    let fx = support::ftp::fixture(false).await;
    fx.put("@mod/ünïcødé✓/ファイル.pbo", b"u");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    assert!(
        session
            .exists("@mod/ünïcødé✓/ファイル.pbo", &ctx)
            .await
            .unwrap()
    );
    drop(session);
    t.close().await;
}

/// Only the leaf of a known directory vanished; its parent is still there (B14).
#[tokio::test]
async fn verify_ftp_known_leaf_dir_vanished_is_recreated() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session.upload_bytes(b"a", "k/l/a.bin", &ctx).await.unwrap();
    std::fs::remove_dir_all(fx.base.join("k/l")).unwrap();
    session.upload_bytes(b"b", "k/l/b.bin", &ctx).await.unwrap();
    assert_eq!(std::fs::read(fx.base.join("k/l/b.bin")).unwrap(), b"b");
    drop(session);
    t.close().await;
}
