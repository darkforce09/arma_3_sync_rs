//! Temporary files written next to their target and renamed over it.

use std::io;
use std::path::Path;

use tempfile::NamedTempFile;

use crate::layout::TEMP_SUFFIX;

/// Permissions of a temporary file, kept by the rename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TempFileMode {
    /// Owner only (`0600`), like Java's `Files.createTempFile`.
    Private,

    /// `0666` minus the process umask, like a Java `FileOutputStream`: a web server running as
    /// another user can serve the file.
    Umask,
}

/// Creates `<target name>.<random>.a3s.tmp` in `target`'s folder.
///
/// The name contains `.a3s`, so a temporary file left behind by a killed process is never a
/// sync leaf of a repository build (Java's and Rust's builders skip such names).
// Mirrors dao/A3SFilesAccessor.java:188-193
pub fn temp_file_next_to(target: &Path, mode: TempFileMode) -> io::Result<NamedTempFile> {
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut prefix = target.file_name().unwrap_or_default().to_owned();
    prefix.push(".");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix).suffix(TEMP_SUFFIX);
    #[cfg(unix)]
    if mode == TempFileMode::Umask {
        use std::os::unix::fs::PermissionsExt as _;
        // `open(2)` applies the umask to this mode.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    #[cfg(not(unix))]
    let _ = mode;
    builder.tempfile_in(dir)
}
