//! Update archives: verification, flat staging and installation with managed-file tracking.
//!
//! The archive is untrusted. [`validate_archive`] copies it once into a private unnamed
//! temporary file while hashing it, then checks every entry (names, conflicts, links, CRC, size
//! limits) from that copy; [`stage_archive`] extracts from the same copy, so the bytes that were
//! verified are the bytes that are installed. The installation is written only after the whole
//! archive has been staged.

mod install;
mod names;

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek as _, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

pub use install::{
    CopyReplacer, FileReplacer, InstallReport, MANAGED_FILES_NAME, install_staged, is_user_managed,
};
pub(crate) use names::{is_portable_file_name, normalize_entry_name};

use crate::UpdateError;

/// Most entries an update archive may have.
pub(crate) const MAX_ENTRIES: usize = 65_536;

/// Largest update archive, compressed (download) or uncompressed (all entries together): 4 GiB.
pub(crate) const MAX_ARCHIVE_BYTES: u64 = 4 << 30;

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

    /// Permission bits to give the installed file on Unix; they are sanitized when applied
    /// (`(mode & 0o755) | 0o600`, `0o644` when `None`).
    pub unix_mode: Option<u32>,
}

/// A verified update archive: a private copy whose SHA-256 matched and whose entries all passed
/// validation. Stage it with [`stage_archive`].
#[derive(Debug)]
pub struct VerifiedArchive {
    zip: zip::ZipArchive<File>,
    sha256: String,
}

impl VerifiedArchive {
    /// Lowercase hex SHA-256 of the archive bytes that were validated.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Verifies an update archive before anything is extracted.
///
/// The file is opened once and copied, while being hashed, into an unnamed temporary file that
/// no other process can open; replacing or rewriting `archive` afterwards has no effect. With
/// `expected_sha256`, a different hash fails with [`UpdateError::Integrity`]. Then every entry
/// is checked from the copy: names (no escape, absolute, drive, device or `:` names), no two
/// paths that collide ignoring case or as file and directory, no symbolic links, the CRC of
/// every file, at most 65 536 entries and 4 GiB in total.
// Mirrors a3sUpdater/dao/FtpDAO.java:122-145
pub fn validate_archive(
    archive: &Path,
    expected_sha256: Option<&str>,
) -> Result<VerifiedArchive, UpdateError> {
    let (copy, sha256) = private_copy(archive)?;
    if let Some(expected) = expected_sha256
        && !sha256.eq_ignore_ascii_case(expected.trim())
    {
        return Err(UpdateError::Integrity);
    }
    let mut zip = zip::ZipArchive::new(copy).map_err(|e| invalid(&e))?;
    if zip.len() > MAX_ENTRIES {
        return Err(UpdateError::ArchiveInvalid(format!(
            "more than {MAX_ENTRIES} entries"
        )));
    }
    let mut paths = PathSet::default();
    let mut files = 0usize;
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| invalid(&e))?;
        let name = entry_name(&entry)?;
        let Some(relative) = classify(&entry, &name)? else {
            let directory = normalize_entry_name(&name)?;
            paths.add_directory(&directory)?;
            continue;
        };
        paths.add_file(&relative)?;
        files += 1;
        let limit = MAX_ARCHIVE_BYTES - total;
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
    Ok(VerifiedArchive { zip, sha256 })
}

/// Copies `archive` into a private unnamed temporary file, returning it rewound with the hash of
/// the copied bytes.
fn private_copy(archive: &Path) -> Result<(File, String), UpdateError> {
    let source = File::open(archive).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            UpdateError::ArchiveNotFound(archive.to_path_buf())
        } else {
            UpdateError::io(format!("Could not open {}", archive.display()), e)
        }
    })?;
    let copy_error = |e| UpdateError::io(format!("Could not copy {}", archive.display()), e);
    let mut copy = tempfile::tempfile().map_err(copy_error)?;
    let mut hasher = Sha256::new();
    let mut reader = source.take(MAX_ARCHIVE_BYTES + 1);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut copied = 0u64;
    loop {
        let read = reader.read(&mut buffer).map_err(copy_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        copy.write_all(&buffer[..read]).map_err(copy_error)?;
        copied += read as u64;
    }
    if copied > MAX_ARCHIVE_BYTES {
        return Err(too_large());
    }
    copy.rewind().map_err(copy_error)?;
    Ok((copy, crate::http::to_hex(&hasher.finalize())))
}

