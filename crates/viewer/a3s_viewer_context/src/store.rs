use std::sync::Arc;

use a3s_foundation::error::UserFacing;
use a3s_repo_files::hash::Sha1Cache;
use a3s_repo_files::local::CacheKind;
use a3s_sync::pipeline::{StoreChanges, StoreError, SyncStore};
use a3s_types::local::{Configuration, Repository};
use parking_lot::RwLock;

use crate::state::AppState;

/// The synchronization's view of the shared [`AppState`].
///
/// Repository and configuration changes go through the client data services, which write them
/// at once. The state lock is held only for those small writes; SHA-1 caches are read and
/// written outside it.
pub struct StateStore {
    state: Arc<RwLock<AppState>>,
}

impl StateStore {
    /// A store over `state`.
    pub fn new(state: Arc<RwLock<AppState>>) -> Self {
        Self { state }
    }
}

impl SyncStore for StateStore {
    fn update(
        &self,
        repository: &str,
        f: &mut dyn FnMut(&mut Repository, &mut Configuration) -> StoreChanges,
    ) -> Result<(), StoreError> {
        let mut state = self.state.write();
        let client = &mut state.client;
        let mut next_repository = client
            .repository(repository)
            .map_err(|_not_found| StoreError::repository_not_found(repository))?
            .clone();
        let mut next_configuration = client.configuration().clone();
        let changes = f(&mut next_repository, &mut next_configuration);
        if changes.repository {
            client
                .update_repository(repository, move |stored| {
                    *stored = next_repository;
                    Ok(true)
                })
                .map_err(store_error)?;
        }
        if changes.configuration {
            client
                .update_configuration(move |stored| {
                    *stored = next_configuration;
                    true
                })
                .map_err(store_error)?;
        }
        Ok(())
    }

    fn read_sync_cache(&self, repository: &str) -> Sha1Cache {
        let store = Arc::clone(self.state.read().client.store());
        store.read_sha1_cache(repository, CacheKind::Sync)
    }

    fn write_sync_cache(&self, repository: &str, cache: &Sha1Cache) -> Result<(), StoreError> {
        let store = Arc::clone(self.state.read().client.store());
        store
            .write_sha1_cache(repository, CacheKind::Sync, cache)
            .map_err(store_error)
    }
}

fn store_error<E: UserFacing + Send + Sync + 'static>(err: E) -> StoreError {
    StoreError {
        message: err.user_message(),
        source: Some(Box::new(err)),
    }
}
