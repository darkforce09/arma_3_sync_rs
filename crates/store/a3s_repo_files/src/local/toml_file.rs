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

/// The only accepted value of `format` (a missing key reads as this).
const SUPPORTED: i64 = FORMAT_VERSION as i64;

/// Decodes an envelope. A missing `format` reads as 1, any other value is rejected, unknown
/// keys are ignored and a missing payload table reads as an empty table (serde defaults apply).
///
/// Error messages never contain file content (the files hold credentials): only the parser's
/// message, the line and column, and serde texts with the offending values removed.
pub(crate) fn decode<T: DeserializeOwned>(path: &Path, text: &str, key: &str) -> Result<T, Error> {
    let parse_error = |message: String| Error::Parse {
        path: path.to_path_buf(),
        message,
    };
    let mut table: toml::Table =
        toml::from_str(text).map_err(|err| parse_error(describe(&err, text)))?;
    match table.get("format") {
        None | Some(toml::Value::Integer(SUPPORTED)) => {}
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
        .map_err(|err: toml::de::Error| parse_error(describe(&err, text)))
}

/// A content-free description of a TOML error: message plus `at line L, column C` when known.
fn describe(err: &toml::de::Error, text: &str) -> String {
    let message = redact(err.message());
    match err.span() {
        Some(span) => {
            let before = text.get(..span.start).unwrap_or(text);
            let line = before.matches('\n').count() + 1;
            let column = before
                .rsplit('\n')
                .next()
                .map_or(1, |l| l.chars().count() + 1);
            format!("{message} at line {line}, column {column}")
        }
        None => message,
    }
}

/// Removes the values that serde messages quote (`invalid type: string "x", expected …`,
/// ``unknown variant `x`, expected …``) and keeps only the first line.
fn redact(message: &str) -> String {
    let first = message.lines().next().unwrap_or_default();
    for prefix in ["invalid type", "invalid value", "unknown variant"] {
        if first.starts_with(prefix) {
            return match first.rfind(", expected") {
                Some(i) => format!("{prefix}{}", &first[i..]),
                None => prefix.to_owned(),
            };
        }
    }
    first.to_owned()
}

/// The `name` string of the `[<key>]` table, when `text` is TOML at all: identifies a file that
/// failed to decode (wrong structure or a newer `format`).
pub(crate) fn peek_name(text: &str, key: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(text).ok()?;
    table.get(key)?.get("name")?.as_str().map(str::to_owned)
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
/// On Unix the temporary file, and therefore the result, has mode `0600`, and the directory is
/// synced after the rename (best effort). On failure the target is unchanged and the temporary
/// file is removed. Any failure, including creating the directory, is reported for the target
/// file (Java's `CreateDirectoryException` is an `IOException` of the write).
// Mirrors dao/A3SFilesAccessor.java:144-162, 188-201
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let write_error = |source: io::Error| Error::Write {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir).map_err(write_error)?;
    }
    crate::a3s_file::write_atomic(path, bytes, crate::temp_file::TempFileMode::Private).map_err(
        |err| {
            write_error(match err {
                crate::Error::Io { source, .. } => source,
                other => io::Error::other(other.to_string()),
            })
        },
    )?;
    #[cfg(unix)]
    if let Some(dir) = dir {
        // Makes the rename durable; the data itself was synced before it.
        if let Err(err) = std::fs::File::open(dir).and_then(|d| d.sync_all()) {
            tracing::debug!("Cannot sync directory {}: {err}", dir.display());
        }
    }
    Ok(())
}

/// The regular files of `dir` whose name ends with `.toml`, sorted by file name.
///
/// A missing directory gives an empty list.
pub(crate) fn list_toml_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return if below_a_file(dir) {
                Err(io::Error::from(io::ErrorKind::NotADirectory))
            } else {
                Ok(Vec::new())
            };
        }
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

/// Whether the nearest existing ancestor of `dir` is not a directory. Windows reports a path
/// below a regular file as missing, which must not read as an empty folder.
fn below_a_file(dir: &Path) -> bool {
    dir.ancestors()
        .skip(1)
        .find_map(|ancestor| std::fs::metadata(ancestor).ok())
        .is_some_and(|metadata| !metadata.is_dir())
}

/// The file name of `path`, for messages and failure maps.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `File.lastModified()` in ms; `0` when unknown, as in Java.
pub(crate) fn modified_millis(path: &Path) -> i64 {
    crate::hash::modified_millis(path).unwrap_or(0)
}
