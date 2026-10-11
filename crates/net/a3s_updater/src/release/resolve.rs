//! Release lookup across the configured sources.

use std::path::Path;

use tokio_util::sync::CancellationToken;
use url::Url;

use super::github::{API_ACCEPT, API_VERSION, check_github_request};
use super::xml::LEGACY_XML_NAME;
use super::{
    Release, UpdateSource, parse_github_release, parse_json_manifest, parse_legacy_xml, parse_url,
    read_local_xml,
};
use crate::config::DEFAULT_BASE_URL;
use crate::{DiagnosticLog, HttpClient, UpdateConfig, UpdateError};

/// What [`resolve_release`] looks for.
#[derive(Clone, Copy, Debug)]
pub struct ResolveRequest<'a> {
    /// Installation directory (for the local `a3s.xml` fallback).
    pub install_dir: &'a Path,

    /// Use the `dev_*` URLs and asset pattern.
    pub dev: bool,
    pub preference: UpdateSource,

    /// Rust target triple of the wanted archive (see [`super::host_target`]).
    pub target: &'a str,
}

/// Finds the latest release.
///
/// Order: GitHub (when enabled and not skipped by the preference), the JSON manifest, the
/// remote legacy XML, then the installation's own `a3s.xml`, whose archive is fetched next to
/// the configured legacy XML URL. A failing source is skipped; when none succeeds the JSON
/// error wins over the GitHub error, else [`UpdateError::NoMetadata`]. Legacy XML failures are
/// never reported. Cancellation stops the lookup at once.
// Mirrors a3sUpdater/service/Service.java:48-68,231-275
pub async fn resolve_release(
    config: &UpdateConfig,
    http: &HttpClient,
    request: &ResolveRequest<'_>,
    diag: &DiagnosticLog,
    cancel: &CancellationToken,
) -> Result<Release, UpdateError> {
    diag.info(&format!(
        "Reading update metadata: dev={}, sourcePreference={:?}",
        request.dev, request.preference
    ));
    match resolve(config, http, request, cancel).await {
        Ok(release) => {
            diag.info(&format!(
                "Update metadata loaded: version={}, file={}, format={:?}, uri={}",
                release.version, release.file_name, release.format, release.download_url
            ));
            Ok(release)
        }
        Err(err) => {
            diag.error("Reading update metadata failed.", Some(&err));
            Err(err)
        }
    }
}

async fn resolve(
    config: &UpdateConfig,
    http: &HttpClient,
    request: &ResolveRequest<'_>,
    cancel: &CancellationToken,
) -> Result<Release, UpdateError> {
    let dev = request.dev;
    let pattern = config.github_asset_pattern(dev);

    let mut github_failure = None;
    if config.github_enabled() && request.preference != UpdateSource::Manifest {
        match read_github(config, http, request, cancel).await {
            Ok(release) => return Ok(release),
            Err(err) => github_failure = Some(step_failed("GitHub", err)?),
        }
    }

    let mut json_failure = None;
    let manifest_url = config.manifest_url(dev);
    if !manifest_url.trim().is_empty() {
        match read_manifest(manifest_url, http, request.target, cancel).await {
            Ok(release) => return Ok(release),
            Err(err) => json_failure = Some(step_failed("JSON manifest", err)?),
        }
    }

    let legacy_url = config.legacy_xml_url(dev);
    if !legacy_url.trim().is_empty() {
        // A document without `<nom>` is a failed step like any other.
        match read_remote_xml(legacy_url, http, pattern, request.target, cancel).await {
            Ok(Some(release)) => return Ok(release),
            Ok(None) => tracing::debug!("legacy XML has no version"),
            Err(err) => {
                step_failed("legacy XML", err)?;
            }
        }
    }

    // Unlike the remote sources, an unreadable local file is reported.
    if let Some(local) = read_local_xml(request.install_dir, pattern, request.target)? {
        // The local file is metadata only; the archive comes from the configured repository.
        let base = if legacy_url.trim().is_empty() {
            format!("{DEFAULT_BASE_URL}/{LEGACY_XML_NAME}")
        } else {
            legacy_url.to_owned()
        };
        let download_url = parse_url(&base, "legacy XML URL")?
            .join(&local.file_name)
            .map_err(|e| UpdateError::Manifest(format!("Invalid legacy download URL: {e}")))?;
        return Ok(Release {
            download_url,
            ..local
        });
    }

    Err(json_failure
        .or(github_failure)
        .unwrap_or(UpdateError::NoMetadata))
}

/// Logs a failed source; cancellation is not a source failure and ends the lookup.
fn step_failed(source: &str, err: UpdateError) -> Result<UpdateError, UpdateError> {
    if matches!(err, UpdateError::Cancelled) {
        return Err(err);
    }
    tracing::debug!(source, error = %err, "update source failed");
    Ok(err)
}

// Mirrors a3sUpdater/dao/GitHubReleaseDAO.java:16-27
async fn read_github(
    config: &UpdateConfig,
    http: &HttpClient,
    request: &ResolveRequest<'_>,
    cancel: &CancellationToken,
) -> Result<Release, UpdateError> {
    let pattern = config.github_asset_pattern(request.dev);
    let api_url = check_github_request(config.github_api_url(request.dev), pattern)?;
    let headers = [
        ("Accept", API_ACCEPT),
        ("X-GitHub-Api-Version", API_VERSION),
    ];
    let json = http.read_text(&api_url, &headers, cancel).await?;
    parse_github_release(&json, &api_url, pattern, request.target)
}

async fn read_manifest(
    manifest_url: &str,
    http: &HttpClient,
    target: &str,
    cancel: &CancellationToken,
) -> Result<Release, UpdateError> {
    let source = parse_url(manifest_url, "manifest URL")?;
    let json = http.read_text(&source, &[], cancel).await?;
    parse_json_manifest(&json, &source, target)
}

async fn read_remote_xml(
    legacy_url: &str,
    http: &HttpClient,
    pattern: &str,
    target: &str,
    cancel: &CancellationToken,
) -> Result<Option<Release>, UpdateError> {
    let source: Url = parse_url(legacy_url, "legacy XML URL")?;
    let xml = http.read_text(&source, &[], cancel).await?;
    parse_legacy_xml(&xml, &source, pattern, target)
}
