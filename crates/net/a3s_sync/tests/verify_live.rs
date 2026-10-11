//! Adversarial live checks against the public WOG repository (read-only, `.a3s/*` only).
//!
//! Run with `cargo test -p a3s_sync --test verify_live -- --ignored --test-threads=1`.

use a3s_foundation::error::ErrorKind;
use a3s_sync::remote::import_autoconfig;
use a3s_transport::TransferCtx;
use a3s_types::{Protocol, ProtocolType};

fn wog(path: &str) -> Protocol {
    Protocol::new(
        ProtocolType::Ftp,
        format!("88.99.69.243{path}"),
        "21",
        "anonymous",
        "",
        false,
    )
}

#[tokio::test]
#[ignore = "network"]
async fn verify_live_import_autoconfig_wog() {
    let protocol = wog("/Repo/WOG/.a3s");
    let options =
        a3s_transport::proxy::transport_options(&protocol, &a3s_types::local::Proxy::default())
            .expect("no proxy");
    let ctx = TransferCtx::new();
    let auto_config = import_autoconfig(&protocol, &options, &ctx)
        .await
        .expect("import autoconfig");
    let remote = auto_config.protocol.as_ref().expect("autoconfig protocol");
    assert_eq!(remote.protocol_type, ProtocolType::Ftp);
    assert!(
        remote.url.contains("88.99.69.243"),
        "autoconfig url {:?}",
        remote.url
    );

    let missing = wog("/Repo/WOG/.a3s-missing");
    let err = import_autoconfig(&missing, &options, &ctx)
        .await
        .expect_err("missing autoconfig");
    assert_eq!(err.kind(), ErrorKind::NotFound, "{err:?}");
}
