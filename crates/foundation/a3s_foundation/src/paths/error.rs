use std::io;
use std::path::PathBuf;

use crate::error::{ErrorKind, UserFacing, messages};

/// Errors of directory resolution and creation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The user's home directory is unknown (`HOME` / `USERPROFILE` unset or blank).
    #[error("cannot determine the user's home directory")]
    NoHomeDir,

    /// The current executable, needed for portable mode, cannot be located.
    #[error("cannot locate the current executable")]
    CurrentExe(#[source] io::Error),

    /// A directory could not be created.
    #[error("{}", messages::cannot_create_directory(path))]
    CreateDir {
        /// The directory that could not be created, made absolute.
        path: PathBuf,

        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
}

impl UserFacing for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::NoHomeDir => ErrorKind::InvalidInput,
            Self::CurrentExe(_) => ErrorKind::Io,
            // Java reports every failure as `CreateDirectoryException` (a permission problem).
            Self::CreateDir { source, .. } => match ErrorKind::from_io(source) {
                ErrorKind::Io => ErrorKind::PermissionDenied,
                kind => kind,
            },
        }
    }

    fn user_message(&self) -> String {
        match self {
            // Mirrors exception/CreateDirectoryException.java:17-21
            Self::CreateDir { path, .. } => messages::cannot_create_directory(path),
            _ => crate::error::format_chain(self),
        }
    }
}
