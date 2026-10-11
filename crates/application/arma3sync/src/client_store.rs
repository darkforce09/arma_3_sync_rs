//! The synchronization's view of the client data.

use std::sync::Arc;

use a3s_foundation::error::UserFacing;
use a3s_repo_files::hash::Sha1Cache;
use a3s_repo_files::local::CacheKind;
use a3s_repo_files::services::ClientData;
use a3s_sync::pipeline::{StoreChanges, StoreError, SyncStore};
use a3s_types::local::{Configuration, Repository};
use parking_lot::Mutex;

/// A [`SyncStore`] over the shared [`ClientData`]: revision, status and configuration changes
/// go through its services, which write them to the local store at once.
pub(crate) struct ClientStore {
    data: Arc<Mutex<ClientData>>,
}

impl ClientStore {
    pub(crate) fn new(data: Arc<Mutex<ClientData>>) -> Self {
        Self { data }
    }
}

impl SyncStore for ClientStore {
    fn update(
        &self,
        repository: &str,
        f: &mut dyn FnMut(&mut Repository, &mut Configuration) -> StoreChanges,
    ) -> Result<(), StoreError> {
        let mut data = self.data.lock();
        let mut next_repository = data
            .repository(repository)
            .map_err(|_not_found| StoreError::repository_not_found(repository))?
            .clone();
        let mut next_configuration = data.configuration().clone();
        let changes = f(&mut next_repository, &mut next_configuration);
        if changes.repository {
            data.update_repository(repository, move |stored| {
                *stored = next_repository;
                Ok(true)
            })
            .map_err(store_error)?;
        }
        if changes.configuration {
            data.update_configuration(move |stored| {
                *stored = next_configuration;
                true
            })
            .map_err(store_error)?;
        }
        Ok(())
    }

    fn read_sync_cache(&self, repository: &str) -> Sha1Cache {
        let store = Arc::clone(self.data.lock().store());
        store.read_sha1_cache(repository, CacheKind::Sync)
    }

    fn write_sync_cache(&self, repository: &str, cache: &Sha1Cache) -> Result<(), StoreError> {
        let store = Arc::clone(self.data.lock().store());
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
