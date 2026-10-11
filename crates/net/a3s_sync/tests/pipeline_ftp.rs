//! End-to-end synchronization of a Java-built compressed repository over a local FTP server.

mod download_support;
mod pipeline_support;

use a3s_sync::pipeline::SyncEnd;
use a3s_sync::{EventSender, Outcome, SyncSession, sync_repository};
use a3s_transport::CancellationToken;
use a3s_types::constants::RepositoryStatus;
use download_support::{FtpServer, drain, sha1};
use pipeline_support::{
    check_request, files_below, finished, fixture, job, leaves, read_sync, repository, serve, store,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn java_built_compressed_repository_over_ftp() {
    let server = FtpServer::start().await;
    serve(&server.root, &fixture("ftp_zip", "1_initial"));
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest = dest_dir.path();
    let store = store(repository(server.protocol(), dest, true));

    let (events, rx) = EventSender::channel();
    let report = sync_repository(
        job(server.protocol()),
        store.clone(),
        &CancellationToken::new(),
        &events,
    )
    .await;
    assert_eq!(
        report.end,
        SyncEnd::Finished(Outcome::Success),
        "{report:?}"
    );
    assert_eq!(finished(&drain(&rx)), [&Outcome::Success]);
    assert_eq!(report.status, Some(RepositoryStatus::Ok));
    assert_eq!(report.remaining_downloads, Some(0));
    let leaves = leaves(&read_sync(&server.root));
    for (path, leaf) in &leaves {
        assert_eq!(Some(sha1(&dest.join(path))), leaf.sha1, "{path}");
    }
    assert!(
        files_below(dest)
            .iter()
            .filter(|f| f.ends_with(".zip") || f.ends_with(".part"))
            .all(|f| leaves.iter().any(|(path, _)| path == f))
    );

    // Nothing left to do.
    let mut session = SyncSession::check(
        check_request(server.protocol()),
        store.clone(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");
    session.select_all();
    assert!(session.summary().download_tasks.is_empty());
    assert!(session.summary().delete_tasks.is_empty());
}
