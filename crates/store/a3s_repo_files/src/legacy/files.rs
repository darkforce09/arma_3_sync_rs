//! Reading the Java client's gzip-compressed serialized files (`dao/A3SFilesAccessor.java`).

use std::io::{self, Read as _, Write as _};
use std::path::Path;

use a3s_jserial::Graph;
use a3s_types::local::{LegacyConfiguration, LegacyDecode, LegacyRepository, Preferences, Profile};
use flate2::Compression;
use flate2::read::GzDecoder;

use super::allowlist::check_java_allowlist;
use super::error::{DeserializeError, Error};
use super::sealed;
use crate::a3s_file::MAX_DECOMPRESSED_SIZE;
use crate::local::toml_file::file_name;

/// A decoded legacy file and the warnings of its lenient decoding, each prefixed with the file
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyFile<T> {
    pub value: T,
    pub warnings: Vec<String>,
}

/// Gunzips `compressed`, at most [`MAX_DECOMPRESSED_SIZE`] bytes (Java's `streamBytes` limit).
///
/// Errors of the gzip format or a truncated input are [`Error::Corrupted`] (Java
/// `ZipException` / `EOFException`); other read errors are [`Error::Io`].
// Mirrors dao/A3SFilesAccessor.java:28, 62
pub(crate) fn gunzip(file: &str, compressed: impl io::Read) -> Result<Vec<u8>, Error> {
    let mut raw = Vec::new();
    GzDecoder::new(compressed)
        .take(MAX_DECOMPRESSED_SIZE + 1)
        .read_to_end(&mut raw)
        .map_err(|source| match source.kind() {
            io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidData
            | io::ErrorKind::UnexpectedEof => Error::Corrupted {
                file: file.to_owned(),
                source,
            },
            _ => Error::Io {
                file: file.to_owned(),
                source,
            },
        })?;
    if raw.len() as u64 > MAX_DECOMPRESSED_SIZE {
        return Err(Error::Io {
            file: file.to_owned(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                format!("decompressed data exceeds {MAX_DECOMPRESSED_SIZE} bytes"),
            ),
        });
    }
    Ok(raw)
}

/// Gzips `raw` with a header like Java's `GZIPOutputStream`.
pub(crate) fn gzip(raw: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = flate2::GzBuilder::new()
        .operating_system(255)
        .write(Vec::with_capacity(raw.len() / 2), Compression::default());
    encoder.write_all(raw)?;
    encoder.finish()
}

/// Parses a Java stream; a truncated one counts as corrupted (Java `EOFException`).
pub(crate) fn parse_stream(file: &str, raw: &[u8]) -> Result<Graph, Error> {
    a3s_jserial::read(raw).map_err(|err| match err {
        a3s_jserial::Error::UnexpectedEof => Error::Corrupted {
            file: file.to_owned(),
            source: io::Error::new(io::ErrorKind::UnexpectedEof, err.to_string()),
        },
        other => Error::Deserialize {
            file: file.to_owned(),
            source: DeserializeError::Stream(other),
        },
    })
}

/// Reads `GZIP(ObjectOutputStream(object))` and checks Java's class allowlist.
///
/// Used for `a3s.cfg`, `a3s.prefs` and `*.a3s.profile`. The stream is parsed with generous
/// limits rather than Java's depth and reference limits; the allowlist is enforced.
// Mirrors dao/A3SFilesAccessor.java:55-83
pub fn read_object_file(path: &Path) -> Result<Graph, Error> {
    let file = file_name(path);
    let compressed = std::fs::File::open(path).map_err(|source| Error::Io {
        file: file.clone(),
        source,
    })?;
    // Streamed: `GzDecoder` buffers the file itself.
    let raw = gunzip(&file, compressed)?;
    let graph = parse_stream(&file, &raw)?;
    check_java_allowlist(&graph).map_err(|rejected| Error::Deserialize {
        file,
        source: rejected.into(),
    })?;
    Ok(graph)
}

/// Decodes the root of `graph` as `T`, prefixing warnings with the file name.
fn decode_graph<T: LegacyDecode>(file: &str, graph: &Graph) -> Result<LegacyFile<T>, Error> {
    let deserialize = |source: DeserializeError| Error::Deserialize {
        file: file.to_owned(),
        source,
    };
    let root = graph.root_value().map_err(|err| deserialize(err.into()))?;
    let mut warnings = Vec::new();
    let value =
        T::decode_legacy(graph, root, &mut warnings).map_err(|err| deserialize(err.into()))?;
    Ok(LegacyFile {
        value,
        warnings: prefix_warnings(file, warnings),
    })
}

pub(crate) fn prefix_warnings(file: &str, warnings: Vec<String>) -> Vec<String> {
    warnings
        .into_iter()
        .map(|warning| format!("{file}: {warning}"))
        .collect()
}

fn read_plain<T: LegacyDecode>(path: &Path) -> Result<LegacyFile<T>, Error> {
    let graph = read_object_file(path)?;
    decode_graph(&file_name(path), &graph)
}

/// Reads `a3s.cfg`, including the deprecated fields to migrate.
// Mirrors dao/ConfigurationDAO.java:24-37
pub fn read_configuration_file(path: &Path) -> Result<LegacyFile<LegacyConfiguration>, Error> {
    read_plain(path)
}

/// Reads `a3s.prefs`.
// Mirrors dao/PreferencesDAO.java:15-28
pub fn read_preferences_file(path: &Path) -> Result<LegacyFile<Preferences>, Error> {
    read_plain(path)
}

/// Reads a `*.a3s.profile`.
// Mirrors dao/ProfileDAO.java:33
pub fn read_profile_file(path: &Path) -> Result<LegacyFile<Profile>, Error> {
    read_plain(path)
}

/// Reads an encrypted `*.a3s.repository`:
/// `GZIP(ObjectOutputStream(SealedObject(AES(ObjectOutputStream(Repository)))))`.
///
/// The outer stream is checked against the allowlist; the decrypted inner stream is not, as
/// Java's `SealedObject.getObject` uses its own unfiltered stream.
// Mirrors dao/A3SFilesAccessor.java:85-114
pub fn read_repository_file(path: &Path) -> Result<LegacyFile<LegacyRepository>, Error> {
    let file = file_name(path);
    let graph = read_object_file(path)?;
    let mut warnings = Vec::new();
    let inner = sealed::open(&graph, &mut warnings).map_err(|err| err.with_file(&file))?;
    let inner_graph = parse_stream(&file, &inner)?;
    let mut decoded: LegacyFile<LegacyRepository> = decode_graph(&file, &inner_graph)?;
    let mut all = prefix_warnings(&file, warnings);
    all.append(&mut decoded.warnings);
    decoded.warnings = all;
    Ok(decoded)
}
