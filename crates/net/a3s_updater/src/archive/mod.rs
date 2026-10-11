//! Update archives: validation, flat staging and installation with managed-file tracking.
//!
//! The archive is untrusted. Every entry name is checked before anything is extracted, every
//! entry is read once to verify its CRC, and the installation is written only after the whole
//! archive has been staged.

mod install;

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub use install::{
    CopyReplacer, FileReplacer, InstallReport, MANAGED_FILES_NAME, install_archive, install_staged,
    install_tree, is_user_managed,
};

use crate::UpdateError;

/// Most entries an update archive may have.
pub const MAX_ENTRIES: usize = 65_536;

/// Largest total uncompressed size of an update archive (4 GiB).
pub const MAX_UNCOMPRESSED_BYTES: u64 = 4 << 30;

/// Unix file-type bits of an entry's mode.
const FILE_TYPE_MASK: u32 = 0o170_000;
const FILE_TYPE_REGULAR: u32 = 0o100_000;
const FILE_TYPE_DIRECTORY: u32 = 0o040_000;

/// One archive file extracted to a flat temporary file, ready to be installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedEntry {
    /// The staged file.
    pub source: PathBuf,

    /// Normalized `/`-separated path inside the installation.
    pub relative: String,

    /// Permission bits (`0o777` at most) to give the installed file on Unix.
    pub unix_mode: Option<u32>,
}

/// Checks the whole archive before anything is extracted: every entry name (directories
/// included), no symbolic links, the CRC of every file entry, and the size limits.
// Mirrors a3sUpdater/dao/FtpDAO.java:122-145
pub fn validate_archive(archive: &Path) -> Result<(), UpdateError> {
    let mut zip = open_archive(archive)?;
    let mut files = 0usize;
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| invalid(&e))?;
        let name = entry_name(&entry)?;
        if classify(&entry, &name)?.is_none() {
            continue;
        }
        files += 1;
        let limit = MAX_UNCOMPRESSED_BYTES - total;
        let read = io::copy(&mut (&mut entry).take(limit + 1), &mut io::sink())
            .map_err(|e| UpdateError::ArchiveInvalid(format!("{name}: {e}")))?;
        total += read;
        if read > limit {
            return Err(too_large());
        }
    }
    if files == 0 {
        return Err(UpdateError::ArchiveEmpty);
    }
    Ok(())
}

/// Extracts every file entry, in archive order, into its own flat `entry-*.tmp` file in `dir`.
///
/// `dir` is created if needed. Directory entries produce nothing. On Unix the entry's permission bits
/// (without setuid, setgid and sticky) are applied to the staged file.
// Mirrors a3sUpdater/dao/FtpDAO.java:200-229
pub fn stage_archive(archive: &Path, dir: &Path) -> Result<Vec<StagedEntry>, UpdateError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| UpdateError::io(format!("Could not create {}", dir.display()), e))?;
    let mut zip = open_archive(archive)?;
    let mut staged = Vec::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| invalid(&e))?;
        let name = entry_name(&entry)?;
        let Some(relative) = classify(&entry, &name)? else {
            continue;
        };
        let unix_mode = entry_permissions(&entry);
        let limit = MAX_UNCOMPRESSED_BYTES - total;
        let (source, written) = stage_entry(&mut (&mut entry).take(limit + 1), dir, unix_mode)
            .map_err(|e| UpdateError::io(format!("Could not stage ZIP entry '{name}'."), e))?;
        total += written;
        if written > limit {
            // The entry's temporary file is part of `dir`; the caller removes `dir`.
            return Err(too_large());
        }
        staged.push(StagedEntry {
            source,
            relative,
            unix_mode,
        });
    }
    Ok(staged)
}

