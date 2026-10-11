//! Atomic writes of small local files (autostart entries, desktop launchers).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{Error, Result};

/// Writes `contents` to `path` through a temporary file in the same directory and a rename, so a
/// reader never sees a half-written file. On Unix the file gets `mode` (umask permissions when
/// `None`).
pub(crate) fn write_atomic(path: &Path, contents: &[u8], mode: Option<u32>) -> Result<()> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::InvalidInput(format!("{} has no file name", path.display())))?;
    let temp = dir.join(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = fs::write(&temp, contents)
        .and_then(|()| set_mode(&temp, mode))
        .and_then(|()| fs::rename(&temp, path));
    if let Err(source) = result {
        // Best effort: the temporary file may not exist when the first write failed.
        let _ignored = fs::remove_file(&temp);
        return Err(Error::io(path, source));
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    match mode {
        Some(mode) => fs::set_permissions(path, fs::Permissions::from_mode(mode)),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the Unix version"
)]
fn set_mode(_path: &Path, _mode: Option<u32>) -> std::io::Result<()> {
    Ok(())
}

/// Removes `path`; a missing file is not an error.
pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::io(path, source)),
    }
}
