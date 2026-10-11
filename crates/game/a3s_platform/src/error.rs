use std::io;
use std::path::PathBuf;

use a3s_foundation::error::{ErrorKind, UserFacing};

/// Errors of the platform integration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A file operation failed.
    #[error("{}", match path {
        Some(path) => format!("I/O error on {}", path.display()),
        None => "I/O error".to_owned(),
    })]
    Io {
        /// The file or directory involved, when known.
        path: Option<PathBuf>,

        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// A registry key could not be read or written.
    #[error("registry access to {key} failed")]
    Registry {
        /// The full key path, hive included.
        key: String,

        /// The underlying OS error.
        #[source]
        source: io::Error,
    },

    /// The operation is not available on this operating system.
    #[error("This feature is not available for your system.")]
    Unsupported,

    /// A shortcut name that cannot be a file name.
    #[error("invalid shortcut name {0:?}")]
    InvalidShortcutName(String),

    /// The executable a shortcut points to does not exist.
    #[error("shortcut target {} does not exist", .0.display())]
    ShortcutTargetMissing(PathBuf),

    /// The user's desktop folder is unknown.
    #[error("the desktop folder is unknown")]
    NoDesktopDir,

    /// An argument or path that cannot be represented in the target format.
    #[error("{0}")]
    InvalidInput(String),

    /// The `.lnk` writer failed.
    #[error("cannot write the shortcut: {0}")]
    Lnk(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: Some(path.into()),
            source,
        }
    }
}

impl UserFacing for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io { source, .. } => ErrorKind::from_io(source),
            Self::Registry { source, .. } => {
                if source.kind() == io::ErrorKind::PermissionDenied {
                    ErrorKind::PermissionDenied
                } else {
                    ErrorKind::Io
                }
            }
            Self::Unsupported => ErrorKind::Unsupported,
            Self::InvalidShortcutName(_) | Self::InvalidInput(_) | Self::Lnk(_) => {
                ErrorKind::InvalidInput
            }
            Self::ShortcutTargetMissing(_) | Self::NoDesktopDir => ErrorKind::NotFound,
        }
    }
}
