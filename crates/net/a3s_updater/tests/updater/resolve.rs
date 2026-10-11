//! Source order and error precedence of `resolve_release` against a local server.

use std::path::Path;

use a3s_updater::{
    CancellationToken, DiagnosticLog, HttpClient, Release, ReleaseFormat, ResolveRequest,
    UpdateConfig, UpdateError, UpdateSource, resolve_release,
};

use crate::support::{HASH, Reply, Server, local_config};

const TARGET: &str = "x86_64-unknown-linux-gnu";

/// Configuration pointing every source at `server`.
fn config(server: &Server, github_enabled: bool) -> UpdateConfig {
    local_config(&format!(
        "[update]\nmanifest_url = \"{}\"\nlegacy_xml_url = \"{}\"\n\
         [github]\nenabled = {github_enabled}\napi_url = \"{}\"\n\
         asset_pattern = \"Arma3Sync-{{version}}.zip\"\n",
        server.url("/updates/a3s.json"),
        server.url("/updates/a3s.xml"),
        server.url("/latest"),
    ))
}

fn github_body(server: &Server) -> String {
    format!(
        r#"{{"tag_name":"v2026.1.1","assets":[{{"name":"Arma3Sync-2026.1.1.zip",
            "browser_download_url":"{}","digest":"sha256:{HASH}","size":1234}}]}}"#,
        server.url("/dl/Arma3Sync-2026.1.1.zip")
    )
}

fn json_body() -> String {
    format!(r#"{{"version":"2026.2.0","file":"Arma3Sync-2026.2.0.zip","sha256":"{HASH}"}}"#)
}

async fn resolve(
    config: &UpdateConfig,
    install_dir: &Path,
    preference: UpdateSource,
) -> Result<Release, UpdateError> {
    let http = HttpClient::new(config).expect("client");
    let request = ResolveRequest {
        install_dir,
        dev: false,
        preference,
        target: TARGET,
    };
    resolve_release(
        config,
        &http,
        &request,
        &DiagnosticLog::disabled(),
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn github_release_with_configured_asset() {
    let server = Server::start().await;
    server.route("/latest", Reply::ok(github_body(&server)));
    let install = tempfile::tempdir().expect("tempdir");
    let release = resolve(
        &config(&server, true),
        install.path(),
        UpdateSource::Configured,
    )
    .await
    .expect("github");
    assert_eq!(release.version, "2026.1.1");
    assert_eq!(release.file_name, "Arma3Sync-2026.1.1.zip");
    assert_eq!(release.sha256.as_deref(), Some(HASH));
    assert_eq!(release.size, 1234);
    assert_eq!(release.format, ReleaseFormat::GitHub);
    assert_eq!(server.hits("/updates/a3s.json"), 0);
}

#[tokio::test]
async fn source_order() {
    let server = Server::start().await;
    let install = tempfile::tempdir().expect("tempdir");
    let enabled = config(&server, true);

    // GitHub failing: the JSON manifest is next.
    server.route("/latest", Reply::status(500));
    server.route("/updates/a3s.json", Reply::ok(json_body()));
    let release = resolve(&enabled, install.path(), UpdateSource::Configured)
        .await
        .expect("json");
    assert_eq!(release.format, ReleaseFormat::Json);
    assert_eq!(server.hits("/latest"), 1);

    // Preference `Manifest` skips GitHub.
    server.route("/latest", Reply::ok(github_body(&server)));
    let release = resolve(&enabled, install.path(), UpdateSource::Manifest)
        .await
        .expect("manifest preference");
    assert_eq!(release.format, ReleaseFormat::Json);
    assert_eq!(server.hits("/latest"), 1);

    // `enabled = false` wins over a GitHub preference.
    let release = resolve(
        &config(&server, false),
        install.path(),
        UpdateSource::GitHub,
    )
    .await
    .expect("github disabled");
    assert_eq!(release.format, ReleaseFormat::Json);
    assert_eq!(server.hits("/latest"), 1);

    // GitHub and JSON failing: the remote legacy XML.
    server.route("/latest", Reply::status(404));
    server.route("/updates/a3s.json", Reply::ok("{"));
    server.route(
        "/updates/a3s.xml",
        Reply::ok("<a3s><nom>1.6.0</nom><file>Old.zip</file></a3s>"),
    );
    let release = resolve(&enabled, install.path(), UpdateSource::Configured)
        .await
        .expect("xml");
    assert_eq!(release.format, ReleaseFormat::Xml);
    assert_eq!(release.download_url, server.url("/updates/Old.zip"));
}

#[tokio::test]
async fn local_xml_fallback_and_error_precedence() {
    let server = Server::start().await;
    let install = tempfile::tempdir().expect("tempdir");
    let enabled = config(&server, true);
    server.route("/latest", Reply::status(500));
    server.route("/updates/a3s.json", Reply::status(503));
    // A remote XML without `<nom>` does not end the lookup.
    server.route("/updates/a3s.xml", Reply::ok("<a3s></a3s>"));

    // No local file: the JSON error wins over the GitHub error.
    let err = resolve(&enabled, install.path(), UpdateSource::Configured)
        .await
        .expect_err("all failing");
    assert!(matches!(err, UpdateError::Http { status: 503 }), "{err:?}");

    // Local `a3s.xml`: its archive is fetched next to the configured legacy URL.
    std::fs::write(
        install.path().join("a3s.xml"),
        "<a3s><nom>1.5.0</nom><file>Local.zip</file></a3s>",
    )
    .expect("write");
    let release = resolve(&enabled, install.path(), UpdateSource::Configured)
        .await
        .expect("local");
    assert_eq!(release.version, "1.5.0");
    assert_eq!(release.download_url, server.url("/updates/Local.zip"));
    assert!(release.source.starts_with("file://"));
    assert_eq!(release.sha256, None);
    std::fs::remove_file(install.path().join("a3s.xml")).expect("remove");

    // Only GitHub configured and failing: the GitHub error.
    let github_only = local_config(&format!(
        "[update]\nmanifest_url = \"\"\nlegacy_xml_url = \"\"\n[github]\napi_url = \"{}\"\n",
        server.url("/latest")
    ));
    let err = resolve(&github_only, install.path(), UpdateSource::Configured)
        .await
        .expect_err("github only");
    assert!(matches!(err, UpdateError::Http { status: 500 }), "{err:?}");

    // Nothing configured at all.
    let nothing = local_config(
        "[update]\nmanifest_url = \"\"\nlegacy_xml_url = \"\"\n[github]\nenabled = false\n",
    );
    let err = resolve(&nothing, install.path(), UpdateSource::Configured)
        .await
        .expect_err("nothing");
    assert!(matches!(err, UpdateError::NoMetadata), "{err:?}");
}

#[tokio::test]
async fn cancellation_ends_the_lookup() {
    let server = Server::start().await;
    let install = tempfile::tempdir().expect("tempdir");
    let config = config(&server, true);
    let http = HttpClient::new(&config).expect("client");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let request = ResolveRequest {
        install_dir: install.path(),
        dev: false,
        preference: UpdateSource::Configured,
        target: TARGET,
    };
    let result = resolve_release(
        &config,
        &http,
        &request,
        &DiagnosticLog::disabled(),
        &cancel,
    )
    .await;
    assert!(matches!(result, Err(UpdateError::Cancelled)), "{result:?}");
    assert_eq!(server.hits("/updates/a3s.json"), 0);
}
