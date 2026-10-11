//! Acceptance checks shared by every upload protocol (FTP, FTPS, SFTP, HTTP-WebDAV).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use a3s_repo_files::{decode_a3s, encode_a3s};
use a3s_transport::{Error, RemoteKind, Throttle, TransferCtx, Transport};
use a3s_types::ServerInfo;

use crate::support::{Fixture, random_bytes, snapshot};

/// Upload a 3 MiB file below two missing directories, read it back three ways.
pub async fn upload_file_round_trip(fx: &Fixture) {
    const LEN: usize = 3 << 20;
    let data = random_bytes(LEN, 3);
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("a b.pbo");
    std::fs::write(&local, &data).unwrap();

    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let relative = "@mod/addons/a b.pbo";
    let sent = t.upload_file(&local, relative, &ctx).await.unwrap();
    assert_eq!(sent, LEN as u64);
    assert_eq!(ctx.progress.load(Ordering::Relaxed), LEN as u64);
    assert!(std::fs::read(fx.base.join(relative)).unwrap() == data);

    let fetched = t.fetch(relative, &TransferCtx::new()).await.unwrap();
    assert!(fetched == data);
    let dest = local_dir.path().join("download");
    let len = t
        .download_file(relative, &dest, 0, &TransferCtx::new())
        .await
        .unwrap();
    assert_eq!(len, LEN as u64);
    assert!(std::fs::read(&dest).unwrap() == data);
    t.close().await;
}

/// `.a3s/serverinfo` bytes reach the server untouched; overwriting replaces them.
pub async fn upload_bytes_identity(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    for revision in [7, 8] {
        let info = ServerInfo {
            revision,
            number_of_files: 42,
            ..ServerInfo::default()
        };
        let bytes = encode_a3s(&info).unwrap();
        t.upload_bytes(&bytes, ".a3s/serverinfo", &ctx)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(fx.base.join(".a3s/serverinfo")).unwrap(),
            bytes
        );
        let fetched = t.fetch(".a3s/serverinfo", &ctx).await.unwrap();
        assert_eq!(fetched, bytes);
        let decoded: ServerInfo = decode_a3s(&fetched).unwrap();
        assert_eq!(decoded, info);
    }
    t.close().await;
}

/// `exists` for present, missing, missing-parent and dot files, outside and inside a
/// session.
pub async fn exists(fx: &Fixture) {
    async fn check(t: &mut Transport, ctx: &TransferCtx) {
        assert!(t.exists("dir/up.bin", ctx).await.unwrap());
        assert!(!t.exists("dir/missing.bin", ctx).await.unwrap());
        assert!(!t.exists("nodir/missing.bin", ctx).await.unwrap());
        assert!(!t.exists(".hidden", ctx).await.unwrap());
        assert!(t.exists(".a3s/sync", ctx).await.unwrap());
    }
    fx.put(".a3s/sync", b"sync");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"x", "dir/up.bin", &ctx).await.unwrap();
    check(&mut t, &ctx).await;
    {
        let mut session = t.upload_session();
        check(&mut session, &ctx).await;
    }
    t.close().await;
}

/// `mkdirs` is idempotent, `mkdirs("")` is a no-op.
pub async fn mkdirs_idempotent(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.mkdirs("x/y/z", &ctx).await.unwrap();
    t.mkdirs("x/y/z", &ctx).await.unwrap();
    assert!(fx.base.join("x/y/z").is_dir());
    let before = snapshot(&fx.root());
    t.mkdirs("", &ctx).await.unwrap();
    assert_eq!(snapshot(&fx.root()), before);
    t.close().await;
}

/// Delete files and directory trees; missing targets are fine.
pub async fn delete(fx: &Fixture) {
    fx.put("file.bin", b"f");
    fx.put("d/f1", b"1");
    fx.put("d/s/f2", b"2");
    std::fs::create_dir_all(fx.base.join("d/s/t")).unwrap();
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.delete("file.bin", RemoteKind::File, &ctx).await.unwrap();
    assert!(!fx.base.join("file.bin").exists());
    t.delete("file.bin", RemoteKind::File, &ctx).await.unwrap();
    t.delete("d", RemoteKind::Directory, &ctx).await.unwrap();
    assert!(!fx.base.join("d").exists());
    t.delete("d", RemoteKind::Directory, &ctx).await.unwrap();
    t.close().await;
}

/// Unsafe paths fail with `InvalidPath` before any request reaches the server.
pub async fn path_traversal(fx: &Fixture) {
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("f");
    std::fs::write(&local, b"escape").unwrap();
    let before = snapshot(&fx.root());
    let requests_before = fx.request_count.as_ref().map(|count| count());

    let results = [
        t.upload_file(&local, "../escape", &ctx).await.map(|_| ()),
        t.upload_bytes(b"x", "a/../../escape", &ctx).await,
        t.delete("..", RemoteKind::Directory, &ctx).await,
        t.delete("", RemoteKind::Directory, &ctx).await,
        t.delete("/", RemoteKind::File, &ctx).await,
        t.mkdirs("a/../..", &ctx).await,
        t.exists("../x", &ctx).await.map(|_| ()),
        t.fetch("../x", &ctx).await.map(|_| ()),
    ];
    for (i, result) in results.into_iter().enumerate() {
        assert!(
            matches!(result, Err(Error::InvalidPath(_))),
            "case {i}: {result:?}"
        );
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
    t.close().await;
}

/// Cancelling a throttled upload returns quickly; the transport stays usable (or reports
/// a lost connection). A pre-cancelled context sends nothing.
pub async fn cancellation(fx: &Fixture, must_survive: bool) {
    let local_dir = tempfile::tempdir().unwrap();
    let local = local_dir.path().join("big.bin");
    std::fs::write(&local, random_bytes(4 << 20, 17)).unwrap();
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
    let start = Instant::now();
    let result = t.upload_file(&local, "big/big.bin", &ctx).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );

    let after = t.exists(".a3s/x", &TransferCtx::new()).await;
    match after {
        Ok(found) => assert!(!found),
        Err(err) => assert!(
            !must_survive && err.is_connection_lost(),
            "connection unusable after cancel: {err:?}"
        ),
    }

    let requests_before = fx.request_count.as_ref().map(|count| count());
    let cancelled = TransferCtx::new();
    cancelled.cancel.cancel();
    let result = t.upload_bytes(b"x", "pre/cancelled.bin", &cancelled).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert!(!fx.base.join("pre").exists());
    assert_eq!(
        fx.request_count.as_ref().map(|count| count()),
        requests_before
    );
}

/// Resume: download, cut the local file in half, resume from there.
pub async fn resume_download(fx: &Fixture) {
    let data = random_bytes(1033, 21);
    fx.put("addons/f.pbo", &data);
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.pbo");
    assert_eq!(
        t.download_file("addons/f.pbo", &dest, 0, &ctx)
            .await
            .unwrap(),
        1033
    );
    let half = 516;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&dest)
        .unwrap()
        .set_len(half)
        .unwrap();
    let len = t
        .download_file("addons/f.pbo", &dest, half, &ctx)
        .await
        .unwrap();
    assert_eq!(len, 1033);
    assert!(std::fs::read(&dest).unwrap() == data);
    t.close().await;
}
