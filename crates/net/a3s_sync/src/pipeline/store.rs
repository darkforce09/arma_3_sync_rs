//! Persistence of what a synchronization changes, injected by the caller.

use std::collections::{BTreeMap, HashMap};

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_repo_files::hash::Sha1Cache;
use a3s_types::local::{Configuration, Repository};
use parking_lot::Mutex;

/// What an update changed and must be persisted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreChanges {
    /// The repository changed.
    pub repository: bool,

    /// The configuration changed.
    pub configuration: bool,
}

/// Persisting failed.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct StoreError {
    /// What failed.
    pub message: String,

    /// The cause, if any.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl StoreError {
    /// An error without a cause.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// The repository is not stored.
    pub fn repository_not_found(repository: &str) -> Self {
        Self::new(format!("Repository not found: {repository}"))
    }
}

impl UserFacing for StoreError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Internal
    }
}

/// Where the synchronization reads and writes the client data of a repository: its settings
/// and revision, the client configuration, and the SHA-1 cache of the local files.
///
/// The pipeline does no client-data I/O of its own; the application adapts its local store to
/// this trait.
pub trait SyncStore: Send + Sync {
    /// Applies `f` to the stored repository and the configuration, then persists the parts `f`
    /// reports as changed. Fails if the repository is not stored.
    fn update(
        &self,
        repository: &str,
        f: &mut dyn FnMut(&mut Repository, &mut Configuration) -> StoreChanges,
    ) -> Result<(), StoreError>;

    /// The SHA-1 cache of the repository's local files (empty if none).
    fn read_sync_cache(&self, repository: &str) -> Sha1Cache;

    /// Persists the SHA-1 cache of the repository's local files.
    fn write_sync_cache(&self, repository: &str, cache: &Sha1Cache) -> Result<(), StoreError>;
}

/// An in-memory [`SyncStore`] that counts writes, for tests and dry runs.
#[derive(Debug, Default)]
pub struct MemoryStore {
    state: Mutex<MemoryState>,
}

#[derive(Debug, Default)]
struct MemoryState {
    repositories: BTreeMap<String, Repository>,
    configuration: Configuration,
    caches: HashMap<String, Sha1Cache>,
    repository_writes: usize,
    configuration_writes: usize,
}

impl MemoryStore {
    /// A store holding `repositories` and a default configuration.
    pub fn new(repositories: impl IntoIterator<Item = Repository>) -> Self {
        let store = Self::default();
        {
            let mut state = store.state.lock();
            for repository in repositories {
                state
                    .repositories
                    .insert(repository.name.clone(), repository);
            }
        }
        store
    }

    /// A copy of the stored repository.
    pub fn repository(&self, name: &str) -> Option<Repository> {
        self.state.lock().repositories.get(name).cloned()
    }

    /// Replaces the configuration.
    pub fn set_configuration(&self, configuration: Configuration) {
        self.state.lock().configuration = configuration;
    }

    /// A copy of the configuration.
    pub fn configuration(&self) -> Configuration {
        self.state.lock().configuration.clone()
    }

    /// A copy of the stored SHA-1 cache of `name`.
    pub fn sync_cache(&self, name: &str) -> Option<Sha1Cache> {
        self.state.lock().caches.get(name).cloned()
    }

    /// Number of repository and configuration writes so far.
    pub fn writes(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.repository_writes, state.configuration_writes)
    }
}

impl SyncStore for MemoryStore {
    fn update(
        &self,
        repository: &str,
        f: &mut dyn FnMut(&mut Repository, &mut Configuration) -> StoreChanges,
    ) -> Result<(), StoreError> {
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let stored = state
            .repositories
            .get_mut(repository)
            .ok_or_else(|| StoreError::repository_not_found(repository))?;
        let changes = f(stored, &mut state.configuration);
        state.repository_writes += usize::from(changes.repository);
        state.configuration_writes += usize::from(changes.configuration);
        Ok(())
    }

    fn read_sync_cache(&self, repository: &str) -> Sha1Cache {
        self.state
            .lock()
            .caches
            .get(repository)
            .cloned()
            .unwrap_or_default()
    }

    fn write_sync_cache(&self, repository: &str, cache: &Sha1Cache) -> Result<(), StoreError> {
        self.state
            .lock()
            .caches
            .insert(repository.to_owned(), cache.clone());
        Ok(())
    }
}
