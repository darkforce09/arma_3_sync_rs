use std::io::{Read as _, Write as _};
use std::path::Path;

use a3s_types::JavaCodec;
use flate2::Compression;
use flate2::read::GzDecoder;

use crate::error::IoContext as _;
use crate::temp_file::{TempFileMode, temp_file_next_to};
use crate::{Error, Result};

/// Maximum decompressed size accepted, matching the Java reader's stream limit.
pub const MAX_DECOMPRESSED_SIZE: u64 = 256 * 1024 * 1024;

/// The metadata files of a repository's `.a3s/` folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum A3sFile {
    /// `SyncTreeDirectory`: the full content tree.
    Sync,

    /// `ServerInfo`: revision, counts, flags.
    ServerInfo,

    /// `Changelogs`: build history.
    Changelogs,

    /// `AutoConfig`: protocol and favorite servers for importing the repository.
    AutoConfig,

    /// `Events`: modsets.
    Events,
}

impl A3sFile {
    pub const ALL: [Self; 5] = [
        Self::Sync,
        Self::ServerInfo,
        Self::Changelogs,
        Self::AutoConfig,
        Self::Events,
    ];

    /// File name inside `.a3s/`.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::ServerInfo => "serverinfo",
            Self::Changelogs => "changelogs",
            Self::AutoConfig => "autoconfig",
            Self::Events => "events",
        }
    }

    /// Path relative to the repository root, e.g. `.a3s/sync`.
    pub fn relative_path(self) -> String {
        format!("{}/{}", crate::layout::A3S_FOLDER, self.file_name())
    }
}

/// Decodes gzip-compressed Java serialization bytes (e.g. a downloaded `.a3s` file).
pub fn decode_a3s<T: JavaCodec>(compressed: &[u8]) -> Result<T> {
    Ok(a3s_types::decode_stream(&gunzip(compressed)?)?)
}

/// Decompresses gzip bytes, refusing more than [`MAX_DECOMPRESSED_SIZE`] bytes of output.
pub(crate) fn gunzip(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    GzDecoder::new(compressed)
        .take(MAX_DECOMPRESSED_SIZE + 1)
        .read_to_end(&mut raw)
        .map_err(Error::Gzip)?;
    if raw.len() as u64 > MAX_DECOMPRESSED_SIZE {
        return Err(Error::TooLarge(MAX_DECOMPRESSED_SIZE));
    }
    Ok(raw)
}

/// Encodes `value` as gzip-compressed Java serialization bytes.
pub fn encode_a3s<T: JavaCodec>(value: &T) -> Result<Vec<u8>> {
    gzip(&a3s_types::encode_stream(value)?)
}

/// Compresses `raw` like Java's `GZIPOutputStream`.
pub(crate) fn gzip(raw: &[u8]) -> Result<Vec<u8>> {
    // Header like Java's `GZIPOutputStream` (mtime 0, OS "unknown" since JDK 16).
    let mut encoder = flate2::GzBuilder::new()
        .operating_system(255)
        .write(Vec::with_capacity(raw.len() / 4), Compression::default());
    encoder.write_all(raw).map_err(Error::Gzip)?;
    encoder.finish().map_err(Error::Gzip)
}

/// Reads and decodes an `.a3s` file.
pub fn read_a3s<T: JavaCodec>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_path(path)?;
    decode_a3s(&bytes)
}

/// Encodes and writes an `.a3s` file atomically (temporary file, then rename).
///
/// The file gets the umask permissions so a web server running as another user can serve the
/// repository (Java's `Files.createTempFile` leaves it `0600`).
pub fn write_a3s<T: JavaCodec>(path: &Path, value: &T) -> Result<()> {
    let bytes = encode_a3s(value)?;
    write_atomic(path, &bytes, TempFileMode::Umask)
}

/// Writes an already encoded Java serialization stream as an `.a3s` file (gzip, temporary
/// file, then rename), so the bytes on disk are exactly the bytes the caller validated.
///
/// Permissions as for [`write_a3s`].
pub fn write_a3s_stream(path: &Path, raw: &[u8]) -> Result<()> {
    write_atomic(path, &gzip(raw)?, TempFileMode::Umask)
}

/// Writes `bytes` to `path` through a temporary file renamed over it.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], mode: TempFileMode) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).with_path(dir)?;
    let mut tmp = temp_file_next_to(path, mode).with_path(dir)?;
    tmp.write_all(bytes).with_path(tmp.path())?;
    tmp.as_file().sync_all().with_path(tmp.path())?;
    tmp.persist(path).map_err(|err| Error::Io {
        path: path.to_owned(),
        source: err.error,
    })?;
    Ok(())
}
