//! `export.a3s.autoconfig`: profiles, favorite servers and repositories exchanged between
//! clients (`GZIP(ObjectOutputStream(fr.soe.a3s.domain.AutoConfig))`, unencrypted).
//!
//! Passwords of the exported repositories are written in clear text, as in Java.

use std::path::{Path, PathBuf};

use a3s_jserial::JavaFilterLimits;
use a3s_types::local::{AutoConfigExport, Legacy, decode_legacy_stream};

use crate::a3s_file::{MAX_DECOMPRESSED_SIZE, gunzip, gzip, write_atomic};
use crate::error::IoContext as _;
use crate::temp_file::TempFileMode;
use crate::{Error, Result};

/// Name of the export file inside the chosen destination folder (`export` followed by
/// [`crate::layout::AUTOCONFIG_EXPORT_EXTENSION`]).
// Mirrors dao/DataAccessConstants.java:20,46
pub const AUTOCONFIG_EXPORT_FILE_NAME: &str = "export.a3s.autoconfig";

/// Encodes the export as gzip bytes.
///
/// Fails when a current Java client could not read the result: its reader rejects streams
/// nested deeper than 100 levels (each addon group level of a profile tree costs two, so trees
/// deeper than about 47 groups), with more than 250 000 references or over 256 MiB. Java
/// writes such a file and then fails to import it; this refuses it with
/// [`Error::Serialization`] ([`a3s_jserial::Error::LimitExceeded`]) or [`Error::TooLarge`].
// Mirrors dao/CommonDAO.java:36-49, dao/A3SFilesAccessor.java:27-30, 144-162
pub fn encode_auto_config_export(export: &AutoConfigExport) -> Result<Vec<u8>> {
    let raw = a3s_types::encode_stream(export)?;
    if raw.len() as u64 > MAX_DECOMPRESSED_SIZE {
        return Err(Error::TooLarge(MAX_DECOMPRESSED_SIZE));
    }
    a3s_jserial::java_filter_stats(&raw)?.check(JavaFilterLimits::JAVA_A3S)?;
    gzip(&raw)
}

/// Decodes gzip bytes of an export leniently; the warnings name every fallback taken, including
/// skipped profiles and repositories.
// Mirrors dao/CommonDAO.java:22-34, dao/A3SFilesAccessor.java:55-83
pub fn decode_auto_config_export(compressed: &[u8]) -> Result<Legacy<AutoConfigExport>> {
    Ok(decode_legacy_stream(&gunzip(compressed)?)?)
}

/// Writes `destination_folder/export.a3s.autoconfig` atomically, replacing an existing file,
/// and returns its path. The folder must already exist.
// Mirrors dao/CommonDAO.java:36-49, dao/A3SFilesAccessor.java:144-162, 188-200
pub fn export_auto_config(export: &AutoConfigExport, destination_folder: &Path) -> Result<PathBuf> {
    let bytes = encode_auto_config_export(export)?;
    if !std::fs::metadata(destination_folder)
        .with_path(destination_folder)?
        .is_dir()
    {
        return Err(Error::Io {
            path: destination_folder.to_owned(),
            source: std::io::ErrorKind::NotADirectory.into(),
        });
    }
    let path = destination_folder.join(AUTOCONFIG_EXPORT_FILE_NAME);
    write_atomic(&path, &bytes, TempFileMode::Private)?;
    Ok(path)
}

/// Reads and decodes an export file ([`decode_auto_config_export`]).
// Mirrors dao/CommonDAO.java:22-34
pub fn import_auto_config(file: &Path) -> Result<Legacy<AutoConfigExport>> {
    let bytes = std::fs::read(file).with_path(file)?;
    decode_auto_config_export(&bytes)
}
