use std::io;

use a3s_foundation::error::{ErrorKind, UserFacing};

use super::allowlist::Rejected;
use super::crypto::CryptoError;

/// Appended to read errors caused by a broken gzip stream or truncated input.
// Mirrors dao/A3SFilesAccessor.java:26
pub const FILE_CORRUPTED: &str = "The file seems to be corrupted.";

/// Errors reading one of the Java client's files. `file` is the file name only.
///
/// The `Display` texts are Java's (`dao/A3SFilesAccessor.java:65-76, 96-107`).
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be read.
    #[error("Failed to read file: {file}\n{source}")]
    Io {
        file: String,
        #[source]
        source: io::Error,
    },

    /// The gzip stream is broken or the input ends early.
    #[error("Failed to read file: {file}\n{source}\n{FILE_CORRUPTED}")]
    Corrupted {
        file: String,
        #[source]
        source: io::Error,
    },

    /// The serialization stream is invalid, holds a forbidden class or the wrong objects.
    ///
    /// The text follows Java's exception path: "Failed to read file: …" for a filter rejection
    /// or a broken stream, "Failed to deserialize file: …" for unexpected classes.
    #[error("{}", deserialize_text(file, source))]
    Deserialize {
        file: String,
        #[source]
        source: DeserializeError,
    },

    /// The encrypted content of a repository file cannot be decrypted.
    #[error("Failed to decrypt serialized file: {file}")]
    Decrypt {
        file: String,
        #[source]
        source: CryptoError,
    },

    /// A repository file does not hold a `javax.crypto.SealedObject` (Java's
    /// `ClassCastException`, reported like a decryption failure).
    #[error(
        "Failed to decrypt serialized file: {file}\nExpected javax.crypto.SealedObject, found {found}"
    )]
    UnexpectedClass { file: String, found: String },
}

/// Java's message for each kind of deserialization failure.
// Mirrors dao/A3SFilesAccessor.java:65-76, 96-107
fn deserialize_text(file: &str, source: &DeserializeError) -> String {
    match source {
        // `InvalidClassException("filter status: REJECTED")`, an `IOException`.
        DeserializeError::Rejected(_) => {
            format!(
                "Failed to read file: {file}
filter status: REJECTED"
            )
        }
        // `StreamCorruptedException` and friends, `IOException`s.
        DeserializeError::Stream(err) => format!(
            "Failed to read file: {file}
{err}"
        ),
        // A null `encryptedContent` fails inside `SealedObject.getObject`.
        DeserializeError::MissingEncryptedContent => {
            format!("Failed to decrypt serialized file: {file}")
        }
        // Objects of an unknown or unexpected class: `ClassNotFoundException`.
        DeserializeError::Types(_) => format!("Failed to deserialize file: {file}"),
    }
}

/// Why a stream could not be deserialized.
#[derive(Debug, thiserror::Error)]
pub enum DeserializeError {
    /// The Java serialization stream is invalid.
    #[error(transparent)]
    Stream(#[from] a3s_jserial::Error),

    /// The objects do not decode as the expected type.
    #[error(transparent)]
    Types(#[from] a3s_types::Error),

    /// A class outside Java's allowlist.
    #[error("filter status: REJECTED")]
    Rejected(#[from] Rejected),

    /// A `SealedObject` without a byte-array `encryptedContent`.
    #[error("SealedObject has no encrypted content")]
    MissingEncryptedContent,
}

impl Error {
    /// The file name the error is about.
    pub fn file(&self) -> &str {
        match self {
            Self::Io { file, .. }
            | Self::Corrupted { file, .. }
            | Self::Deserialize { file, .. }
            | Self::Decrypt { file, .. }
            | Self::UnexpectedClass { file, .. } => file,
        }
    }
}

impl UserFacing for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Io { source, .. } => ErrorKind::from_io(source),
            Self::Corrupted { .. }
            | Self::Deserialize { .. }
            | Self::Decrypt { .. }
            | Self::UnexpectedClass { .. } => ErrorKind::InvalidData,
        }
    }

    fn user_message(&self) -> String {
        self.to_string()
    }
}
