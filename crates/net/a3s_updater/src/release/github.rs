//! GitHub Releases API (`releases/latest`) response.

use url::Url;

use super::json::{JsonValue, parse_object};
use super::{
    Release, ReleaseFormat, expand_asset_pattern, is_safe_zip_name, is_sha256_hex, parse_url,
    require_http_url,
};
use crate::UpdateError;

const WHAT: &str = "GitHub release";

/// `Accept` header of GitHub API requests.
// Mirrors a3sUpdater/dao/GitHubReleaseDAO.java:13
pub(crate) const API_ACCEPT: &str = "application/vnd.github+json";

/// `X-GitHub-Api-Version` header of GitHub API requests.
// Mirrors a3sUpdater/dao/GitHubReleaseDAO.java:14
pub(crate) const API_VERSION: &str = "2022-11-28";

/// Checks the API URL and asset pattern before any request.
// Mirrors a3sUpdater/dao/GitHubReleaseDAO.java:18-24
pub(crate) fn check_github_request(api_url: &str, asset_pattern: &str) -> Result<Url, UpdateError> {
    if api_url.trim().is_empty() {
        return Err(UpdateError::Manifest("GitHub API URL is empty.".to_owned()));
    }
    let url = parse_url(api_url, "GitHub API URL")?;
    require_http_url(&url, "GitHub API URL")?;
    if asset_pattern.trim().is_empty() {
        return Err(UpdateError::Manifest(
            "GitHub asset pattern is empty.".to_owned(),
        ));
    }
    Ok(url)
}

/// Turns a GitHub "release" API response into a [`Release`].
///
/// The version is the tag without leading `v`/`V`. The asset is the one whose `name` equals
/// `asset_pattern` with `{version}`, `{tag}` and `{target}` filled in; it must carry a SHA-256
/// `digest` (`sha256:` prefix optional).
// Mirrors a3sUpdater/dao/GitHubReleaseDAO.java:28-67
pub fn parse_github_release(
    json: &str,
    api_url: &Url,
    asset_pattern: &str,
    target: &str,
) -> Result<Release, UpdateError> {
    let release = parse_object(json)?;
    let tag = release
        .string("tag_name", WHAT)?
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| UpdateError::Manifest("GitHub release has no tag_name.".to_owned()))?;
    let version = tag.trim_start_matches(['v', 'V']).to_owned();
    if !crate::is_supported_version(&version) {
        return Err(UpdateError::Manifest(format!(
            "GitHub release tag is not a supported version: {tag}"
        )));
    }

    let expected = expand_asset_pattern(asset_pattern, &version, &tag, target);
    if !is_safe_zip_name(&expected) {
        return Err(UpdateError::Manifest(
            "GitHub asset pattern must resolve to a ZIP filename.".to_owned(),
        ));
    }

    let Some(JsonValue::Array(assets)) = release.get("assets") else {
        return Err(UpdateError::Manifest(
            "GitHub release has no assets.".to_owned(),
        ));
    };
    // Exact (untrimmed) name equality; non-object elements are skipped; the first match wins.
    let asset = assets
        .iter()
        .find_map(|asset| match asset {
            JsonValue::Object(object)
                if matches!(object.get("name"), Some(JsonValue::String(name)) if *name == expected) =>
            {
                Some(object)
            }
            _ => None,
        })
        .ok_or_else(|| {
            UpdateError::Manifest(format!("GitHub release asset not found: {expected}"))
        })?;

    let download = asset
        .string("browser_download_url", WHAT)?
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            UpdateError::Manifest(
                "GitHub release field is missing: browser_download_url".to_owned(),
            )
        })?;
    let digest = asset
        .string("digest", WHAT)?
        .map(|digest| match digest.get(..7) {
            Some(prefix) if prefix.eq_ignore_ascii_case("sha256:") => digest[7..].trim().to_owned(),
            _ => digest,
        });
    let sha256 = match digest {
        Some(digest) if is_sha256_hex(&digest) => digest.to_ascii_lowercase(),
        _ => {
            return Err(UpdateError::Manifest(format!(
                "GitHub release asset has no valid SHA-256 digest: {expected}"
            )));
        }
    };
    let download_url = parse_url(&download, "GitHub asset download URL")?;
    require_http_url(&download_url, "GitHub asset download URL")?;
    let size = asset.size("size", WHAT)?;

    Ok(Release {
        version,
        tag: Some(tag),
        file_name: expected,
        download_url,
        sha256: Some(sha256),
        size,
        format: ReleaseFormat::GitHub,
        source: api_url.to_string(),
    })
}
