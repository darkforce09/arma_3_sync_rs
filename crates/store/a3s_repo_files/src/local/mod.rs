//! The Rust client's own store: configuration, preferences, profiles, repositories, SHA-1 caches
//! and the Workshop metadata cache, as versioned TOML files.
//!
//! The store replaces Java's `ConfigurationDAO`, `PreferencesDAO`, `ProfileDAO`, `RepositoryDAO`
//! and `WorkshopMetadataStore` file handling. It keeps no in-memory state besides its directories;
//! the load functions port Java's error collection, messages and duplicate rules.
//!
//! Every file is `format = 1` plus one payload table and is written atomically (temporary file,
//! fsync, rename). On Unix the files are created with mode `0600`, as they hold credentials in
//! plain text (Java's AES "encryption" of repositories used a hard-coded key).

mod error;
pub(crate) mod naming;
mod profiles;
mod repositories;
mod settings;
mod sha1_cache;
pub(crate) mod toml_file;
mod workshop;

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub use error::Error;
pub(crate) use naming::file_stem_java;
pub use naming::{file_stem, name_hash, sanitize};
pub use profiles::ProfilesLoad;
pub(crate) use profiles::{ignored_warning, replaced_warning};
pub use repositories::{LoadedRepository, RepositoriesLoad};
pub use workshop::{WorkshopEntry, WorkshopMetadata};

/// Version written in, and accepted from, the `format` key of every file.
pub const FORMAT_VERSION: u32 = 1;

/// Configuration file in the config directory.
pub const CONFIGURATION_FILE: &str = "configuration.toml";

/// Preferences file in the config directory.
pub const PREFERENCES_FILE: &str = "preferences.toml";

/// Workshop metadata cache in the config directory.
pub const WORKSHOP_FILE: &str = "workshop_mods.toml";

/// Marker of a completed import of the Java client's data, in the config directory.
pub const LEGACY_IMPORT_MARKER_FILE: &str = "legacy_import.toml";

/// SHA-1 cache directory below the data directory.
pub const SHA1_CACHE_DIR: &str = "sha1_cache";

/// Extension of every store file.
pub const TOML_EXTENSION: &str = ".toml";

/// The directories of a [`LocalStore`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDirs {
    /// Configuration, preferences, Workshop cache and import marker.
    pub config: PathBuf,

    /// One file per profile.
    pub profiles: PathBuf,

    /// One file per repository.
    pub repositories: PathBuf,

    /// Two cache files per repository.
    pub sha1_cache: PathBuf,
}

impl LocalDirs {
    /// The store directories inside the application directories.
    pub fn from_app_dirs(dirs: &a3s_foundation::paths::AppDirs) -> Self {
        Self {
            config: dirs.config_dir().to_path_buf(),
            profiles: dirs.profiles_dir(),
            repositories: dirs.repositories_dir(),
            sha1_cache: dirs.data_dir().join(SHA1_CACHE_DIR),
        }
    }
}

/// Which SHA-1 cache of a repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheKind {
    /// `Repository.mapFilesForSync`: hashes of the client's downloaded files.
    Sync,

    /// `Repository.mapFilesForBuild`: hashes of an administered repository's files.
    Build,
}

impl CacheKind {
    fn file_suffix(self) -> &'static str {
        match self {
            Self::Sync => ".sync.toml",
            Self::Build => ".build.toml",
        }
    }
}

/// Record of a completed import of the Java client's data (`config/legacy_import.toml`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportMarker {
    /// When the import finished, in ms since the Unix epoch.
    pub imported_at_ms: i64,

    /// Configuration directories of the imported sources.
    pub sources: Vec<String>,

    /// Item counts, keyed `<kind>.<imported|skipped|failed>`.
    pub counts: std::collections::BTreeMap<String, u32>,
}

/// The local TOML store. `Send + Sync`; writes are serialized by one internal lock.
#[derive(Debug)]
pub struct LocalStore {
    dirs: LocalDirs,

    /// Counterpart of Java's `REPOSITORY_WRITE_LOCK`, for every kind of write.
    // Mirrors dao/repository/RepositoryDAO.java:37
    write_lock: Mutex<()>,
}

impl LocalStore {
    /// A store in the application directories. Nothing is created until the first write.
    pub fn new(dirs: &a3s_foundation::paths::AppDirs) -> Self {
        Self::with_dirs(LocalDirs::from_app_dirs(dirs))
    }

    /// A store in explicit directories.
    pub fn with_dirs(dirs: LocalDirs) -> Self {
        Self {
            dirs,
            write_lock: Mutex::new(()),
        }
    }

    /// The store directories.
    pub fn dirs(&self) -> &LocalDirs {
        &self.dirs
    }

    /// `config/configuration.toml`.
    pub fn configuration_path(&self) -> PathBuf {
        self.dirs.config.join(CONFIGURATION_FILE)
    }

    /// `config/preferences.toml`.
    pub fn preferences_path(&self) -> PathBuf {
        self.dirs.config.join(PREFERENCES_FILE)
    }

    /// `config/workshop_mods.toml`.
    pub fn workshop_path(&self) -> PathBuf {
        self.dirs.config.join(WORKSHOP_FILE)
    }

    /// `config/legacy_import.toml`.
    pub fn legacy_import_marker_path(&self) -> PathBuf {
        self.dirs.config.join(LEGACY_IMPORT_MARKER_FILE)
    }

    /// `profiles/<file_stem(name)>.toml`.
    pub fn profile_path(&self, name: &str) -> PathBuf {
        self.dirs
            .profiles
            .join(format!("{}{TOML_EXTENSION}", file_stem(name)))
    }

    /// `repositories/<file_stem(name)>.toml`.
    pub fn repository_path(&self, name: &str) -> PathBuf {
        self.dirs
            .repositories
            .join(format!("{}{TOML_EXTENSION}", file_stem(name)))
    }

    /// `sha1_cache/<file_stem(name)>.{sync,build}.toml`.
    pub fn sha1_cache_path(&self, name: &str, kind: CacheKind) -> PathBuf {
        self.dirs
            .sha1_cache
            .join(format!("{}{}", file_stem(name), kind.file_suffix()))
    }

    /// The Workshop metadata cache.
    pub fn workshop(&self) -> WorkshopMetadata<'_> {
        WorkshopMetadata::new(self)
    }

    /// The marker of a completed legacy import, if one was written and is readable.
    ///
    /// An unreadable marker gives `None` here; use [`Self::read_legacy_import_marker`] to tell
    /// it apart from a missing one.
    pub fn legacy_import_marker(&self) -> Option<ImportMarker> {
        self.read_legacy_import_marker().ok().flatten()
    }

    /// The marker of a completed legacy import: `Ok(None)` when there is none, `Err` when one
    /// exists but cannot be read (or has another `format`).
    pub fn read_legacy_import_marker(&self) -> Result<Option<ImportMarker>, Error> {
        toml_file::read(&self.legacy_import_marker_path(), "import")
    }

    /// Writes the import marker atomically.
    pub fn write_legacy_import_marker(&self, marker: &ImportMarker) -> Result<(), Error> {
        let _guard = self.write_lock.lock();
        toml_file::write(&self.legacy_import_marker_path(), "import", marker)
    }

    /// Deletes `path` (the caller holds the write lock); `Ok(false)` when it did not exist.
    fn delete_file(path: &Path) -> Result<bool, Error> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(Error::Write {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}
