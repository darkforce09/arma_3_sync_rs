use std::io;

use a3s_error::{ErrorKind, UserFacing};

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
    #[error("Failed to deserialize file: {file}")]
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
    #[error(transparent)]
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
