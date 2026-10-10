//! Live, read-only verification of FTP resume against the WOG repository: the server must
//! honour `REST` (no silent restart from 0), and edge offsets behave.
//!
//! Run with `scripts/gate.sh live a3s_transport verify_live`.

use std::sync::atomic::Ordering;

use a3s_transport::{TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType, SyncTreeDirectory, SyncTreeNode};

fn wog() -> Protocol {
    Protocol::new(
        ProtocolType::Ftp,
        "88.99.69.243/Repo/WOG",
        "21",
        "anonymous",
        "",
        false,
    )
}

fn truncate(path: &std::path::Path, len: u64) {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(len)
        .unwrap();
}

#[tokio::test]
#[ignore = "network"]
async fn verify_live_wog_ftp_resume_uses_rest() {
    let protocol = wog();
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    let sync: SyncTreeDirectory =
        a3s_repo_files::decode_a3s(&t.fetch(".a3s/sync", &ctx).await.unwrap()).unwrap();

    let mut smallest: Option<(String, i64, String)> = None;
    sync.walk(&mut |path, node| {
        if let SyncTreeNode::Leaf(l) = node
            && l.size > 1000
            && smallest.as_ref().is_none_or(|s| l.size < s.1)
        {
            smallest = Some((path.to_owned(), l.size, l.sha1.clone().unwrap()));
        }
    });
    let (path, size, sha1) = smallest.unwrap();
    let size = u64::try_from(size).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file");
    assert_eq!(
        t.download_file(&path, &dest, 0, &TransferCtx::new())
            .await
            .unwrap(),
        size
    );

    // Resume from a third: exactly the remaining bytes must be transferred (REST honoured,
    // no fallback restart, which would count `size` bytes).
    let third = size / 3;
    truncate(&dest, third);
    let resume_ctx = TransferCtx::new();
    let len = t
        .download_file(&path, &dest, third, &resume_ctx)
        .await
        .unwrap();
    assert_eq!(len, size);
    assert_eq!(a3s_hash::sha1_file(&dest).unwrap(), sha1);
    assert_eq!(
        resume_ctx.progress.load(Ordering::Relaxed),
        size - third,
        "server did not honour REST: the download restarted from 0"
    );

    // Resume at the full size: nothing to transfer.
    let done_ctx = TransferCtx::new();
    let len = t
        .download_file(&path, &dest, size, &done_ctx)
        .await
        .unwrap();
    assert_eq!(len, size);
    assert_eq!(done_ctx.progress.load(Ordering::Relaxed), 0);
    assert_eq!(a3s_hash::sha1_file(&dest).unwrap(), sha1);

    // Resume from 1 byte before the end.
    truncate(&dest, size - 1);
    let len = t
        .download_file(&path, &dest, size - 1, &TransferCtx::new())
        .await
        .unwrap();
    assert_eq!(len, size);
    assert_eq!(a3s_hash::sha1_file(&dest).unwrap(), sha1);

    // The control connection is still in sync: a session `exists` on the repo works.
    {
        let mut session = t.upload_session();
        assert!(session.exists(&path, &TransferCtx::new()).await.unwrap());
        assert!(
            !session
                .exists(".a3s/does-not-exist", &TransferCtx::new())
                .await
                .unwrap()
        );
        assert!(
            session
                .exists(".a3s/sync", &TransferCtx::new())
                .await
                .unwrap()
        );
    }
    assert!(
        t.exists(".a3s/serverinfo", &TransferCtx::new())
            .await
            .unwrap()
    );
    t.close().await;
}
