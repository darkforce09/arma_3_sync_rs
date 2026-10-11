use std::path::PathBuf;

use super::install::InstallKind;
use super::text;
use crate::UpdateError;

/// Why checking for or applying an update failed. `Display` is the message shown to the user.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApplyError {
    /// The configuration or the release metadata could not be read.
    // Mirrors a3sUpdater/service/Service.java:52,64-66
    #[error("Can't read update metadata. {0}")]
    Metadata(#[source] UpdateError),

    /// No release archives are built for this platform.
    #[error("Automatic updates are not available for this platform.")]
    NoTarget,

    /// The installation is of a kind this updater does not replace.
    #[error(
        "This installation cannot be updated automatically ({kind}). Download the new version manually."
    )]
    Unsupported { kind: InstallKind },

    /// The release has no SHA-256 checksum (legacy XML metadata).
    #[error("Update {version} has no SHA-256 checksum and cannot be installed automatically.")]
    Unverified { version: String },

    /// The size request for the archive failed.
    // Mirrors a3sUpdater/service/Service.java:103-111
    #[error("Can't find update file on repository.")]
    SizeUnknown(#[source] UpdateError),

    /// The download failed or the archive does not match its SHA-256 checksum.
    #[error(transparent)]
    Download(UpdateError),

    /// The archive is not a release for this platform or contains user data.
    #[error("{0}")]
    Layout(String),

    /// The archive could not be validated, extracted or copied into the installation.
    // Mirrors a3sUpdater/service/Service.java:167-173
    #[error("Can't write files to the installation directory '{}'. {source}", dir.display())]
    Install {
        dir: PathBuf,
        #[source]
        source: UpdateError,
    },

    /// The installation needs administrator rights and this system has no UAC.
    // Mirrors a3sUpdater/service/ElevationSupport.java:73
    #[error("UAC elevation is only available on Windows.")]
    ElevationUnavailable,

    /// The elevated updater was refused or could not be started.
    #[error("{0}")]
    ElevationNotGranted(String),

    /// The elevated updater started but did not install the update.
    #[error("{0}")]
    ElevatedInstallFailed(String),

    #[error("Update cancelled.")]
    Cancelled,

    /// Another update of the same session is running.
    #[error("An update is already running.")]
    Busy,
}

impl ApplyError {
    /// Whether the update failed for lack of administrator rights.
    pub fn needs_admin(&self) -> bool {
        matches!(
            self,
            Self::ElevationUnavailable | Self::ElevationNotGranted(_)
        )
    }

    /// Whether the error comes from missing file-system permissions
    /// (see [`crate::elevation::is_permission_failure`]).
    pub fn is_permission_failure(&self) -> bool {
        crate::elevation::is_permission_failure(self)
    }

    /// Title and body of the dialog that reports this error; `None` for a cancellation, which
    /// closes the updater silently.
    // Mirrors a3sUpdater/ui/Updater.java:104-160
    pub fn dialog(&self) -> Option<(&'static str, String)> {
        if matches!(self, Self::Cancelled) {
            None
        } else if self.needs_admin() {
            Some((
                text::TITLE_ADMIN_REQUIRED,
                format!("{}\n{self}", text::ADMIN_REQUIRED),
            ))
        } else {
            Some((
                text::TITLE_ERROR,
                format!("An error occurred:\n{self}\n{}", text::ABORTED),
            ))
        }
    }
}
