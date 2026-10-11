//! The bikey extractor: finds every `*.bikey` file below a folder of mods and copies them, flat,
//! into one target folder (usually the server's `keys` folder).
//!
//! [`validate`] checks the two folder fields, [`extract`] walks and copies. The walk finishes
//! before the first copy; [`find_bikeys`] and [`copy_bikeys`] expose the two phases separately.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::fs_util::{copy_file_bytes, is_writable};
use crate::{CancelToken, Severity};

/// Extension of a BattlEye public key, compared ignoring ASCII case.
pub const BIKEY_EXTENSION: &str = ".bikey";

/// Title of the wizard window and of every outcome message box.
pub const WIZARD_TITLE: &str = "Bikey extractor wizard";

/// Description line of the wizard.
pub const WIZARD_DESCRIPTION: &str = "Extract *.bikey files";

/// Label of the source folder field.
pub const SOURCE_LABEL: &str = "Source directory to search for *.bikey files";

/// Label of the target folder field.
pub const TARGET_LABEL: &str = "Target directory to copy *.bikey files";

/// Title of the progress dialog; also the first line the command line prints.
pub const PROGRESS_TITLE: &str = "Extracting *.bikey files...";

/// Command-line line printed after a successful extraction.
pub const CLI_DONE: &str = "Extraction done.";

/// Command-line line printed before the error message of a failed extraction.
pub const CLI_FAILED: &str = "Extraction failed.";

/// Message shown when the user cancelled the extraction.
pub const CANCELLED_MESSAGE: &str = "Extraction canceled!";

/// Why the source or target field is rejected. Checked in declaration order; the first failure
/// wins.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum BikeyValidationError {
    /// The source field is empty.
    #[error("Source directory is empty!")]
    SourceEmpty,

    /// The source path does not exist.
    #[error("Source directory does not exists!")]
    SourceMissing,

    /// The target field is empty.
    #[error("Target directory is empty!")]
    TargetEmpty,

    /// The target path does not exist.
    #[error("Target directory does not exists!")]
    TargetMissing,

    /// No file can be created in the target folder (also when the target is a file).
    #[error("Can't write on target directory!")]
    TargetNotWritable,
}

/// Checks the two fields as typed (the GUI does not trim them; the command line trims its
/// arguments before calling this).
// Mirrors ui/tools/bikey/BiKeyExtractorDialog.java:233-246 and console/CommandGeneral.java:536-549
pub fn validate(source: &str, target: &str) -> Result<(), BikeyValidationError> {
    if source.is_empty() {
        Err(BikeyValidationError::SourceEmpty)
    } else if !Path::new(source).exists() {
        Err(BikeyValidationError::SourceMissing)
    } else if target.is_empty() {
        Err(BikeyValidationError::TargetEmpty)
    } else if !Path::new(target).exists() {
        Err(BikeyValidationError::TargetMissing)
    } else if !is_writable(Path::new(target)) {
        Err(BikeyValidationError::TargetNotWritable)
    } else {
        Ok(())
    }
}

/// Whether `file_name` is a bikey: the text from its **last** `.` equals [`BIKEY_EXTENSION`]
/// ignoring ASCII case, so `a.BIKEY` and `.bikey` match while `a.bikey.bak` and `bikey` do not.
// Mirrors dao/CommonDAO.java:92-97
pub fn is_bikey(file_name: &OsStr) -> bool {
    let name = file_name.to_string_lossy();
    name.rfind('.')
        .is_some_and(|index| name[index..].eq_ignore_ascii_case(BIKEY_EXTENSION))
}

/// Every bikey below `source`, in walk order, or `None` when `cancel` fired during the walk.
///
/// The walk is recursive and follows directory symbolic links (each real directory is visited
/// once, so link cycles end); entries are visited in file-name order, so among keys with the same
/// name the last one found is deterministic. Unreadable directories are skipped. `source` may
/// itself be a bikey file.
// Mirrors dao/CommonDAO.java:81-101
pub fn find_bikeys(source: &Path, cancel: &CancelToken) -> Option<Vec<PathBuf>> {
    let mut walk = Walk::new(cancel);
    walk.visit(source).then_some(walk.found)
}

