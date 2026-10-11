//! Copying the repository's `userconfig` folder into the Arma 3 installation folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cleanup::copy_directory;
use crate::model::PlanTree;

/// The question asked before copying.
// Mirrors ui/repository/workers/UserconfigUpdater.java:54-56
pub const USERCONFIG_CONFIRM_MESSAGE: &str =
    "Userconfig folder have changed.\nCopy userconfig folder into ArmA 3 installation directory?";

/// What to do with the `userconfig` folder after a download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserconfigAction {
    /// The download did not touch a root `userconfig` folder.
    NotNeeded,

    /// No Arma 3 executable is configured.
    NoArma3Directory,

    /// The destination is the Arma 3 folder: the folder is already in place.
    SameFolder,

    /// Copy the folder.
    Copy(UserconfigCopy),
}

/// A `userconfig` copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserconfigCopy {
    /// The synchronized folder.
    pub source: PathBuf,

    /// The folder in the Arma 3 installation (same name as written in the repository).
    pub target: PathBuf,
}

/// How a `userconfig` copy ended. Nothing here changes the synchronization outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserconfigOutcome {
    /// The synchronized folder does not exist; nothing was done.
    SourceMissing(String),

    /// The target folder could not be created.
    CreateFailed(String),

    /// Copied.
    Copied(String),

    /// The copy failed.
    CopyFailed(String),

    /// The user declined the copy.
    Declined,
}

impl UserconfigOutcome {
    /// The message to show, if any.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::SourceMissing(m)
            | Self::CreateFailed(m)
            | Self::Copied(m)
            | Self::CopyFailed(m) => Some(m),
            Self::Declined => None,
        }
    }
}

/// Whether a `userconfig` copy happens without asking, after asking, or never.
#[derive(Clone)]
pub enum UserconfigPolicy {
    /// Never copy.
    Skip,

    /// Copy without asking (automatic updates).
    Silent,

    /// Ask with [`USERCONFIG_CONFIRM_MESSAGE`]; copy when the callback returns `true`.
    Ask(Arc<dyn Fn(&UserconfigCopy) -> bool + Send + Sync>),
}

impl std::fmt::Debug for UserconfigPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Skip => "Skip",
            Self::Silent => "Silent",
            Self::Ask(_) => "Ask",
        })
    }
}

/// What to do with the folder `userconfig_dir` (the root folder name as written, if the download
/// touched it) of `tree`, synchronized into `destination`, for the Arma 3 executable
/// `arma3_exe_path`.
// Mirrors ui/repository/workers/UserconfigUpdater.java:39-78
pub fn plan_userconfig_copy(
    tree: &PlanTree,
    userconfig_dir: Option<&str>,
    destination: &Path,
    arma3_exe_path: Option<&Path>,
) -> UserconfigAction {
    let Some(name) = userconfig_dir else {
        return UserconfigAction::NotNeeded;
    };
    let Some(arma3_dir) = arma3_exe_path
        .filter(|p| !p.as_os_str().is_empty())
        .and_then(Path::parent)
    else {
        return UserconfigAction::NoArma3Directory;
    };
    if absolute(arma3_dir) == absolute(destination) {
        return UserconfigAction::SameFolder;
    }
    let Some(source) = tree.get(name).and_then(crate::PlanNode::local_path) else {
        return UserconfigAction::NotNeeded;
    };
    UserconfigAction::Copy(UserconfigCopy {
        source,
        target: arma3_dir.join(name),
    })
}

/// Copies the folder. Blocking.
///
/// A missing source does nothing (Java in silent mode still creates an empty target). The
/// target folder is created one level deep; existing files are overwritten and extra files
/// kept.
// Mirrors ui/repository/workers/UserconfigUpdater.java:80-118
pub fn copy_userconfig(copy: &UserconfigCopy) -> UserconfigOutcome {
    let outcome = if !copy.source.exists() {
        UserconfigOutcome::SourceMissing(format!("File not found:{}", copy.source.display()))
    } else {
        if !copy.target.exists()
            && let Err(err) = std::fs::create_dir(&copy.target)
        {
            tracing::debug!("Creating {}: {err}", copy.target.display());
        }
        if copy.target.is_dir() {
            match copy_directory(&copy.source, &copy.target) {
                Ok(_) => UserconfigOutcome::Copied(
                    "Userconfig folder have been updated into ArmA 3 installation directory."
                        .to_owned(),
                ),
                Err(err) => UserconfigOutcome::CopyFailed(err.to_string()),
            }
        } else {
            UserconfigOutcome::CreateFailed(format!(
                "Failed to create directory: {}\nPlease checkout file access permissions.",
                copy.target.display()
            ))
        }
    };
    if let Some(message) = outcome.message() {
        tracing::info!("{message}");
    }
    outcome
}

/// Like Java `File.getAbsolutePath()`: no symlink resolution.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}
