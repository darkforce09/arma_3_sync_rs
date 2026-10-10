//! Live, read-only tests against public Arma3Sync repositories.
//!
//! Run with `cargo test -p a3s_transport -- --ignored`.

use a3s_transport::{TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType, ServerInfo, SyncTreeDirectory, SyncTreeNode};

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

#[tokio::test]
#[ignore = "network"]
async fn live_wog_ftp() {
    let protocol = wog();
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();

    let info: ServerInfo =
        a3s_repo_files::decode_a3s(&t.fetch(".a3s/serverinfo", &ctx).await.unwrap()).unwrap();
    assert!(info.revision > 0);
    let sync: SyncTreeDirectory =
        a3s_repo_files::decode_a3s(&t.fetch(".a3s/sync", &ctx).await.unwrap()).unwrap();
    assert!(!t.supports_partial_transfer(&ctx).await);

    // Smallest non-empty file somewhere in the tree.
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

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file");
    let len = t.download_file(&path, &dest, 0, &ctx).await.unwrap();
    assert_eq!(len as i64, size);
    assert_eq!(a3s_hash::sha1_file(&dest).unwrap(), sha1);

    // Resume: keep the first half, fetch the rest with REST.
    let half = (size / 2) as u64;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&dest)
        .unwrap()
        .set_len(half)
        .unwrap();
    let len = t.download_file(&path, &dest, half, &ctx).await.unwrap();
    assert_eq!(len as i64, size);
    assert_eq!(a3s_hash::sha1_file(&dest).unwrap(), sha1);

    let missing = t.fetch("does/not/exist.pbo", &ctx).await;
    assert!(
        matches!(missing, Err(a3s_transport::Error::NotFound(_))),
        "{missing:?}"
    );
    t.close().await;
}
