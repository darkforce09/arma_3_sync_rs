use std::io;
use std::path::{Path, PathBuf};

/// Every failure of the updater core.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// `updater.toml` could not be read, parsed or has a value of the wrong type.
    #[error("Invalid updater configuration{}: {message}", path_note(.path.as_deref()))]
    Config {
        path: Option<PathBuf>,
        message: String,
    },

    /// Release metadata (GitHub API, JSON manifest, legacy XML) is malformed or unusable, or the
    /// downloaded archive does not match the announced size.
    #[error("{0}")]
    Manifest(String),

    /// The update server answered outside `200..=299`.
    #[error("Update server returned HTTP {status}.")]
    Http { status: u16 },

    /// Connection, TLS, timeout or body read failure.
    #[error("{0}")]
    Transport(String),

    /// A URL or redirect uses a scheme the configuration does not allow.
    #[error("{0}")]
    InsecureScheme(String),

    /// Release metadata larger than 1 MiB.
    #[error("Update manifest is larger than 1 MiB.")]
    TooLarge,

    /// The downloaded archive does not match its SHA-256 checksum.
    #[error("SHA-256 verification failed for the update archive.")]
    Integrity,

    /// The release has no SHA-256 checksum and unverified downloads were not allowed.
    #[error("The update has no SHA-256 checksum, so its download cannot be verified.")]
    Unverified,

    /// The archive is not a readable ZIP file, an entry fails its CRC check, or a size limit is
    /// exceeded.
    #[error("Update archive is invalid or incomplete: {0}")]
    ArchiveInvalid(String),

    /// The archive has no file entries.
    #[error("Update archive is empty.")]
    ArchiveEmpty,

    /// The archive to install does not exist.
    #[error("Update archive not found: {}", .0.display())]
    ArchiveNotFound(PathBuf),

    /// An archive entry has a blank name.
    #[error("Update archive contains an empty entry name.")]
    EmptyEntryName,

    /// An archive entry or update path that would leave the installation, is absolute, has a
    /// drive or device prefix, or is a symbolic link.
    #[error("Unsafe ZIP entry: {0}")]
    UnsafeEntry(String),

    /// `.a3s-updater-files` exists but is not a regular file.
    #[error("Updater state is not a file: {}", .0.display())]
    StateNotFile(PathBuf),

    /// A line of `.a3s-updater-files` points outside the installation.
    #[error("Unsafe updater state path: {0}")]
    UnsafeStatePath(String),

    /// A staged source file disappeared before it was installed.
    #[error("Update archive extraction is incomplete; source entry is missing: {}", .0.display())]
    MissingSource(PathBuf),

    /// A failure inside one installation stage (`archive validation`, `archive extraction`,
    /// `installation copy`).
    #[error("Update {stage} failed.{} {source}", missing_note(source))]
    Stage {
        stage: &'static str,
        source: Box<Self>,
    },

    /// No release source produced update metadata.
    #[error("No update metadata was found.")]
    NoMetadata,

    /// The JSON manifest has a `targets` table without an entry for this platform.
    #[error("JSON manifest has no asset for target {0}")]
    NoTarget(String),

    /// The operation was cancelled.
    #[error("Update cancelled.")]
    Cancelled,

    /// A local file-system operation failed.
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

impl UpdateError {
    /// An [`UpdateError::Io`] with `context` describing the operation.
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Wraps `self` as the failure of an installation stage.
    // Mirrors a3sUpdater/dao/FtpDAO.java:327-332
    pub(crate) fn in_stage(self, stage: &'static str) -> Self {
        Self::Stage {
            stage,
            source: Box::new(self),
        }
    }
}

fn path_note(path: Option<&Path>) -> String {
    path.map(|p| format!(" {}", p.display()))
        .unwrap_or_default()
}

fn missing_note(source: &UpdateError) -> String {
    match source {
        UpdateError::MissingSource(path) => format!(" Missing path: {}.", path.display()),
        _ => String::new(),
    }
}