/// Extracts every file entry of a verified archive, in archive order, into its own flat
/// `entry-*.tmp` file in `dir`.
///
/// `dir` is created if needed. Directory entries produce nothing. On Unix the entry's sanitized
/// permission bits are applied to the staged file.
///
/// [`install_staged`] reopens the staged files by path, so `dir` must be a directory that no
/// less-privileged user can write: an elevated installer passes a directory it created itself
/// in a location only it may modify.
// Mirrors a3sUpdater/dao/FtpDAO.java:200-229
pub fn stage_archive(
    archive: &mut VerifiedArchive,
    dir: &Path,
) -> Result<Vec<StagedEntry>, UpdateError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| UpdateError::io(format!("Could not create {}", dir.display()), e))?;
    let zip = &mut archive.zip;
    let mut staged = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| invalid(&e))?;
        let name = entry_name(&entry)?;
        let Some(relative) = classify(&entry, &name)? else {
            continue;
        };
        let unix_mode = entry_permissions(&entry);
        // Validation bounded the total size of this same copy.
        let source = stage_entry(&mut entry, dir, unix_mode)
            .map_err(|e| UpdateError::io(format!("Could not stage ZIP entry '{name}'."), e))?;
        staged.push(StagedEntry {
            source,
            relative,
            unix_mode,
        });
    }
    Ok(staged)
}

/// Writes one entry to a new temporary file in `dir`; the file is removed on failure.
fn stage_entry(reader: &mut impl Read, dir: &Path, unix_mode: Option<u32>) -> io::Result<PathBuf> {
    let mut temp = tempfile::Builder::new()
        .prefix("entry-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    io::copy(reader, temp.as_file_mut())?;
    set_permissions(temp.as_file(), unix_mode)?;
    let (_file, path) = temp.keep().map_err(|e| e.error)?;
    Ok(path)
}

fn invalid(err: &zip::result::ZipError) -> UpdateError {
    UpdateError::ArchiveInvalid(err.to_string())
}

fn too_large() -> UpdateError {
    UpdateError::ArchiveInvalid(format!("more than {MAX_ARCHIVE_BYTES} bytes"))
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
    if !matches!(file_type, None | Some(0 | FILE_TYPE_REGULAR)) || relative.is_empty() {
        return Err(UpdateError::UnsafeEntry(name.to_owned()));
    }
    Ok(Some(relative))
}

/// Permission bits of a file entry written by a Unix archiver.
fn entry_permissions(entry: &zip::read::ZipFile<'_, File>) -> Option<u32> {
    if entry.system() != zip::System::Unix {
        return None;
    }
    entry.unix_mode().map(|mode| sanitize_mode(Some(mode)))
}

/// Installed permission bits: the owner can always read and write, nobody else can write, and
/// setuid, setgid and sticky are dropped (`(mode & 0o755) | 0o600`); `None` gives `0o644`.
pub(crate) fn sanitize_mode(unix_mode: Option<u32>) -> u32 {
    unix_mode.map_or(0o644, |mode| (mode & 0o755) | 0o600)
}

/// Paths of one archive or one installation: no two may collide ignoring case, and no file may
/// also be the parent directory of another path.
#[derive(Default)]
pub(crate) struct PathSet {
    files: HashSet<String>,
    directories: HashSet<String>,
}

impl PathSet {
    fn conflict(relative: &str) -> UpdateError {
        UpdateError::PathConflict(relative.to_owned())
    }

    /// Adds a file path (normalized, non-empty).
    pub(crate) fn add_file(&mut self, relative: &str) -> Result<(), UpdateError> {
        let key = relative.to_lowercase();
        if self.files.contains(&key) || self.directories.contains(&key) {
            return Err(Self::conflict(relative));
        }
        self.add_parents(relative, &key)?;
        self.files.insert(key);
        Ok(())
    }

    /// Adds a directory path (normalized; empty is the root).
    pub(crate) fn add_directory(&mut self, relative: &str) -> Result<(), UpdateError> {
        if relative.is_empty() {
            return Ok(());
        }
        let key = relative.to_lowercase();
        if self.files.contains(&key) {
            return Err(Self::conflict(relative));
        }
        self.add_parents(relative, &key)?;
        self.directories.insert(key);
        Ok(())
    }

    fn add_parents(&mut self, relative: &str, key: &str) -> Result<(), UpdateError> {
        for (index, _) in key.match_indices('/') {
            let parent = &key[..index];
            if self.files.contains(parent) {
                return Err(Self::conflict(relative));
            }
            self.directories.insert(parent.to_owned());
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn set_permissions(file: &File, unix_mode: Option<u32>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(std::fs::Permissions::from_mode(sanitize_mode(unix_mode)))
}

#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the Unix version, which can fail"
)]
pub(crate) fn set_permissions(_file: &File, _unix_mode: Option<u32>) -> io::Result<()> {
    Ok(())
}
