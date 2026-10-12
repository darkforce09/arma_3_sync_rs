//! Repository content check against in-process HTTP and FTP servers.

use crate::net_support;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use a3s_admin::{CheckError, ContentCheckReport, check_repository_content};
use a3s_foundation::error::ErrorKind;
use a3s_repo_files::A3sFile;
use a3s_sync::{Outcome, Phase, SyncEvent};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::local::Repository;
use a3s_types::{ProtocolType, ServerInfo, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

use net_support::{Events, HttpServer, built_repo, copy_tree, repository};

/// Number of leaves in the sample repositories' sync tree.
const LEAVES: usize = 10;

async fn check(
    repository: &Repository,
    ctx: &TransferCtx,
    events: &Events,
) -> Result<ContentCheckReport, CheckError> {
    let options = TransportOptions::from_protocol(&repository.protocol);
    check_repository_content(repository, &options, ctx, &events.sender()).await
}

fn missing_paths(report: &ContentCheckReport) -> Vec<String> {
    report
        .missing
        .iter()
        .map(a3s_sync::RemoteFile::transport_path)
        .collect()
}

/// A served copy of a sample repository; `uploaded_server_info` replaces its serverinfo by the
/// one Java uploaded (with the upload's `compressedPboFilesOnly`).
fn served_copy(variant: &str, uploaded_server_info: bool, root: &Path) {
    copy_tree(&built_repo(variant, "1_initial"), root);
    if uploaded_server_info {
        let uploaded = built_repo(variant, "1_initial").join("../uploaded/serverinfo");
        std::fs::copy(uploaded, root.join(".a3s/serverinfo")).expect("copy serverinfo");
    }
}

#[tokio::test]
async fn reports_missing_files_in_plan_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("root");
    served_copy("http_plain", false, &root);
    let server = HttpServer::serve(&root).await;
    let mut repo = repository(ProtocolType::Http, None);
    repo.protocol = server.protocol.clone();

    let events = Events::default();
    let report = check(&repo, &TransferCtx::new(), &events)
        .await
        .expect("check");
    assert!(report.is_synchronized());
    // Every leaf and its `.zsync`.
    assert_eq!(report.checked, 2 * LEAVES);
    assert!(report.metadata.sync.is_some() && report.metadata.auto_config.is_some());
    assert!(report.metadata.events.is_none());
    let all = events.all();
    assert_eq!(all[0], SyncEvent::Phase(Phase::CheckingRemoteFiles));
    let progress: Vec<u8> = all
        .iter()
        .filter_map(|e| match e {
            SyncEvent::Progress { percent, .. } => Some(*percent),
            _ => None,
        })
        .collect();
    assert!(progress.windows(2).all(|w| w[0] < w[1]), "{progress:?}");
    assert_eq!(progress.last(), Some(&100));
    assert_eq!(all.last(), Some(&SyncEvent::Finished(Outcome::Success)));
    // The four metadata files, never the events.
    assert_eq!(
        server.requests("GET"),
        [
            "/.a3s/sync",
            "/.a3s/serverinfo",
            "/.a3s/changelogs",
            "/.a3s/autoconfig"
        ]
    );

    std::fs::remove_file(root.join("@mod_b/Addons/mod_b.ebo")).expect("remove");
    std::fs::remove_file(root.join("@Mod_A/addons/mod_a.pbo.zsync")).expect("remove");
    let events = Events::default();
    let report = check(&repo, &TransferCtx::new(), &events)
        .await
        .expect("check");
    assert!(!report.is_synchronized());
    assert_eq!(
        missing_paths(&report),
        ["@Mod_A/addons/mod_a.pbo.zsync", "@mod_b/Addons/mod_b.ebo"]
    );
    assert_eq!(report.error_messages().len(), 2);
    assert_eq!(report.console_lines("Sample").len(), 3);
    let mut failed: Vec<String> = events
        .all()
        .into_iter()
        .filter_map(|e| match e {
            SyncEvent::FileFailed {
                relative_path,
                error,
            } => {
                assert_eq!(error.kind, ErrorKind::NotFound);
                Some(relative_path)
            }
            _ => None,
        })
        .collect();
    failed.sort();
    assert_eq!(failed, missing_paths(&report));
    match events.all().last() {
        Some(SyncEvent::Finished(Outcome::CompletedWithErrors { errors })) => {
            assert_eq!(errors.len(), 2);
        }
        other => panic!("{other:?}"),
    }
}

/// Only-zip repositories expect only `.zip` for compressed leaves; `.zsync` files only for HTTP.
#[tokio::test]
async fn expected_files_follow_server_info_and_download_protocol() {
    let fixture = net_support::servers::ftp::fixture(false).await;
    served_copy("http_zip", true, &fixture.base);
    let base = &fixture.base;
    std::fs::remove_file(base.join("@Mod_A/addons/mod_a.pbo")).expect("remove");
    std::fs::remove_file(base.join("@Mod_A/addons/mod_a.pbo.zsync")).expect("remove");
    std::fs::remove_file(base.join("@mod_b/Addons/mod_b.ebo.zip")).expect("remove");
    let mut repo = repository(ProtocolType::Ftp, None);
    repo.protocol = fixture.protocol.clone();

    let report = check(&repo, &TransferCtx::new(), &Events::default())
        .await
        .expect("check");
    assert_eq!(missing_paths(&report), ["@mod_b/Addons/mod_b.ebo.zip"]);
    // Three compressed leaves are only their `.zip`; no `.zsync` over FTP.
    assert_eq!(report.checked, LEAVES);
}

