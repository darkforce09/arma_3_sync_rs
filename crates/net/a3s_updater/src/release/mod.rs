//! Release metadata: one normalized [`Release`] from the GitHub Releases API, the JSON manifest
//! (`a3s.json`) or the legacy `a3s.xml`.

mod github;
mod json;
mod resolve;
mod xml;

use url::Url;

pub use github::parse_github_release;
pub use json::parse_json_manifest;
pub use resolve::{ResolveRequest, resolve_release};
pub use xml::{parse_legacy_xml, read_local_xml};

use crate::UpdateError;

/// Which release source the caller prefers.
// Mirrors a3sUpdater/model/UpdateSource.java
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdateSource {
    /// GitHub first when enabled, then the manifest and the legacy XML.
    #[default]
    Configured,

    /// Same order as [`Self::Configured`]; a configuration that disables GitHub still wins.
    GitHub,

    /// Skip GitHub.
    Manifest,
}

/// The source format a [`Release`] was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseFormat {
    Json,
    GitHub,
    Xml,
}

/// Normalized update metadata, independent of its source format.
// Mirrors a3sUpdater/model/UpdateManifest.java
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,

    /// The release tag (GitHub only).
    pub tag: Option<String>,

    /// Archive file name: a plain `.zip` name without separators.
    pub file_name: String,
    pub download_url: Url,

    /// Lowercase hex SHA-256 of the archive; `None` only for legacy XML releases.
    pub sha256: Option<String>,

    /// Archive size in bytes; 0 when unknown.
    pub size: u64,
    pub format: ReleaseFormat,

    /// Where the metadata was read from (URL).
    pub source: String,
}

/// The Rust target triple of this build when release archives exist for it.
pub fn host_target() -> Option<&'static str> {
    if cfg!(all(
        target_arch = "x86_64",
        target_os = "linux",
        target_env = "gnu"
    )) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(
        target_arch = "x86_64",
        target_os = "windows",
        target_env = "msvc"
    )) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        Some("x86_64-apple-darwin")
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        Some("aarch64-apple-darwin")
    } else {
        None
    }
}

/// Fills an asset name pattern: `{version}`, then `{tag}`, then `{target}`.
// Mirrors a3sUpdater/dao/GitHubReleaseDAO.java:31
pub(crate) fn expand_asset_pattern(
    pattern: &str,
    version: &str,
    tag: &str,
    target: &str,
) -> String {
    pattern
        .replace("{version}", version)
        .replace("{tag}", tag)
        .replace("{target}", target)
}

/// A non-blank `.zip` file name without path separators that is not `.` or `..`.
// Mirrors a3sUpdater/dao/JsonManifestDAO.java:91-94
pub(crate) fn is_safe_zip_name(name: &str) -> bool {
    !name.trim().is_empty()
        && !name.contains(['/', '\\'])
        && name != "."
        && name != ".."
        && name
            .get(name.len().saturating_sub(4)..)
            .is_some_and(|ext| ext.eq_ignore_ascii_case(".zip"))
}

/// Requires an `http`/`https` URL with a host and without user information.
// Mirrors a3sUpdater/dao/JsonManifestDAO.java:100-106
pub(crate) fn require_http_url(url: &Url, field: &str) -> Result<(), UpdateError> {
    let ok = matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some_and(|host| !host.is_empty())
        && url.username().is_empty()
        && url.password().is_none();
    if ok {
        Ok(())
    } else {
        Err(UpdateError::Manifest(format!("Invalid HTTP(S) {field}.")))
    }
}

/// Parses an absolute URL from configuration or metadata.
pub(crate) fn parse_url(text: &str, field: &str) -> Result<Url, UpdateError> {
    Url::parse(text.trim())
        .map_err(|e| UpdateError::Manifest(format!("Invalid {field} '{text}': {e}")))
}

/// Whether `text` is 64 hexadecimal digits.
pub(crate) fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}
