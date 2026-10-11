//! Self-updater core: configuration (`updater.toml`), release lookup (GitHub Releases, JSON
//! manifest, legacy `a3s.xml`), verified download and safe installation of an update archive.
//!
//! Typical flow: [`UpdateConfig::load`] → [`resolve_release`] → [`is_newer`] against the running
//! version → [`download_release`] into a [`StagingDir`] → [`validate_archive`] with the
//! release's SHA-256 → [`stage_archive`] → [`install_staged`] with a [`FileReplacer`]. Replacing the running executable, elevation and relaunch belong to the
//! caller.
//!
//! HTTP functions are `async` and cancellable; archive and installation functions are blocking.

mod archive;
mod config;
mod diag;
mod error;
mod http;
mod release;
mod version;

pub use archive::{
    CopyReplacer, FileReplacer, InstallReport, MANAGED_FILES_NAME, StagedEntry, VerifiedArchive,
    install_staged, is_user_managed, stage_archive, validate_archive,
};
pub use config::{ConfigOverrides, UpdateConfig};
pub use diag::DiagnosticLog;
pub use error::UpdateError;
pub use http::{DownloadOptions, DownloadedArchive, HttpClient, StagingDir, download_release};
pub use release::{
    Release, ReleaseFormat, ResolveRequest, UpdateSource, host_target, parse_github_release,
    parse_json_manifest, parse_legacy_xml, read_local_xml, resolve_release,
};
pub use tokio_util::sync::CancellationToken;
pub use version::{is_newer, is_supported_version};
