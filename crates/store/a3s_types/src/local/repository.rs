use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::InvalidSetting;
use crate::{FavoriteServer, Protocol};

/// A repository known to the client (`fr.soe.a3s.domain.repository.Repository`).
///
/// Holds the persistent settings only. Runtime state (sync status, remote and local metadata,
/// download report) lives elsewhere, and the SHA-1 caches (`mapFilesForSync`,
/// `mapFilesForBuild`) are returned separately by the legacy decoder
/// ([`super::legacy::LegacyRepository`]).
///
/// Connection counts are stored **raw**, as Java does; the getters apply Java's clamps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Repository {
    pub name: String,

    /// Download protocol (`protocole`).
    pub protocol: Protocol,

    /// Unused by current Java code (`bitTorrentProtocole`).
    #[serde(default)]
    pub bit_torrent_protocol: Option<Protocol>,

    /// Last synchronized revision.
    #[serde(default)]
    pub revision: i32,

    /// Notify when the repository is updated.
    #[serde(default)]
    pub notify: bool,

    /// Synchronize automatically.
    #[serde(default)]
    pub auto: bool,

    /// Local folders hidden from the extra-content check (`hidddenFolderPaths`).
    #[serde(default)]
    pub hidden_folder_paths: BTreeSet<String>,

    #[serde(default = "default_true")]
    pub no_auto_discover: bool,

    #[serde(default)]
    pub exact_match: bool,

    #[serde(default)]
    pub default_download_location: Option<String>,

    /// Event name → download location (`mapEventsDownloadLocation`).
    #[serde(default)]
    pub events_download_location: BTreeMap<String, String>,

    /// Client download connections; `0` means "unset" and is filled from the server's
    /// `ServerInfo.numberOfConnections` on the next metadata update.
    #[serde(default)]
    pub number_of_client_connections: i32,

    /// Raw value; see [`Self::repository_check_connections`].
    #[serde(default = "default_check_connections")]
    pub number_of_repository_check_connections: i32,

    /// Download speed limit; `0` means unlimited.
    #[serde(default)]
    pub maximum_client_download_speed: f64,

    /// Upload protocol of a repository administered by this client (`uploadProtocole`).
    #[serde(default)]
    pub upload_protocol: Option<Protocol>,

    #[serde(default)]
    pub upload_compressed_pbo_files_only: bool,

    /// Raw value; see [`Self::parallel_upload_connections`].
    #[serde(default = "default_parallel_upload_connections")]
    pub parallel_upload_connections: i32,

    /// Local folder of a repository built by this client.
    #[serde(default)]
    pub path: Option<String>,

    /// Build setting written raw into `.a3s/serverinfo` (no normalisation, a legacy `0` stays).
    #[serde(default = "default_number_of_connections")]
    pub number_of_connections: i32,

    /// Build setting: create `.zip` files.
    #[serde(default)]
    pub compressed: bool,

    /// Build setting; the inverse of [`Self::use_partial_file_transfer`].
    #[serde(default)]
    pub no_partial_file_transfer: bool,

    /// Build setting (`autoConfigURL`).
    #[serde(default)]
    pub auto_config_url: Option<String>,

    /// Build setting: servers written to `.a3s/autoconfig`.
    #[serde(default)]
    pub favorite_servers_set_to_autoconfig: Vec<FavoriteServer>,

    #[serde(default)]
    pub excluded_files_from_build: BTreeSet<String>,

    #[serde(default)]
    pub excluded_folders_from_sync: BTreeSet<String>,
}

fn default_true() -> bool {
    true
}

fn default_check_connections() -> i32 {
    Repository::DEFAULT_REPOSITORY_CHECK_CONNECTIONS
}

fn default_parallel_upload_connections() -> i32 {
    Repository::DEFAULT_PARALLEL_UPLOAD_CONNECTIONS
}

fn default_number_of_connections() -> i32 {
    1
}

impl Repository {
    /// Default and safety limit for parallel repository content checks.
    pub const DEFAULT_REPOSITORY_CHECK_CONNECTIONS: i32 = 4;
    pub const MAX_REPOSITORY_CHECK_CONNECTIONS: i32 = 4;

