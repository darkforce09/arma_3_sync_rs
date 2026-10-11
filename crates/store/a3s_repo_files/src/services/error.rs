use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_types::local::{InvalidSetting, ProtocolCheckError};

use crate::local;

/// Errors of the [`super::ClientData`] operations. `Display` is the text shown to the user.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    // Mirrors service/ProfileService.java:71
    #[error("Profile with name {0} already exists.")]
    ProfileExists(String),

    // Mirrors service/ProfileService.java:97, 110
    #[error("Profile with name {0} does not exists.")]
    ProfileNotFound(String),

    // Mirrors ui/profiles/ProfileSelectionDialog.java:240
    #[error("Default profile can't be removed.")]
    DefaultProfileRemoval,

    #[error("Default profile can't be renamed.")]
    DefaultProfileRename,

    #[error("Profile name can't be empty.")]
    EmptyProfileName,

    // Mirrors service/RepositoryService.java:84, 103, 122
    #[error("Repository name can't be empty.")]
    EmptyRepositoryName,

    // Mirrors exception/repository/RepositoryNotFoundException.java:6-8
    #[error("Repository {0} not found!")]
    RepositoryNotFound(String),

    #[error("Repository with name {0} already exists.")]
    RepositoryExists(String),

    /// The protocol factory has no class for the type (A3S); the text names the dialog.
    #[error("{0}")]
    UnsupportedProtocol(&'static str),

    /// A connection field is invalid (`AbstractProtocole.checkData`).
    #[error(transparent)]
    ProtocolCheck(#[from] ProtocolCheckError),

    /// A repository setter rejected its value.
    #[error(transparent)]
    InvalidSetting(#[from] InvalidSetting),

    /// Java `Integer.parseInt` failed.
    #[error("For input string: \"{0}\"")]
    InvalidNumber(String),

    /// A favorite server index outside the list.
    #[error("Index {index} out of bounds for length {len}")]
    FavoriteServerIndex { index: usize, len: usize },

    /// An `update_*` closure changed the name of what it updates; renames have their own
    /// operations.
    #[error("Use the rename operation to change a name.")]
    NameChange,

    /// The local store failed.
    #[error(transparent)]
    Store(#[from] local::Error),

    // Mirrors dao/CommonDAO.java:27-32
    #[error("Failed to read autoconfig file.\n{0}")]
    AutoConfigRead(#[source] crate::Error),

    // Mirrors dao/CommonDAO.java:44-47
    #[error("Failed to write autoconfig file.\n{0}")]
    AutoConfigWrite(#[source] crate::Error),
}

impl UserFacing for ServiceError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::ProfileNotFound(_) | Self::RepositoryNotFound(_) => ErrorKind::NotFound,
            Self::ProfileExists(_)
            | Self::DefaultProfileRemoval
            | Self::DefaultProfileRename
            | Self::EmptyProfileName
            | Self::EmptyRepositoryName
            | Self::RepositoryExists(_)
            | Self::ProtocolCheck(_)
            | Self::InvalidSetting(_)
            | Self::InvalidNumber(_)
            | Self::FavoriteServerIndex { .. }
            | Self::NameChange => ErrorKind::InvalidInput,
            Self::UnsupportedProtocol(_) => ErrorKind::Unsupported,
            Self::Store(err) => err.kind(),
            Self::AutoConfigRead(crate::Error::Io { source, .. }) => ErrorKind::from_io(source),
            Self::AutoConfigRead(_) => ErrorKind::InvalidData,
            Self::AutoConfigWrite(_) => ErrorKind::WriteFailed,
        }
    }

    fn user_message(&self) -> String {
        match self {
            Self::Store(err) => err.user_message(),
            other => other.to_string(),
        }
    }
}
