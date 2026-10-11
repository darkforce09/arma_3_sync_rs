//! The versioned TOML envelope (`format = 1` plus one payload table) and atomic writes.

use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::ser::SerializeMap as _;
use serde::{Serialize, Serializer};

use super::{Error, FORMAT_VERSION, TOML_EXTENSION};

/// Serializes as `format = 1` followed by `[<key>]`.
struct Envelope<'a, T> {
    key: &'static str,
    value: &'a T,
}

impl<T: Serialize> Serialize for Envelope<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("format", &FORMAT_VERSION)?;
        map.serialize_entry(self.key, self.value)?;
        map.end()
    }
}

/// Encodes `value` in the envelope.
pub(crate) fn encode<T: Serialize>(
    path: &Path,
    key: &'static str,
    value: &T,
) -> Result<String, Error> {
    toml::to_string(&Envelope { key, value }).map_err(|err| Error::Serialize {
        path: path.to_path_buf(),
        message: err.to_string(),
    })
}

/// [`FORMAT_VERSION`] as a TOML integer.
const SUPPORTED: i64 = FORMAT_VERSION as i64;

/// Decodes an envelope. A missing `format` reads as 1, a newer one is rejected, unknown keys are
/// ignored and a missing payload table reads as an empty table (serde defaults apply).
pub(crate) fn decode<T: DeserializeOwned>(path: &Path, text: &str, key: &str) -> Result<T, Error> {
    let parse_error = |message: String| Error::Parse {
        path: path.to_path_buf(),
        message,
    };
    let mut table: toml::Table =
        toml::from_str(text).map_err(|err| parse_error(err.to_string()))?;
    match table.get("format") {
        None | Some(toml::Value::Integer(..=SUPPORTED)) => {}
        Some(toml::Value::Integer(found)) => {
            return Err(Error::UnsupportedFormat {
                path: path.to_path_buf(),
                found: *found,
                supported: FORMAT_VERSION,
            });
        }
        Some(other) => {
            return Err(parse_error(format!(
                "`format` must be an integer, found {}",
                other.type_str()
            )));
        }
    }
    let payload = table
        .remove(key)
        .unwrap_or_else(|| toml::Value::Table(toml::Table::new()));
    payload
        .try_into()
        .map_err(|err: toml::de::Error| parse_error(err.to_string()))
}

/// Reads and decodes `path`; `Ok(None)` when it does not exist.
pub(crate) fn read<T: DeserializeOwned>(path: &Path, key: &str) -> Result<Option<T>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => decode(path, &text, key).map(Some),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Encodes `value` and writes it atomically, creating the parent directory if needed.
pub(crate) fn write<T: Serialize>(path: &Path, key: &'static str, value: &T) -> Result<(), Error> {
    let text = encode(path, key, value)?;
    write_bytes(path, text.as_bytes())
}

/// Writes `bytes` atomically (temporary file in the same directory, fsync, rename).
///
/// On Unix the temporary file, and therefore the result, has mode `0600`. On failure the target
/// is unchanged and the temporary file is removed.
// Mirrors dao/A3SFilesAccessor.java:144-162, 188-201
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|source| Error::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    crate::a3s_file::write_atomic(path, bytes).map_err(|err| Error::Write {
        path: path.to_path_buf(),
        source: match err {
            crate::Error::Io { source, .. } => source,
            other => io::Error::other(other.to_string()),
        },
    })
}

/// The regular files of `dir` whose name ends with `.toml`, sorted by file name.
///
/// A missing directory gives an empty list.
pub(crate) fn list_toml_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let is_toml = name.to_str().is_some_and(|n| n.ends_with(TOML_EXTENSION));
        // `metadata` follows symlinks, like Java `File.isFile`.
        if is_toml && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_file()) {
            files.push(entry.path());
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

/// The file name of `path`, for messages and failure maps.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `File.lastModified()` in ms; `0` when unknown, as in Java.
pub(crate) fn modified_millis(path: &Path) -> i64 {
    a3s_hash::modified_millis(path).unwrap_or(0)
}
