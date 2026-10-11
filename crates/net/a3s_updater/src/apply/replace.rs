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

/// Prefix of the directory the elevated updater writes its marker, result and log to:
/// `.a3s-elevated-<pid>-<nonce>` inside the installation.
pub(crate) const ELEVATED_PREFIX: &str = ".a3s-elevated-";

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
/// new file is moved in; the old file is deleted when possible, otherwise by a later
/// [`cleanup_leftovers`] with write access to the installation.
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
            // A running executable cannot be deleted; `cleanup_leftovers` removes it once this
            // process has exited.
            tracing::debug!(path = %old.display(), error = %err, "replaced file kept for a later cleanup");
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

/// Repairs and removes what interrupted updates left in `install_dir`.
///
/// Visits at most four directory levels, without following symbolic links and skipping the
/// portable data folder:
/// - `.a3s-old-<pid>-<n>-<name>`: when `<name>` is missing (the update stopped between
///   renaming the old file away and moving the new one in), the old file is renamed back;
///   otherwise it is deleted.
/// - `.a3s-new-*` files, `.a3s-stage-*` extraction directories and `.a3s-elevated-<pid>-*`
///   directories of the elevated updater are deleted.
///
/// Leftovers named after a process that is still running are left alone. Removing them needs
/// write access to the installation, so in a protected installation only the next elevated
/// update removes them. Errors are logged and skipped. Returns the number of entries repaired
/// or removed.
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
        let leftover = if file_type.is_file() {
            [OLD_PREFIX, NEW_PREFIX]
                .into_iter()
                .find(|prefix| name.starts_with(prefix))
        } else if file_type.is_dir() {
            [STAGE_PREFIX, ELEVATED_PREFIX]
                .into_iter()
                .find(|prefix| name.starts_with(prefix))
        } else {
            None
        };
        let Some(prefix) = leftover else {
            let portable = depth == 0 && name.eq_ignore_ascii_case(PORTABLE_DATA_DIR);
            if file_type.is_dir() && depth < CLEANUP_DEPTH && !portable {
                sweep(&path, depth + 1, removed);
            }
            continue;
        };
        let owner = parse_owner(&name[prefix.len()..]);
        if owner.is_some_and(|(pid, _)| process_is_alive(pid)) {
            continue;
        }
        let result = match (prefix, owner) {
            (OLD_PREFIX, Some((_, target))) if is_missing(&dir.join(target)) => {
                std::fs::rename(&path, dir.join(target))
            }
            (OLD_PREFIX | NEW_PREFIX, _) => std::fs::remove_file(&path),
            _ => std::fs::remove_dir_all(&path),
        };
        match result {
            Ok(()) => *removed += 1,
            Err(err) => {
                tracing::debug!(path = %path.display(), error = %err, "leftover not removed");
            }
        }
    }
}

/// Splits `<pid>-<rest>` of a leftover name; for replacer leftovers `<rest>` is `<n>-<name>`
/// and the second value is `<name>`.
fn parse_owner(suffix: &str) -> Option<(u32, &str)> {
    let (pid, rest) = suffix.split_once('-')?;
    let pid = pid.parse().ok()?;
    let target = rest
        .split_once('-')
        .filter(|(n, _)| n.parse::<u32>().is_ok())
        .map_or(rest, |(_, name)| name);
    Some((pid, target))
}

fn is_missing(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// Whether a process with this id is running; `true` when that cannot be determined.
pub(crate) fn process_is_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    if cfg!(target_os = "linux") {
        return Path::new("/proc").join(pid.to_string()).exists();
    }
    let output = if cfg!(windows) {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
    } else {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string()])
            .output()
    };
    match output {
        Ok(output) if cfg!(windows) => {
            String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
        }
        Ok(output) => output.status.success(),
        Err(_) => true,
    }
}
