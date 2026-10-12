//! Events upload and the admin metadata fetch against in-process servers.

use crate::net_support;

use a3s_admin::events::{AdminMetadata, EventSelection, EventsDoc, EventsError};
use a3s_admin::{fetch_admin_metadata, upload_events};
use a3s_foundation::error::ErrorKind;
use a3s_sync::RemoteConnection;
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::ProtocolType;

use net_support::{HttpServer, copy_tree, fixtures, repository, servers};

fn sample_doc() -> EventsDoc {
    let mut doc = EventsDoc::new();
    doc.add_event(
        "Night Op",
        "Night operation",
        &EventSelection {
            addon_names: vec![("@ace".to_owned(), false), ("@cba".to_owned(), true)],
            ..EventSelection::default()
        },
    );
    doc
}

#[tokio::test]
async fn events_upload_to_ftp_and_ftps() {
    let doc = sample_doc();
    let mut repo = repository(ProtocolType::Http, None);
    let err = upload_events(
        &doc,
        &repo,
        &TransportOptions::from_protocol(&repo.protocol),
        &TransferCtx::new(),
    )
    .await
    .expect_err("no upload protocol");
    assert!(matches!(err, EventsError::UploadProtocolNotSet));

    for ftps in [false, true] {
        let fixture = servers::ftp::fixture(ftps).await;
        repo.upload_protocol = Some(fixture.protocol.clone());
        let options = TransportOptions::from_protocol(&fixture.protocol);
        upload_events(&doc, &repo, &options, &TransferCtx::new())
            .await
            .expect("upload");
        let uploaded = std::fs::read(fixture.base.join(".a3s/events")).expect("uploaded");
        assert_eq!(uploaded, doc.encode().expect("encode"), "ftps={ftps}");
    }
}

#[tokio::test]
async fn admin_metadata_skips_autoconfig_and_keeps_partial_results() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a3s = dir.path().join(".a3s");
    copy_tree(&fixtures().join("java_repos/wog_ftp"), &a3s);
    let server = HttpServer::serve(dir.path()).await;
    let options = TransportOptions::from_protocol(&server.protocol);
    let ctx = TransferCtx::new();

    let mut conn = RemoteConnection::connect(&server.protocol, &options, &ctx)
        .await
        .expect("connect");
    let mut metadata = AdminMetadata::default();
    fetch_admin_metadata(&mut conn, "WOG", &mut metadata, &ctx)
        .await
        .expect("fetch");
    assert!(metadata.sync.is_some() && metadata.server_info.is_some());
    assert!(metadata.changelogs.is_some());
    let events = metadata.events.clone().expect("events");
    let wog = std::fs::read(a3s.join("events")).expect("fixture");
    assert_eq!(
        events.encode().expect("encode"),
        EventsDoc::decode(&wog)
            .expect("decode")
            .encode()
            .expect("encode")
    );
    assert_eq!(
        server.requests("GET"),
        [
            "/.a3s/sync",
            "/.a3s/serverinfo",
            "/.a3s/changelogs",
            "/.a3s/events"
        ]
    );

    server
        .state
        .fail
        .lock()
        .insert("/.a3s/changelogs".to_owned(), 404);
    let err = fetch_admin_metadata(&mut conn, "WOG", &mut metadata, &ctx)
        .await
        .expect_err("changelogs missing");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(metadata.sync.is_some() && metadata.server_info.is_some());
    assert!(metadata.changelogs.is_none());
    assert!(metadata.events.is_some());
}