/// Writes one entry to a new temporary file in `dir`; the file is removed on failure.
fn stage_entry(
    reader: &mut impl Read,
    dir: &Path,
    unix_mode: Option<u32>,
) -> io::Result<(PathBuf, u64)> {
    let mut temp = tempfile::Builder::new()
        .prefix("entry-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    let written = io::copy(reader, temp.as_file_mut())?;
    set_permissions(temp.as_file(), unix_mode)?;
    let (_file, path) = temp.keep().map_err(|e| e.error)?;
    Ok((path, written))
}

fn open_archive(archive: &Path) -> Result<zip::ZipArchive<File>, UpdateError> {
    let file = File::open(archive)
        .map_err(|e| UpdateError::io(format!("Could not open {}", archive.display()), e))?;
    let zip = zip::ZipArchive::new(file).map_err(|e| invalid(&e))?;
    if zip.len() > MAX_ENTRIES {
        return Err(UpdateError::ArchiveInvalid(format!(
            "more than {MAX_ENTRIES} entries"
        )));
    }
    Ok(zip)
}

fn invalid(err: &zip::result::ZipError) -> UpdateError {
    UpdateError::ArchiveInvalid(err.to_string())
}

fn too_large() -> UpdateError {
    UpdateError::ArchiveInvalid(format!(
        "more than {MAX_UNCOMPRESSED_BYTES} bytes uncompressed"
    ))
}

fn entry_name(entry: &zip::read::ZipFile<'_, File>) -> Result<String, UpdateError> {
    entry
        .name()
        .map(std::borrow::Cow::into_owned)
        .map_err(|e| invalid(&e))
}

/// The normalized path of a file entry, or `None` for a directory entry. Rejects unsafe names,
/// symbolic links and other special files.
// Mirrors a3sUpdater/dao/FtpDAO.java:235-244
fn classify(
    entry: &zip::read::ZipFile<'_, File>,
    name: &str,
) -> Result<Option<String>, UpdateError> {
    let relative = normalize_entry_name(name)?;
    let file_type = entry.unix_mode().map(|mode| mode & FILE_TYPE_MASK);
    // A directory flag in the mode or a name ending with `/` or `\`.
    if entry.is_dir() || file_type == Some(FILE_TYPE_DIRECTORY) {
        return Ok(None);
    }
    if !matches!(file_type, None | Some(0 | FILE_TYPE_REGULAR)) {
        return Err(UpdateError::UnsafeEntry(name.to_owned()));
    }
    if relative.is_empty() {
        return Err(UpdateError::UnsafeEntry(name.to_owned()));
    }
    Ok(Some(relative))
}

/// Permission bits of a file entry written by a Unix archiver.
fn entry_permissions(entry: &zip::read::ZipFile<'_, File>) -> Option<u32> {
    if entry.system() != zip::System::Unix {
        return None;
    }
    entry.unix_mode().map(|mode| mode & 0o777)
}

/// Normalizes an archive entry name to a `/`-separated path strictly inside the extraction
/// root; `""` is the root itself.
///
/// `\` counts as a separator; `.` components are dropped and `a/../b` becomes `b`. Rejected: a
/// blank name, NUL, a rooted name (`/x`, `//server/x`), a `:` in any component (drive letters,
/// alternate data streams) and any `..` that leaves the root.
// Mirrors a3sUpdater/dao/FtpDAO.java:246-260
pub(crate) fn normalize_entry_name(name: &str) -> Result<String, UpdateError> {
    if name.trim().is_empty() {
        return Err(UpdateError::EmptyEntryName);
    }
    let unsafe_entry = || UpdateError::UnsafeEntry(name.to_owned());
    let portable = name.replace('\\', "/");
    if portable.contains('\0') || portable.starts_with('/') {
        return Err(unsafe_entry());
    }
    let mut parts: Vec<&str> = Vec::new();
    for component in portable.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or_else(unsafe_entry)?;
            }
            _ if component.contains(':') => return Err(unsafe_entry()),
            _ => parts.push(component),
        }
    }
    Ok(parts.join("/"))
}

#[cfg(unix)]
pub(crate) fn set_permissions(file: &File, unix_mode: Option<u32>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    /// Permission bits of files whose entry carries none.
    const DEFAULT_FILE_MODE: u32 = 0o644;
    let mode = unix_mode.unwrap_or(DEFAULT_FILE_MODE) & 0o777;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
pub(crate) fn set_permissions(_file: &File, _unix_mode: Option<u32>) -> io::Result<()> {
    Ok(())
}
