//! Live, read-only tests against the public WOG repository (`.a3s/*` only, ~125 KB).
//!
//! Run with `cargo test -p a3s_sync -- --ignored --test-threads=1`.

use a3s_sync::remote::RemoteConnection;
use a3s_sync::status::determine_sync_status;
use a3s_sync::{EventSender, PartialTransferSupport, PlanTree, RemoteMetadata};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::constants::RepositoryStatus;
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
async fn live_wog_check() {
    let protocol = wog();
    let options = TransportOptions::from_protocol(&protocol);
    let ctx = TransferCtx::new();
    let mut connection = RemoteConnection::connect(&protocol, &options, &ctx)
        .await
        .expect("connect");

    let mut metadata = RemoteMetadata::default();
    connection
        .check_repository("WOG", &mut metadata, &ctx, &EventSender::noop())
        .await
        .expect("check");
    let server_info = metadata.server_info.as_deref().expect("serverinfo");
    let sync = metadata.sync.as_deref().expect("sync");
    assert!(metadata.changelogs.is_some());
    assert!(metadata.events.is_some());
    assert!(metadata.auto_config.is_some());
    assert!(server_info.revision >= 55, "{}", server_info.revision);
    assert!(metadata.require_for_sync(None).is_ok());

    assert_eq!(
        PlanTree::from_sync_tree(sync).leaf_count(),
        sync.leaf_count()
    );
    assert_ne!(
        determine_sync_status(-1, Some(server_info), metadata.changelogs.as_deref()),
        RepositoryStatus::Ok
    );

    assert_eq!(
        connection
            .probe_partial_transfer(&ctx)
            .await
            .expect("probe"),
        PartialTransferSupport::NotApplicable
    );

    let missing = connection
        .transport_mut()
        .fetch(".a3s/nope", &ctx)
        .await
        .expect_err("missing file");
    assert!(
        matches!(missing, a3s_transport::Error::NotFound(_)),
        "{missing:?}"
    );
    connection.close().await;
}
