//! Synchronization settings derived from a client [`Repository`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use a3s_error::{ErrorKind, UserFacing};
use a3s_types::ServerInfo;
use a3s_types::local::Repository;

/// A download aborts once this many files failed.
// Mirrors service/connection/ConnectionDownloadProcessor.java:151,225-227
pub const MAX_DOWNLOAD_ERRORS: usize = 10;

/// Settings of one synchronization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncOptions {
    /// The repository.
    pub repository_name: String,

    /// The selected event (modset), if any.
    pub event_name: Option<String>,

    /// Existing local folder that receives the repository content.
    pub destination: PathBuf,

    /// Look for addons already present elsewhere in the destination.
    pub auto_discover: bool,

    /// Treat local files not in the repository as extra (delete them).
    pub exact_match: bool,

    /// Folders the client must not touch.
    pub hidden_folder_paths: BTreeSet<String>,

    /// Client download connections; 0 = unset.
    pub client_connections: u32,

    /// Download speed limit in bytes per second; 0 = unlimited.
    pub max_download_speed: u64,

    /// Abort after this many failed files ([`MAX_DOWNLOAD_ERRORS`]).
    pub max_errors: usize,
}

impl SyncOptions {
    /// Options for synchronizing `repository` (optionally restricted to `event_name`).
    ///
    /// Resolves the destination with [`resolve_download_location`], which may update the
    /// repository's stored locations (callers persist the repository).
    // Mirrors service/RepositoryService.java:431-439
    pub fn from_repository(
        repository: &mut Repository,
        event_name: Option<&str>,
    ) -> Result<Self, SyncOptionsError> {
        let destination = match resolve_download_location(repository, event_name) {
            Some(path) if !path.is_empty() => path,
            _ => return Err(SyncOptionsError::DestinationEmpty),
        };
        if !Path::new(&destination).exists() {
            return Err(SyncOptionsError::DestinationMissing(destination));
        }
        Ok(Self {
            repository_name: repository.name.clone(),
            event_name: event_name.map(str::to_owned),
            destination: PathBuf::from(destination),
            // Java stores the inverse and the UI inverts it twice.
            // Mirrors service/RepositoryService.java:812-831, ui/repository/DownloadPanel.java:817-821
            auto_discover: !repository.no_auto_discover,
            exact_match: repository.exact_match,
            hidden_folder_paths: repository.hidden_folder_paths.clone(),
            client_connections: u32::try_from(repository.number_of_client_connections).unwrap_or(0),
            // Mirrors ui/repository/dialogs/DownloadSettingsDialog.java:243-247,295-304
            max_download_speed: repository.maximum_client_download_speed.max(0.0) as u64,
            max_errors: MAX_DOWNLOAD_ERRORS,
        })
    }

    /// Number of download connections: `min(client, server)`, where an unset (0) or negative
    /// value counts as 1 and a missing server info as 0.
    // Mirrors service/synchronization/FilesSynchronizationProcessor.java:66-79
    pub fn download_connections(&self, server_info: Option<&ServerInfo>) -> u32 {
        let server = server_info.map_or(0, |s| s.number_of_connections);
        let server = u32::try_from(server).unwrap_or(0).max(1);
        let client = self.client_connections.max(1);
        client.min(server)
    }
}

/// The download folder of `repository` for `event_name` (or the default one).
///
/// An event location that no longer exists is replaced by the default location (or removed if
/// there is none). A default location that no longer exists is cleared.
// Mirrors service/RepositoryService.java:681-727
pub fn resolve_download_location(
    repository: &mut Repository,
    event_name: Option<&str>,
) -> Option<String> {
    if let Some(event_name) = event_name
        && let Some(path) = repository.events_download_location.get(event_name)
    {
        if Path::new(path).exists() {
            return Some(path.clone());
        }
        let default = default_download_location(repository);
        match &default {
            Some(default) => {
                repository
                    .events_download_location
                    .insert(event_name.to_owned(), default.clone());
            }
            None => {
                repository.events_download_location.remove(event_name);
            }
        }
        return default;
    }
    default_download_location(repository)
}

// Mirrors service/RepositoryService.java:710-727
fn default_download_location(repository: &mut Repository) -> Option<String> {
    let path = repository.default_download_location.as_deref()?;
    if !Path::new(path).exists() {
        repository.default_download_location = None;
    }
    repository.default_download_location.clone()
}

/// Invalid synchronization settings.
#[derive(Debug, thiserror::Error)]
pub enum SyncOptionsError {
    /// No download location is set.
    #[error("Default destination folder is empty.")]
    DestinationEmpty,

    /// The download location doesn't exist (the Java message keeps its typo).
    #[error("Default destination folder does not exists: {0}")]
    DestinationMissing(String),
}

impl UserFacing for SyncOptionsError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::DestinationEmpty => ErrorKind::InvalidInput,
            Self::DestinationMissing(_) => ErrorKind::NotFound,
        }
    }
}

#[cfg(test)]
mod tests {
    use a3s_types::{Protocol, ProtocolType};

