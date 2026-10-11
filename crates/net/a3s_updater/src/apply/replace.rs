//! Replacing installed files, including the running executable, and removing what interrupted
//! replacements leave behind.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use a3s_foundation::paths::PORTABLE_DATA_DIR;

use crate::FileReplacer;
use crate::archive::sanitize_mode;

/// Prefix of a new file copied next to its target before it is renamed over it.
pub(crate) const NEW_PREFIX: &str = ".a3s-new-";

/// Prefix of a replaced file renamed away because it could not be overwritten (Windows).
pub(crate) const OLD_PREFIX: &str = ".a3s-old-";

/// Prefix of the directory an archive is extracted into inside the installation.
pub(crate) const STAGE_PREFIX: &str = ".a3s-stage-";

/// Deepest directory level [`cleanup_leftovers`] visits below the installation.
const CLEANUP_DEPTH: usize = 4;

/// File-system operations of [`SelfReplacer`], replaceable to simulate Windows errors.
#[doc(hidden)]
pub trait FsOps: Send + Sync {
    fn copy(&self, from: &Path, to: &Path) -> io::Result<u64>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Sets the Unix permission bits (no-op elsewhere).
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
}

/// The real file system.
struct RealFs;

impl FsOps for RealFs {
    fn copy(&self, from: &Path, to: &Path) -> io::Result<u64> {
        std::fs::copy(from, to)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    #[cfg(unix)]
    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }

    #[cfg(not(unix))]
    fn set_mode(&self, _path: &Path, _mode: u32) -> io::Result<()> {
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Replaces installed files, the running executable included.
///
/// The source is copied to `.a3s-new-<pid>-<n>-<name>` next to the target and renamed over it.
/// On Unix that rename is atomic and works on a running binary. On Windows a running `.exe`
/// (or a read-only file) cannot be overwritten, so when that rename fails with access denied or
/// a sharing violation, the target is renamed away to `.a3s-old-<pid>-<n>-<name>` first and the
/// new file is moved in; the old file is deleted when possible, otherwise by
/// [`cleanup_leftovers`] on a later start.
pub struct SelfReplacer {
    counter: AtomicU32,
    fs: Box<dyn FsOps>,
    windows_fallback: bool,
}

impl Default for SelfReplacer {
    fn default() -> Self {
        Self::new()
    }
}

impl SelfReplacer {
    /// A replacer on the real file system; the rename-away fallback is used on Windows only.
    pub fn new() -> Self {
        Self::with_fs(Box::new(RealFs), cfg!(windows))
    }

    /// A replacer on `fs`, with or without the Windows rename-away fallback.
    #[doc(hidden)]
    pub fn with_fs(fs: Box<dyn FsOps>, windows_fallback: bool) -> Self {
        Self {
            counter: AtomicU32::new(0),
            fs,
            windows_fallback,
        }
    }

    /// Best-effort removal of a temporary file.
    fn discard(&self, path: &Path) {
        if let Err(err) = self.fs.remove_file(path) {
            tracing::debug!(path = %path.display(), error = %err, "temporary update file not removed");
        }
    }
}

impl FileReplacer for SelfReplacer {
    fn replace(&self, source: &Path, target: &Path, unix_mode: Option<u32>) -> io::Result<()> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let name = target.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} has no file name", target.display()),
            )
        })?;
        let index = self.counter.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let sibling = |prefix: &str| -> PathBuf {
            let mut file = OsString::from(format!("{prefix}{pid}-{index}-"));
            file.push(name);
            parent.join(file)
        };

        let new = sibling(NEW_PREFIX);
        let copied = self
            .fs
            .copy(source, &new)
            .and_then(|_| self.fs.set_mode(&new, sanitize_mode(unix_mode)));
        if let Err(err) = copied {
            self.discard(&new);
            return Err(err);
        }
        let Err(error) = self.fs.rename(&new, target) else {
            return Ok(());
        };
        if !(self.windows_fallback && self.fs.exists(target) && is_locked_or_denied(&error)) {
            self.discard(&new);
            return Err(error);
        }

        let old = sibling(OLD_PREFIX);
        if let Err(away) = self.fs.rename(target, &old) {
            tracing::debug!(target = %target.display(), error = %away, "cannot rename the target away");
            self.discard(&new);
            // The first error tells the caller why the target could not be replaced.
            return Err(error);
        }
        if let Err(into) = self.fs.rename(&new, target) {
            if let Err(rollback) = self.fs.rename(&old, target) {
                tracing::warn!(target = %target.display(), error = %rollback, "cannot restore the replaced file");
            }
            self.discard(&new);
            return Err(into);
        }
        if let Err(err) = self.fs.remove_file(&old) {
            // A running executable cannot be deleted; a later start removes it.
            tracing::debug!(path = %old.display(), error = %err, "replaced file kept until the next start");
        }
        Ok(())
    }
}

/// Access denied, or on Windows a sharing violation (OS errors 5 and 32): the running image or
/// a read-only file.
fn is_locked_or_denied(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied
        || (cfg!(windows) && matches!(err.raw_os_error(), Some(5 | 32)))
}

/// Removes what interrupted updates left in `install_dir`.
///
/// That is files named `.a3s-old-*` or
/// `.a3s-new-*` and extraction directories named `.a3s-stage-*`, at most four directory levels
/// deep, without following symbolic links and skipping the portable data folder. Errors are
/// logged and skipped. Returns the number of files and directories removed.
pub fn cleanup_leftovers(install_dir: &Path) -> usize {
    let mut removed = 0;
    sweep(install_dir, 0, &mut removed);
    removed
}

fn sweep(dir: &Path, depth: usize, removed: &mut usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!(dir = %dir.display(), error = %err, "leftover cleanup cannot list");
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(dir = %dir.display(), error = %err, "leftover cleanup cannot read");
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `DirEntry::file_type` does not follow symbolic links.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let result = if file_type.is_file()
            && (name.starts_with(OLD_PREFIX) || name.starts_with(NEW_PREFIX))
        {
            std::fs::remove_file(&path)
        } else if file_type.is_dir() && name.starts_with(STAGE_PREFIX) {
            std::fs::remove_dir_all(&path)
        } else {
            let portable = depth == 0 && name.eq_ignore_ascii_case(PORTABLE_DATA_DIR);
            if file_type.is_dir() && depth < CLEANUP_DEPTH && !portable {
                sweep(&path, depth + 1, removed);
            }
            continue;
        };
        match result {
            Ok(()) => *removed += 1,
            Err(err) => {
                tracing::debug!(path = %path.display(), error = %err, "leftover not removed");
            }
        }
    }
}
