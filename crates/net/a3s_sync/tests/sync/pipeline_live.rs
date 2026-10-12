//! Live, read-only synchronization of two small files from the public WOG repository
//! (`.a3s/*` plus less than 5 MB).
//!
//! Run with `cargo test -p a3s_sync --test sync pipeline_live:: -- --ignored`.

use crate::pipeline_support;

use a3s_sync::pipeline::RunEnd;
use a3s_sync::{EventSender, MemoryStore, Outcome, PlanNode, SyncSession};
use a3s_transport::CancellationToken;
use a3s_types::{Protocol, ProtocolType};
use pipeline_support::{check_request, repository};

/// Files are picked below this size, and their total stays below 5 MB.
const MAX_FILE_SIZE: u64 = 2_000_000;

#[tokio::test]
#[ignore = "network"]
async fn live_wog_two_small_files() {
    let protocol = Protocol::new(
        ProtocolType::Ftp,
        "88.99.69.243/Repo/WOG",
        "21",
        "anonymous",
        "",
        false,
    );
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = std::sync::Arc::new(MemoryStore::new([repository(
        protocol.clone(),
        dest,
        false,
    )]));
    let mut session = SyncSession::check(
        check_request(protocol.clone()),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");

    // The two smallest non-empty leaves.
    let mut candidates: Vec<(u64, String)> = Vec::new();
    session.tree().walk(&mut |path, node| {
        if let PlanNode::Leaf(leaf) = node {
            let size = leaf.download_size();
            if size > 0 && size < MAX_FILE_SIZE {
                candidates.push((size, path.to_owned()));
            }
        }
    });
    candidates.sort();
    let picked: Vec<String> = candidates.into_iter().take(2).map(|(_, p)| p).collect();
    assert_eq!(picked.len(), 2);
    for path in &picked {
        assert!(session.tree_mut().set_selected_recursive(path, true));
    }
    session.refresh_summary();
    let files: Vec<_> = session
        .summary()
        .download_tasks
        .iter()
        .filter(|t| matches!(t.kind, a3s_sync::DownloadTaskKind::File { .. }))
        .collect();
    assert_eq!(files.len(), 2);
    assert!(session.summary().total_download_size < 5_000_000);
    assert!(session.summary().delete_tasks.is_empty());

    let run = session
        .download(&CancellationToken::new(), &EventSender::noop())
        .await;
    assert_eq!(run.end, RunEnd::Finished(Outcome::Success), "{run:?}");
    for path in &picked {
        let Some(PlanNode::Leaf(leaf)) = session.tree().get(path) else {
            panic!("{path}");
        };
        let local = dest.join(path);
        assert_eq!(
            a3s_repo_files::hash::sha1_file(&local).ok(),
            leaf.sha1.clone(),
            "{path}"
        );
    }

    // A second check finds both files up to date.
    let session = SyncSession::check(
        check_request(protocol),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("second check");
    for path in &picked {
        assert!(
            session.tree().get(path).is_some_and(|n| !n.updated()),
            "{path}"
        );
    }
}