#[tokio::test]
async fn first_error_aborts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("root");
    served_copy("http_plain", false, &root);
    let server = HttpServer::serve(&root).await;
    let mut repo = repository(ProtocolType::Http, None);
    repo.protocol = server.protocol.clone();

    server
        .state
        .fail
        .lock()
        .insert("/.a3s/changelogs".to_owned(), 404);
    let err = check(&repo, &TransferCtx::new(), &Events::default())
        .await
        .expect_err("changelogs missing");
    match &err {
        CheckError::Remote(remote) => assert_eq!(remote.kind(), ErrorKind::NotFound),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        server.requests("GET"),
        ["/.a3s/sync", "/.a3s/serverinfo", "/.a3s/changelogs"]
    );

    server.state.fail.lock().clear();
    server
        .state
        .fail
        .lock()
        .insert("/@Mod_A/addons/empty.txt".to_owned(), 500);
    *server.state.head_delay.lock() = Duration::from_millis(50);
    let events = Events::default();
    let err = check(&repo, &TransferCtx::new(), &events)
        .await
        .expect_err("server error");
    match &err {
        CheckError::Exists { relative_path, .. } => {
            assert_eq!(relative_path, "/@Mod_A/addons/empty.txt");
        }
        other => panic!("{other:?}"),
    }
    assert!(server.requests("HEAD").len() < 2 * LEAVES);
    assert!(matches!(
        events.all().last(),
        Some(SyncEvent::Finished(Outcome::Failed { .. }))
    ));
}

/// A repository with a single compressed leaf and an only-zip server info.
fn single_file_repo(root: &Path) {
    let mut addon = SyncTreeDirectory::new("@x");
    addon.add_child(SyncTreeNode::Leaf(SyncTreeLeaf {
        name: "x.pbo".to_owned(),
        sha1: Some("1".to_owned()),
        size: 1,
        compressed_size: 1,
        compressed: true,
        updated: false,
        deleted: false,
    }));
    let mut sync = SyncTreeDirectory::new_root();
    sync.add_child(SyncTreeNode::Directory(addon));
    let a3s = root.join(".a3s");
    std::fs::create_dir_all(root.join("@x")).expect("mkdir");
    std::fs::write(root.join("@x/x.pbo.zip"), b"z").expect("write");
    a3s_repo_files::write_a3s(&a3s.join("sync"), &sync).expect("sync");
    let server_info = ServerInfo {
        compressed_pbo_files_only: true,
        ..ServerInfo::default()
    };
    a3s_repo_files::write_a3s(&a3s.join("serverinfo"), &server_info).expect("serverinfo");
    let sample = built_repo("http_plain", "1_initial").join(".a3s");
    for file in [A3sFile::Changelogs, A3sFile::AutoConfig] {
        std::fs::copy(sample.join(file.file_name()), a3s.join(file.file_name())).expect("copy");
    }
}

#[tokio::test]
async fn connections_are_limited_and_cancel_is_prompt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("root");
    served_copy("http_plain", false, &root);
    let server = HttpServer::serve(&root).await;
    *server.state.head_delay.lock() = Duration::from_millis(50);
    let mut repo = repository(ProtocolType::Http, None);
    repo.protocol = server.protocol.clone();

    for (raw, limit) in [(3, 3), (9, 4), (0, 4)] {
        repo.number_of_repository_check_connections = raw;
        server.state.max_in_flight.store(0, Ordering::SeqCst);
        check(&repo, &TransferCtx::new(), &Events::default())
            .await
            .expect("check");
        let concurrent = server.state.max_in_flight.load(Ordering::SeqCst);
        assert!(concurrent > 1 && concurrent <= limit, "{raw}: {concurrent}");
    }

    *server.state.head_delay.lock() = Duration::from_millis(300);
    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    let events = Events::default();
    let started = Instant::now();
    let err = check(&repo, &ctx, &events).await.expect_err("cancelled");
    assert!(matches!(err, CheckError::Cancelled), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        events.all().last(),
        Some(&SyncEvent::Finished(Outcome::Cancelled))
    );

    let single = dir.path().join("single");
    single_file_repo(&single);
    let server = HttpServer::serve(&single).await;
    *server.state.head_delay.lock() = Duration::from_millis(50);
    repo.protocol = server.protocol.clone();
    repo.number_of_repository_check_connections = 4;
    let report = check(&repo, &TransferCtx::new(), &Events::default())
        .await
        .expect("check");
    assert_eq!(report.checked, 1);
    assert!(report.is_synchronized());
    assert!(server.state.max_in_flight.load(Ordering::SeqCst) <= 1);
    assert_eq!(server.requests("HEAD"), ["/@x/x.pbo.zip"]);
}
