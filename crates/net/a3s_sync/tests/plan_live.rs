//! Live, read-only planning against the public WOG repository (`.a3s/*` only, ~125 KB).
//!
//! Run with `cargo test -p a3s_sync -- --ignored --test-threads=1`.

use a3s_sync::completion::determine_completion;
use a3s_sync::plan::{NoAddons, plan_repository, select_all_except_root};
use a3s_sync::remote::check_repository;
use a3s_sync::{EventSender, PlanNode, PlanSummary, RemoteMetadata, SyncOptions};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::{Protocol, ProtocolType};

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
async fn live_wog_plan_and_completion() {
    let protocol = wog();
    let options = TransportOptions::from_protocol(&protocol);
    let ctx = TransferCtx::new();
    let mut metadata = RemoteMetadata::default();
    check_repository(
        "WOG",
        &protocol,
        &options,
        &mut metadata,
        &ctx,
        &EventSender::noop(),
    )
    .await
    .expect("check");
    metadata.require_for_sync(None).expect("sync metadata");

    let base = tempfile::tempdir().expect("tempdir");
    let sync_options = SyncOptions {
        repository_name: "WOG".to_owned(),
        event_name: None,
        destination: base.path().to_path_buf(),
        auto_discover: false,
        exact_match: false,
        hidden_folder_paths: Default::default(),
        client_connections: 1,
        max_download_speed: 0,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
    };
    let planned_metadata = metadata.clone();
    let cancel = ctx.cancel.clone();
    let mut tree = tokio::task::spawn_blocking(move || {
        plan_repository(
            &planned_metadata,
            &sync_options,
            &NoAddons,
            &mut a3s_hash::Sha1Cache::default(),
            &cancel,
            &EventSender::noop(),
        )
    })
    .await
    .expect("join")
    .expect("plan");

    let server_info = metadata.server_info.as_deref().expect("serverinfo");
    // FTP: no `.zsync` files, no network access.
    let stats = determine_completion(
        &mut tree,
        &protocol,
        &options,
        server_info,
        &ctx,
        &EventSender::noop(),
    )
    .await
    .expect("completion");
    assert_eq!(stats.checked, 0);

    let mut nodes = 0;
    tree.walk(&mut |path, node| {
        nodes += 1;
        assert!(node.updated(), "{path}");
        if let PlanNode::Leaf(l) = node {
            assert!(l.complete == 0.0, "{path}");
        }
    });
    select_all_except_root(&mut tree);
    let summary = PlanSummary::compute(&tree);
    assert_eq!(summary.download_tasks.len(), nodes);
}