    /// Default and safety limit for parallel FTP/SFTP repository uploads.
    pub const DEFAULT_PARALLEL_UPLOAD_CONNECTIONS: i32 = 4;
    pub const MAX_PARALLEL_UPLOAD_CONNECTIONS: i32 = 10;

    /// A repository with the Java field initializer values.
    // Mirrors domain/repository/Repository.java:30-89
    pub fn new(name: impl Into<String>, protocol: Protocol) -> Self {
        Self {
            name: name.into(),
            protocol,
            bit_torrent_protocol: None,
            revision: 0,
            notify: false,
            auto: false,
            hidden_folder_paths: BTreeSet::new(),
            no_auto_discover: true,
            exact_match: false,
            default_download_location: None,
            events_download_location: BTreeMap::new(),
            number_of_client_connections: 0,
            number_of_repository_check_connections: Self::DEFAULT_REPOSITORY_CHECK_CONNECTIONS,
            maximum_client_download_speed: 0.0,
            upload_protocol: None,
            upload_compressed_pbo_files_only: false,
            parallel_upload_connections: Self::DEFAULT_PARALLEL_UPLOAD_CONNECTIONS,
            path: None,
            number_of_connections: 1,
            compressed: false,
            no_partial_file_transfer: false,
            auto_config_url: None,
            favorite_servers_set_to_autoconfig: Vec::new(),
            excluded_files_from_build: BTreeSet::new(),
            excluded_folders_from_sync: BTreeSet::new(),
        }
    }

    /// Effective parallel upload connections: the raw value capped at 10, or 4 if not positive.
    // Mirrors domain/repository/Repository.java:356-360
    pub fn parallel_upload_connections(&self) -> i32 {
        if self.parallel_upload_connections > 0 {
            self.parallel_upload_connections
                .min(Self::MAX_PARALLEL_UPLOAD_CONNECTIONS)
        } else {
            Self::DEFAULT_PARALLEL_UPLOAD_CONNECTIONS
        }
    }

    /// Stores `min(value, 10)`; rejects values below 1.
    // Mirrors domain/repository/Repository.java:362-367
    pub fn set_parallel_upload_connections(&mut self, value: i32) -> Result<(), InvalidSetting> {
        if value < 1 {
            return Err(InvalidSetting {
                message: "Parallel upload connections must be positive.",
            });
        }
        self.parallel_upload_connections = value.min(Self::MAX_PARALLEL_UPLOAD_CONNECTIONS);
        Ok(())
    }

    /// Effective repository check connections: the raw value capped at 4, or 4 if not positive.
    // Mirrors domain/repository/Repository.java:385-390
    pub fn repository_check_connections(&self) -> i32 {
        if self.number_of_repository_check_connections > 0 {
            self.number_of_repository_check_connections
                .min(Self::MAX_REPOSITORY_CHECK_CONNECTIONS)
        } else {
            Self::DEFAULT_REPOSITORY_CHECK_CONNECTIONS
        }
    }

    /// Stores `min(value, 4)`; rejects values below 1.
    // Mirrors domain/repository/Repository.java:392-398
    pub fn set_repository_check_connections(&mut self, value: i32) -> Result<(), InvalidSetting> {
        if value < 1 {
            return Err(InvalidSetting {
                message: "Repository check connections must be positive.",
            });
        }
        self.number_of_repository_check_connections =
            value.min(Self::MAX_REPOSITORY_CHECK_CONNECTIONS);
        Ok(())
    }

    /// Whether the build allows partial (zsync) transfer: `!no_partial_file_transfer`.
    // Mirrors domain/repository/Repository.java:332-338
    pub fn use_partial_file_transfer(&self) -> bool {
        !self.no_partial_file_transfer
    }

    /// Stores the inverse in [`Self::no_partial_file_transfer`].
    // Mirrors domain/repository/Repository.java:340-346
    pub fn set_use_partial_file_transfer(&mut self, value: bool) {
        self.no_partial_file_transfer = !value;
    }
}

/// SHA-1 cache entry of a local file (`fr.soe.a3s.domain.repository.FileAttributes`).
///
/// The Java caches are keyed by absolute local path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttributes {
    /// Lowercase hex SHA-1 (`"0"` for an empty file).
    pub sha1: String,

    /// `File.lastModified()`, in milliseconds since the Unix epoch.
    pub last_modified: i64,
}
