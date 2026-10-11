//! The user's special folders (home, Documents, Desktop) and the folders derived from them.

use std::path::{Path, PathBuf};

use a3s_foundation::paths::{Os, PlatformEnv};

use crate::registry::RegistryRead;
use crate::steam::{find_arma3, proton_paths};

/// The user's special folders.
pub trait SpecialFolders: Send + Sync {
    /// The home folder.
    fn home(&self) -> Option<PathBuf>;

    /// The Documents folder (Java `JShellLink.getDirectory("personal")`).
    fn documents(&self) -> Option<PathBuf>;

    /// The Desktop folder (Java `JShellLink.getDirectory("desktop")`).
    fn desktop(&self) -> Option<PathBuf>;
}

/// The real folders of the current user.
///
/// They come from the Known Folder API on Windows, `user-dirs.dirs` on Linux and the standard
/// folders on macOS. When those know no Documents or Desktop folder, `<home>/Documents` or
/// `<home>/Desktop` is used if it exists.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemFolders;

impl SystemFolders {
    fn with_fallback(
        pick: impl FnOnce(&directories::UserDirs) -> Option<&Path>,
        fallback: &str,
    ) -> Option<PathBuf> {
        let dirs = directories::UserDirs::new()?;
        pick(&dirs).map(Path::to_path_buf).or_else(|| {
            let candidate = dirs.home_dir().join(fallback);
            candidate.is_dir().then_some(candidate)
        })
    }
}

impl SpecialFolders for SystemFolders {
    fn home(&self) -> Option<PathBuf> {
        directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
    }

    fn documents(&self) -> Option<PathBuf> {
        Self::with_fallback(directories::UserDirs::document_dir, "Documents")
    }

    fn desktop(&self) -> Option<PathBuf> {
        Self::with_fallback(directories::UserDirs::desktop_dir, "Desktop")
    }
}

/// Fixed folders, for tests.
#[derive(Clone, Debug, Default)]
pub struct MemoryFolders {
    /// The home folder.
    pub home: Option<PathBuf>,

    /// The Documents folder.
    pub documents: Option<PathBuf>,

    /// The Desktop folder.
    pub desktop: Option<PathBuf>,
}

impl SpecialFolders for MemoryFolders {
    fn home(&self) -> Option<PathBuf> {
        self.home.clone()
    }

    fn documents(&self) -> Option<PathBuf> {
        self.documents.clone()
    }

    fn desktop(&self) -> Option<PathBuf> {
        self.desktop.clone()
    }
}

/// The Arma 3 user folder (profiles, `Arma3.cfg`): `Documents\Arma 3` on Windows; elsewhere the
/// Proton prefix's `Documents/Arma 3` of a Steam-installed Arma 3 (which may not exist yet).
// Mirrors service/LaunchService.java:159-166
pub fn arma3_documents_dir(
    env: &PlatformEnv,
    folders: &dyn SpecialFolders,
    registry: &dyn RegistryRead,
) -> Option<PathBuf> {
    if env.os() == Os::Windows {
        return folders
            .documents()
            .map(|documents| documents.join("Arma 3"));
    }
    find_arma3(env, registry)
        .as_ref()
        .and_then(proton_paths)
        .map(|paths| paths.documents_arma3)
}

/// Where an exported log goes: the Desktop on Windows (home if the Desktop is unknown), home
/// elsewhere. The file name is [`a3s_foundation::paths::EXPORT_LOG_FILE_NAME`].
// Mirrors ui/ErrorLogDialog.java:59-69
pub fn log_export_dir(env: &PlatformEnv, folders: &dyn SpecialFolders) -> Option<PathBuf> {
    if env.os() == Os::Windows {
        folders.desktop().or_else(|| folders.home())
    } else {
        folders.home()
    }
}
