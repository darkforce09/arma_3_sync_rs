//! End-to-end planning of the WOG golden fixture into a local folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_repo_files::read_a3s;
use a3s_sync::plan::{NoAddons, plan_repository, select_all_except_root};
use a3s_sync::{
    EventSender, Phase, PlanError, PlanNode, PlanSummary, PlanTree, RemoteMetadata, SyncEvent,
    SyncOptions,
};
use a3s_transport::CancellationToken;
use a3s_types::{Event, Events, ServerInfo, SyncTreeDirectory};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_repos/wog_ftp")
}

fn metadata() -> RemoteMetadata {
    let sync: SyncTreeDirectory = read_a3s(&fixture_dir().join("sync")).expect("sync decodes");
    let server_info: ServerInfo =
        read_a3s(&fixture_dir().join("serverinfo")).expect("serverinfo decodes");
    RemoteMetadata {
        sync: Some(Arc::new(sync)),
        server_info: Some(Arc::new(server_info)),
        ..Default::default()
    }
}

fn options(destination: &Path, exact_match: bool) -> SyncOptions {
    SyncOptions {
        repository_name: "WOG".to_owned(),
        event_name: None,
        destination: destination.to_path_buf(),
        auto_discover: true,
        exact_match,
        hidden_folder_paths: Default::default(),
        client_connections: 1,
        max_download_speed: 0,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
    }
}

fn plan(
    metadata: &RemoteMetadata,
    options: &SyncOptions,
    events: &EventSender,
) -> Result<PlanTree, PlanError> {
    plan_repository(
        metadata,
        options,
        &NoAddons,
        &mut a3s_hash::Sha1Cache::default(),
        &CancellationToken::new(),
        events,
    )
}

fn non_root_count(tree: &PlanTree) -> usize {
    let mut n = 0;
    tree.walk(&mut |_, _| n += 1);
    n
}

#[test]
fn a6_wog_plan_into_empty_folder() {
    let metadata = metadata();
    let sync = metadata.sync.as_deref().expect("sync");
    let base = tempfile::tempdir().expect("tempdir");
    let (sender, rx) = EventSender::channel();
    let mut tree = plan(&metadata, &options(base.path(), false), &sender).expect("plan");

    assert_eq!(tree.leaf_count(), sync.leaf_count());
    assert!(tree.root.changed);
    let mut expected_size = 0;
    tree.walk(&mut |path, node| {
        assert!(node.updated(), "{path}");
        assert!(!node.deleted(), "{path}");
        assert!(node.destination_path().is_some(), "{path}");
        if let PlanNode::Leaf(l) = node {
            assert_eq!(l.local_sha1, None, "{path}");
            expected_size += if l.compressed {
                l.compressed_size
            } else {
                l.size
            };
        }
    });
    // Nothing to hash: no progress.
    let events: Vec<SyncEvent> = rx.try_iter().collect();
    assert_eq!(events, [SyncEvent::Phase(Phase::CheckingFiles)]);
    // The remote tree is untouched.
    assert_eq!(metadata.sync.as_deref(), Some(sync));

    select_all_except_root(&mut tree);
    assert!(!tree.root.selected);
    let summary = PlanSummary::compute(&tree);
    let nodes = non_root_count(&tree);
    let mut walk_order = Vec::new();
    tree.walk(&mut |path, _| walk_order.push(path.to_owned()));
    let task_order: Vec<String> = summary
        .download_tasks
        .iter()
        .map(|t| t.relative_path.clone())
        .collect();
    assert_eq!(task_order, walk_order);
    assert_eq!(summary.download_tasks.len(), nodes);
    assert_eq!(summary.total_download_size, expected_size);
    assert_eq!(summary.files_updated, nodes);
    assert_eq!(summary.files_selected, nodes);
    assert_eq!(summary.files_deleted, 0);
    assert!(summary.delete_tasks.is_empty());
}

#[test]
fn a6_wog_plan_exact_match_stale_folder() {
    let metadata = metadata();
    let base = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(base.path().join("@stale")).expect("mkdir");
    std::fs::write(base.path().join("@stale/x.txt"), b"x").expect("write");

    // Without exact match the root is not scanned.
    let tree = plan(
        &metadata,
        &options(base.path(), false),
        &EventSender::noop(),
    )
    .expect("plan");
    assert!(tree.get("@stale").is_none());

    let mut tree =
        plan(&metadata, &options(base.path(), true), &EventSender::noop()).expect("plan");
    let mut deleted = Vec::new();
    tree.walk(&mut |path, node| {
        if node.deleted() {
            deleted.push(path.to_owned());
        }
    });
    assert_eq!(deleted, ["@stale", "@stale/x.txt"]);
    select_all_except_root(&mut tree);
    let summary = PlanSummary::compute(&tree);
    assert_eq!(summary.files_deleted, 2);
    assert_eq!(summary.delete_tasks.len(), 2);
    assert_eq!(
        summary.delete_tasks[0].local_path,
        base.path().join("@stale")
    );
    assert!(summary.delete_tasks[0].is_directory);
    assert_eq!(
        summary.delete_tasks[1].local_path,
        base.path().join("@stale/x.txt")
    );
}

#[test]
fn a6_wog_plan_with_event() {
    let mut metadata = metadata();
    let sync = metadata.sync.as_deref().expect("sync");
    let addons: Vec<String> = sync
        .directories()
        .filter(|d| d.mark_as_addon)
        .take(2)
        .map(|d| d.name.clone())
        .collect();
    assert_eq!(addons.len(), 2, "the fixture has root addons");
    metadata.events = Some(Arc::new(Events {
        list: vec![Event {
            name: "synthetic".to_owned(),
            addon_names: addons.iter().map(|a| (a.clone(), false)).collect(),
            ..Default::default()
        }],
    }));
    let base = tempfile::tempdir().expect("tempdir");
    let mut opts = options(base.path(), false);
    opts.event_name = Some("synthetic".to_owned());
    let tree = plan(&metadata, &opts, &EventSender::noop()).expect("plan");
    let names: Vec<&str> = tree.root.children.iter().map(PlanNode::name).collect();
    assert_eq!(names, addons);

    opts.event_name = Some("unknown".to_owned());
    assert!(matches!(
        plan(&metadata, &opts, &EventSender::noop()),
        Err(PlanError::EventNotFound { .. })
    ));
}
