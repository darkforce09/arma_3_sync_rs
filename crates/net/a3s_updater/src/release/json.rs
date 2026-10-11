//! Strict JSON documents (duplicate keys rejected at any depth) and the `a3s.json` manifest.

use std::fmt;

use serde::de::{Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};
use url::Url;

use super::{Release, ReleaseFormat, is_safe_zip_name, is_sha256_hex, require_http_url};
use crate::UpdateError;

/// A JSON value whose objects keep their keys in document order and never contain duplicates.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum JsonValue {
    Null,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(String),
    Array(Vec<Self>),
    Object(JsonObject),
}

/// A JSON object without duplicate keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct JsonObject(Vec<(String, JsonValue)>);

impl JsonObject {
    /// The value of `key`; `null` counts as absent.
    pub(crate) fn get(&self, key: &str) -> Option<&JsonValue> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .filter(|v| !matches!(v, JsonValue::Null))
    }

    /// A string field, trimmed; `None` when absent or `null`.
    // Mirrors a3sUpdater/dao/JsonManifestDAO.java:65-70
    pub(crate) fn string(&self, key: &str, what: &str) -> Result<Option<String>, UpdateError> {
        match self.get(key) {
            None => Ok(None),
            Some(JsonValue::String(text)) => Ok(Some(text.trim().to_owned())),
            Some(_) => Err(UpdateError::Manifest(format!(
                "{what} field must be a string: {key}"
            ))),
        }
    }

    /// The first present string field among `keys`.
    fn string_alias(&self, keys: &[&str], what: &str) -> Result<Option<String>, UpdateError> {
        for key in keys {
            if let Some(value) = self.string(key, what)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// A non-negative integer field (absent = 0). Accepts integers in `0..=i64::MAX` and floats
    /// with a zero fractional part in that range (`1.0`, `1e3`).
    // Mirrors a3sUpdater/dao/JsonManifestDAO.java:72-83
    pub(crate) fn size(&self, key: &str, what: &str) -> Result<u64, UpdateError> {
        let invalid = || {
            UpdateError::Manifest(format!(
                "{what} field must be a non-negative integer: {key}"
            ))
        };
        // 2^63 as f64; every float below it with no fractional part fits in i64.
        const LIMIT: f64 = (1u64 << 63) as f64;
        match self.get(key) {
            None => Ok(0),
            Some(JsonValue::Unsigned(n)) if i64::try_from(*n).is_ok() => Ok(*n),
            Some(JsonValue::Float(f)) if f.fract() == 0.0 && (0.0..LIMIT).contains(f) => {
                // Exact: the value is an integer below 2^63.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(*f as u64)
            }
            Some(JsonValue::Unsigned(_) | JsonValue::Signed(_) | JsonValue::Float(_)) => {
                Err(invalid())
            }
            Some(_) => Err(UpdateError::Manifest(format!(
                "{what} field must be a number: {key}"
            ))),
        }
    }
}

/// Parses a JSON document whose top level must be an object.
// Mirrors a3sUpdater/dao/JsonManifestDAO.java:19-28
pub(crate) fn parse_object(json: &str) -> Result<JsonObject, UpdateError> {
    match serde_json::from_str::<JsonValue>(json) {
        Ok(JsonValue::Object(object)) => Ok(object),
        Ok(_) => Err(UpdateError::Manifest(
            "JSON document must be an object.".to_owned(),
        )),
        Err(e) => Err(UpdateError::Manifest(format!("Invalid JSON: {e}"))),
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = JsonValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> Result<JsonValue, E> {
        Ok(JsonValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<JsonValue, E> {
        Ok(JsonValue::Bool(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<JsonValue, E> {
        Ok(JsonValue::Unsigned(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<JsonValue, E> {
        Ok(JsonValue::Signed(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<JsonValue, E> {
        Ok(JsonValue::Float(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<JsonValue, E> {
        Ok(JsonValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<JsonValue, E> {
        Ok(JsonValue::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JsonValue, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element()? {
            values.push(value);
        }
        Ok(JsonValue::Array(values))
    }

    // Mirrors a3sUpdater/dao/JsonManifestDAO.java:136-153
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonValue, A::Error> {
        let mut entries: Vec<(String, JsonValue)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if entries.iter().any(|(k, _)| *k == key) {
                return Err(A::Error::custom(format!(
                    "duplicate JSON object key '{key}'"
                )));
            }
            let value = map.next_value()?;
            entries.push((key, value));
        }
        Ok(JsonValue::Object(JsonObject(entries)))
    }
}

/// Reads an `a3s.json` manifest fetched from `source`.
///
/// Fields: `version` (or `nom`), `file` (or `filename`), `downloadUrl` (or `url`), `sha256` (or
/// `hash`) and `size`. Without a file name, the last path segment of the download URL is used;
/// without a download URL, the file name is resolved against `source`. An optional `targets`
/// object maps target triples to their own `file`/`downloadUrl`/`sha256`/`size`; when present,
/// the entry for `target` is used.
// Mirrors a3sUpdater/dao/JsonManifestDAO.java:30-63
pub fn parse_json_manifest(json: &str, source: &Url, target: &str) -> Result<Release, UpdateError> {
    const WHAT: &str = "JSON";
    let root = parse_object(json)?;

    let version = root.string_alias(&["version", "nom"], WHAT)?;
    let version = match version {
        Some(version) if crate::is_supported_version(&version) => version,
        _ => {
            return Err(UpdateError::Manifest(
                "JSON manifest has no valid version.".to_owned(),
            ));
        }
    };

    let asset = match root.get("targets") {
        None => &root,
        Some(JsonValue::Object(targets)) => match targets.get(target) {
            Some(JsonValue::Object(asset)) => asset,
            Some(_) => {
                return Err(UpdateError::Manifest(format!(
                    "JSON manifest target entry must be an object: {target}"
                )));
            }
            None => return Err(UpdateError::NoTarget(target.to_owned())),
        },
        Some(_) => {
            return Err(UpdateError::Manifest(
                "JSON manifest field must be an object: targets".to_owned(),
            ));
        }
    };

    let file = asset.string_alias(&["file", "filename"], WHAT)?;
    let download = asset
        .string_alias(&["downloadUrl", "url"], WHAT)?
        .filter(|d| !d.is_empty());
    let file_name = match (file, &download) {
        (Some(file), _) => file,
        (None, Some(download)) => file_name_of(source, download)?,
        (None, None) => String::new(),
    };
    if !is_safe_zip_name(&file_name) {
        return Err(UpdateError::Manifest(
            "JSON manifest has no valid ZIP filename.".to_owned(),
        ));
    }

    let reference = download.as_deref().unwrap_or(&file_name);
    let download_url = source.join(reference).map_err(|e| {
        UpdateError::Manifest(format!("JSON manifest has an invalid download URL: {e}"))
    })?;
    require_http_url(&download_url, "download URL")?;

    let sha256 = match asset.string_alias(&["sha256", "hash"], WHAT)? {
        Some(hash) if is_sha256_hex(&hash) => hash.to_ascii_lowercase(),
        _ => {
            return Err(UpdateError::Manifest(
                "JSON manifest must contain a valid SHA-256 hash.".to_owned(),
            ));
        }
    };
    let size = asset.size("size", WHAT)?;

    Ok(Release {
        version,
        tag: None,
        file_name,
        download_url,
        sha256: Some(sha256),
        size,
        format: ReleaseFormat::Json,
        source: source.to_string(),
    })
}

/// The percent-decoded last path segment of `download` resolved against `source`.
// Mirrors a3sUpdater/dao/JsonManifestDAO.java:85-89
fn file_name_of(source: &Url, download: &str) -> Result<String, UpdateError> {
    let url = source.join(download).map_err(|e| {
        UpdateError::Manifest(format!("JSON manifest has an invalid download URL: {e}"))
    })?;
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .ok_or_else(|| UpdateError::Manifest("Download URL has no filename.".to_owned()))?;
    Ok(percent_decode(segment))
}

/// Decodes `%XX` escapes; invalid UTF-8 is replaced.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escaped {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
