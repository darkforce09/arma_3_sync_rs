//! Live, read-only download test against the public WOG repository.
//!
//! Downloads `.a3s/sync` plus at most 3 files of at most 1 MB each (3 MB in total).
//! Run with `cargo test -p a3s_sync -- --ignored --test-threads=1`.

use std::sync::Arc;

use a3s_sync::download::{DownloadEnd, DownloadRequest, NoZipSink, download};
use a3s_sync::{DownloadStatus, DownloadTask, EventSender, PlanNode, PlanTree};
use a3s_transport::{CancellationToken, Throttle, TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType, SyncTreeDirectory};

const MAX_FILE: u64 = 1_000_000;
const MAX_TOTAL: u64 = 3_000_000;

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
async fn live_wog_download() {
    let protocol = wog();
    let options = TransportOptions::from_protocol(&protocol);
    let mut transport = Transport::connect(&protocol, &options)
        .await
        .expect("connect");
    let bytes = transport
        .fetch(".a3s/sync", &TransferCtx::new())
        .await
        .expect("fetch sync");
    transport.close().await;
    let sync: SyncTreeDirectory = a3s_repo_files::decode_a3s(&bytes).expect("decode sync");
    let tree = PlanTree::from_sync_tree(&sync);

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let mut tasks: Vec<DownloadTask> = Vec::new();
    let mut total = 0;
    tree.walk(&mut |relative, node| {
        let PlanNode::Leaf(leaf) = node else {
            return;
        };
        let size = leaf.download_size();
        if tasks.len() >= 3 || size == 0 || size > MAX_FILE || total + size > MAX_TOTAL {
            return;
        }
        let mut leaf = leaf.clone();
        let parent = a3s_sync::parent_relative_path(relative);
        leaf.destination_path = Some(dest.join(parent));
        let task = DownloadTask::from_node(relative, &PlanNode::Leaf(leaf)).expect("task");
        total += size;
        tasks.push(task);
    });
    assert!(!tasks.is_empty(), "no small leaf in the WOG tree");

    let request = DownloadRequest {
        repository_name: "WOG".to_owned(),
        transport_options: options,
        connections: 2,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
        throttle: Arc::new(Throttle::new(0)),
        disk_usage_path: None,
        protocol,
    };
    let report = download(
        &request,
        &mut tasks,
        Arc::new(NoZipSink),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await;
    assert!(
        matches!(report.end, DownloadEnd::Completed),
        "{:?}",
        report.end
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for task in &tasks {
        assert_eq!(task.status, DownloadStatus::Done, "{}", task.relative_path);
        let a3s_sync::DownloadTaskKind::File {
            compressed,
            compressed_size,
            ref sha1,
            ..
        } = task.kind
        else {
            continue;
        };
        if compressed {
            let zip = task.destination_dir.join(format!("{}.zip", task.name()));
            let bytes = std::fs::read(&zip).expect("zip");
            assert_eq!(bytes.len() as u64, compressed_size);
            assert!(bytes.starts_with(b"PK\x03\x04"));
        } else {
            let actual = a3s_hash::sha1_file(&task.local_path()).expect("sha1");
            assert_eq!(Some(actual), sha1.clone(), "{}", task.relative_path);
        }
    }
}
