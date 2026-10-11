use std::io;
use std::path::{Path, PathBuf};

use a3s_foundation::error::{ErrorKind, UserFacing};

/// Errors of the local TOML store.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file exists but could not be read.
    #[error("Failed to read file: {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The repositories directory could not be listed (`dao/repository/RepositoryDAO.java:134-142`).
    #[error("Failed to read repository files")]
    ReadRepositories {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A file is not valid TOML or does not match the expected structure.
    #[error("Failed to read file: {}\n{message}", path.display())]
    Parse { path: PathBuf, message: String },

    /// A file has an unsupported `format` (e.g. written by a newer version of the store).
    #[error("Unsupported format {found} in {} (supported: {supported})", path.display())]
    UnsupportedFormat {
        path: PathBuf,
        found: i64,
        supported: u32,
    },

    /// A file could not be written.
    #[error("Failed to write file: {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A profile file could not be written; the user message carries the detail
    /// (`dao/ProfileDAO.java:90-92`).
    #[error("Failed to write file: {}", path.display())]
    WriteProfile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A value could not be serialized to TOML.
    #[error("Failed to write file: {}\n{message}", path.display())]
    Serialize { path: PathBuf, message: String },

    /// A repository file holds a blank name (`dao/repository/RepositoryDAO.java:121-124`).
    #[error("Repository name is missing.")]
    MissingRepositoryName { path: PathBuf },

    /// [`super::WorkshopMetadata::save`] rejected its input; the text is Java's.
    #[error("{0}")]
    InvalidWorkshopEntry(&'static str),
}

/// Java `FileAccessMethods.getCanonicalPath`: the canonical path, else the path as given.
pub(crate) fn display_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_err| path.to_path_buf())
        .display()
        .to_string()
}

impl Error {
    /// Per-file text used in the load summaries: the message and its details, one per line.
    pub fn detail(&self) -> String {
        a3s_foundation::error::format_chain_lines(self)
    }
}

impl UserFacing for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Read { source, .. } | Self::ReadRepositories { source, .. } => {
                ErrorKind::from_io(source)
            }
            Self::Parse { .. }
            | Self::UnsupportedFormat { .. }
            | Self::MissingRepositoryName { .. } => ErrorKind::InvalidData,
            Self::Write { .. } | Self::WriteProfile { .. } | Self::Serialize { .. } => {
                ErrorKind::WriteFailed
            }
            Self::InvalidWorkshopEntry(_) => ErrorKind::InvalidInput,
        }
    }

    fn user_message(&self) -> String {
        match self {
            // Mirrors dao/ConfigurationDAO.java:35, dao/PreferencesDAO.java:26
            Self::Read { path, .. }
            | Self::Parse { path, .. }
            | Self::UnsupportedFormat { path, .. } => {
                format!("Failed to read file: {}", display_path(path))
            }
            // Mirrors dao/ConfigurationDAO.java:51, dao/repository/RepositoryDAO.java:236
            Self::Write { path, .. } | Self::Serialize { path, .. } => {
                format!("Failed to write file: {}", display_path(path))
            }
            // Mirrors dao/ProfileDAO.java:90-92
            Self::WriteProfile { path, source } => {
                format!("Failed to write file: {}\n{source}", display_path(path))
            }
            // Mirrors dao/repository/RepositoryDAO.java:136-141
            Self::ReadRepositories { .. } => self.detail(),
            Self::MissingRepositoryName { .. } | Self::InvalidWorkshopEntry(_) => self.to_string(),
        }
    }
}