    use super::*;

    fn repo() -> Repository {
        Repository::new(
            "WOG",
            Protocol::new(ProtocolType::Ftp, "host/path", "21", "anonymous", "", false),
        )
    }

    fn s(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn resolve_download_location_cases() {
        let default = tempfile::tempdir().expect("tempdir");
        let event_dir = tempfile::tempdir().expect("tempdir");
        let missing = default.path().join("missing");

        // No event → default.
        let mut r = repo();
        r.default_download_location = Some(s(default.path()));
        assert_eq!(
            resolve_download_location(&mut r, None),
            Some(s(default.path()))
        );
        // Unknown event → default.
        assert_eq!(
            resolve_download_location(&mut r, Some("e")),
            Some(s(default.path()))
        );

        // Event mapped to an existing dir.
        r.events_download_location
            .insert("e".to_owned(), s(event_dir.path()));
        assert_eq!(
            resolve_download_location(&mut r, Some("e")),
            Some(s(event_dir.path()))
        );

        // Event mapped to a missing dir → default, entry replaced.
        r.events_download_location
            .insert("e".to_owned(), s(&missing));
        assert_eq!(
            resolve_download_location(&mut r, Some("e")),
            Some(s(default.path()))
        );
        assert_eq!(
            r.events_download_location.get("e"),
            Some(&s(default.path()))
        );

        // Event mapped to a missing dir, no default → None, entry removed.
        let mut r = repo();
        r.events_download_location
            .insert("e".to_owned(), s(&missing));
        assert_eq!(resolve_download_location(&mut r, Some("e")), None);
        assert!(r.events_download_location.is_empty());

        // Missing default → None and cleared.
        let mut r = repo();
        r.default_download_location = Some(s(&missing));
        assert_eq!(resolve_download_location(&mut r, None), None);
        assert_eq!(r.default_download_location, None);
    }

    #[test]
    fn from_repository_cases() {
        let mut r = repo();
        let err = SyncOptions::from_repository(&mut r, None).expect_err("empty");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);

        r.default_download_location = Some(String::new());
        let err = SyncOptions::from_repository(&mut r, None).expect_err("empty");
        assert!(matches!(err, SyncOptionsError::DestinationEmpty));

        let missing = SyncOptionsError::DestinationMissing("/nope".to_owned());
        assert_eq!(missing.kind(), ErrorKind::NotFound);

        let dir = tempfile::tempdir().expect("tempdir");
        let mut r = repo();
        r.default_download_location = Some(s(dir.path()));
        r.hidden_folder_paths.insert("@hidden".to_owned());
        r.exact_match = true;
        r.number_of_client_connections = 3;
        r.maximum_client_download_speed = -5.0;
        let o = SyncOptions::from_repository(&mut r, Some("event")).expect("valid");
        assert_eq!(o.repository_name, "WOG");
        assert_eq!(o.event_name.as_deref(), Some("event"));
        assert_eq!(o.destination, dir.path());
        assert!(!o.auto_discover, "no_auto_discover defaults to true");
        assert!(o.exact_match);
        assert!(o.hidden_folder_paths.contains("@hidden"));
        assert_eq!(o.client_connections, 3);
        assert_eq!(o.max_download_speed, 0);
        assert_eq!(o.max_errors, 10);

        r.no_auto_discover = false;
        r.maximum_client_download_speed = 1.5e6;
        r.number_of_client_connections = -2;
        let o = SyncOptions::from_repository(&mut r, None).expect("valid");
        assert!(o.auto_discover);
        assert_eq!(o.max_download_speed, 1_500_000);
        assert_eq!(o.client_connections, 0);
    }

    #[test]
    fn download_connections_table() {
        let options = |client| SyncOptions {
            repository_name: String::new(),
            event_name: None,
            destination: PathBuf::new(),
            auto_discover: false,
            exact_match: false,
            hidden_folder_paths: BTreeSet::new(),
            client_connections: client,
            max_download_speed: 0,
            max_errors: MAX_DOWNLOAD_ERRORS,
        };
        let server = |n| ServerInfo {
            number_of_connections: n,
            ..Default::default()
        };
        assert_eq!(options(0).download_connections(None), 1);
        assert_eq!(options(0).download_connections(Some(&server(5))), 1);
        assert_eq!(options(3).download_connections(Some(&server(5))), 3);
        assert_eq!(options(8).download_connections(Some(&server(5))), 5);
        assert_eq!(options(8).download_connections(Some(&server(0))), 1);
        assert_eq!(options(8).download_connections(Some(&server(-2))), 1);

        // A negative client count clamps to 0 in the options, then counts as 1.
        let dir = tempfile::tempdir().expect("tempdir");
        let mut r = repo();
        r.default_download_location = Some(s(dir.path()));
        r.number_of_client_connections = -2;
        let o = SyncOptions::from_repository(&mut r, None).expect("valid");
        assert_eq!(o.download_connections(Some(&server(0))), 1);
    }
}
