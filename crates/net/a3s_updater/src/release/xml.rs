//! Legacy `a3s.xml` metadata (`<nom>` version, optional `<file>` archive name).

use std::path::Path;

use url::Url;

use super::{Release, ReleaseFormat, expand_asset_pattern, is_safe_zip_name};
use crate::UpdateError;

/// Name of the legacy metadata file, remote and in the installation.
pub(crate) const LEGACY_XML_NAME: &str = "a3s.xml";

/// Reads a legacy `a3s.xml` fetched from `source`.
///
/// The version is the text of the first `<nom>` element anywhere in the document and the file
/// name that of the first `<file>` element; without `<nom>` there is no release (`Ok(None)`).
/// Without `<file>`, the name is `asset_pattern` with `{version}` and `{tag}` set to the
/// version and `{target}` to `target`. DTDs are rejected. The release has no SHA-256.
// Mirrors a3sUpdater/dao/XmlDAO.java:38-59
pub fn parse_legacy_xml(
    xml: &str,
    source: &Url,
    asset_pattern: &str,
    target: &str,
) -> Result<Option<Release>, UpdateError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        ..roxmltree::ParsingOptions::default()
    };
    let document = roxmltree::Document::parse_with_options(xml, options)
        .map_err(|e| UpdateError::Manifest(format!("Invalid legacy XML: {e}")))?;
    let Some(version) = element_text(&document, "nom") else {
        return Ok(None);
    };
    let file_name = element_text(&document, "file")
        .unwrap_or_else(|| expand_asset_pattern(asset_pattern, &version, &version, target));
    if !is_safe_zip_name(&file_name) {
        return Err(UpdateError::Manifest(format!(
            "Invalid legacy XML ZIP filename: {file_name}"
        )));
    }
    let download_url = source
        .join(&file_name)
        .map_err(|e| UpdateError::Manifest(format!("Invalid legacy XML download URL: {e}")))?;
    Ok(Some(Release {
        version,
        tag: None,
        file_name,
        download_url,
        sha256: None,
        size: 0,
        format: ReleaseFormat::Xml,
        source: source.to_string(),
    }))
}

/// Reads `<install_dir>/a3s.xml` when it is a regular file.
///
/// The download URL of the result points next to the local file; [`super::resolve_release`]
/// replaces it with the configured legacy URL.
// Mirrors a3sUpdater/dao/XmlDAO.java:28-36
pub fn read_local_xml(
    install_dir: &Path,
    asset_pattern: &str,
    target: &str,
) -> Result<Option<Release>, UpdateError> {
    let path = install_dir.join(LEGACY_XML_NAME);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| UpdateError::io(format!("Could not read {}", path.display()), e))?;
    let absolute = std::path::absolute(&path)
        .map_err(|e| UpdateError::io(format!("Could not resolve {}", path.display()), e))?;
    let source = Url::from_file_path(&absolute).map_err(|()| {
        UpdateError::Manifest(format!(
            "Invalid local metadata path: {}",
            absolute.display()
        ))
    })?;
    parse_legacy_xml(
        &String::from_utf8_lossy(&bytes),
        &source,
        asset_pattern,
        target,
    )
}

/// Trimmed text content of the first element named `name`; blank counts as absent.
// Mirrors a3sUpdater/dao/XmlDAO.java:61-66
fn element_text(document: &roxmltree::Document<'_>, name: &str) -> Option<String> {
    let element = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == name)?;
    let text: String = element
        .descendants()
        .filter(roxmltree::Node::is_text)
        .filter_map(|node| node.text())
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