/// What an extraction did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BikeyOutcome {
    /// `count` bikeys were found and copied; duplicates by name count each time.
    Copied {
        /// Number of keys found (not the number of distinct target files).
        count: usize,
    },

    /// No bikey was found; the target was not touched.
    NoneFound,

    /// The user cancelled. The keys copied before that stay in the target.
    Cancelled {
        /// Keys found before the cancellation.
        found: usize,

        /// Keys copied before the cancellation.
        copied: usize,
    },
}

impl BikeyOutcome {
    /// The message box text, shown under [`WIZARD_TITLE`].
    // Mirrors ui/tools/bikey/ExtractProgressDialog.java:30-41
    pub fn message(&self) -> String {
        match self {
            Self::Copied { count } => format!("{count} have been copied to target directory."),
            Self::NoneFound => "0 have been found.".to_owned(),
            Self::Cancelled { .. } => CANCELLED_MESSAGE.to_owned(),
        }
    }

    /// How the message is presented.
    pub fn severity(&self) -> Severity {
        match self {
            Self::Copied { .. } | Self::Cancelled { .. } => Severity::Info,
            Self::NoneFound => Severity::Warning,
        }
    }
}

/// A bikey could not be copied. Its Display is the message shown under [`WIZARD_TITLE`] with
/// [`Severity::Error`].
#[derive(Debug, thiserror::Error)]
#[error("{}: {source}", path.display())]
pub struct BikeyCopyError {
    /// The file that could not be read or written.
    pub path: PathBuf,

    /// The underlying error.
    #[source]
    pub source: io::Error,
}

/// Finds the bikeys below `source` and copies them into `target`; see [`find_bikeys`] and
/// [`copy_bikeys`].
// Mirrors dao/CommonDAO.java:61-79
pub fn extract(
    source: &Path,
    target: &Path,
    cancel: &CancelToken,
) -> Result<BikeyOutcome, BikeyCopyError> {
    let mut walk = Walk::new(cancel);
    if !walk.visit(source) {
        return Ok(BikeyOutcome::Cancelled {
            found: walk.found.len(),
            copied: 0,
        });
    }
    copy_bikeys(&walk.found, target, cancel)
}

/// Copies each of `found` to `target/<file name>`, in order, so a later key overwrites an earlier
/// one with the same name.
///
/// `cancel` is checked before each copy. The first error aborts the remaining copies. A key that
/// already is its own target (the target folder lies inside the source tree) is left untouched
/// and still counted: copying a file onto itself would truncate it.
pub fn copy_bikeys(
    found: &[PathBuf],
    target: &Path,
    cancel: &CancelToken,
) -> Result<BikeyOutcome, BikeyCopyError> {
    if found.is_empty() {
        return Ok(BikeyOutcome::NoneFound);
    }
    for (copied, key) in found.iter().enumerate() {
        if cancel.is_cancelled() {
            return Ok(BikeyOutcome::Cancelled {
                found: found.len(),
                copied,
            });
        }
        let Some(name) = key.file_name() else {
            continue;
        };
        let destination = target.join(name);
        if is_same_file(key, &destination) {
            continue;
        }
        copy_file_bytes(key, &destination).map_err(|(path, source)| BikeyCopyError {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(BikeyOutcome::Copied { count: found.len() })
}

/// Recursive bikey walk with cancellation and symbolic-link cycle protection.
struct Walk<'a> {
    cancel: &'a CancelToken,
    visited: HashSet<PathBuf>,
    found: Vec<PathBuf>,
}

impl<'a> Walk<'a> {
    fn new(cancel: &'a CancelToken) -> Self {
        Self {
            cancel,
            visited: HashSet::new(),
            found: Vec::new(),
        }
    }

    /// Visits `path`; `false` once cancelled.
    fn visit(&mut self, path: &Path) -> bool {
        if self.cancel.is_cancelled() {
            return false;
        }
        // `fs::metadata` follows symbolic links like Java's `File.isDirectory`; a broken link is
        // tested by name like any file.
        if fs::metadata(path).is_ok_and(|meta| meta.is_dir()) {
            let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !self.visited.insert(real) {
                return true;
            }
            let Ok(entries) = fs::read_dir(path) else {
                return true;
            };
            let mut children: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect();
            children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            for child in children {
                if !self.visit(&child) {
                    return false;
                }
            }
        } else if path.file_name().is_some_and(is_bikey) {
            self.found.push(path.to_path_buf());
        }
        true
    }
}

/// Whether both paths name the same existing file.
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
