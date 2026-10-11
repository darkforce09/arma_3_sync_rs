//! Per-repository SHA-1 caches (`Repository.mapFilesForSync` / `mapFilesForBuild`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use a3s_hash::{FileAttributes, Sha1Cache};
use serde::{Deserialize, Serialize};

use super::{CacheKind, Error, LocalStore, toml_file};

/// `[cache]`: absolute path → `{ sha1, last_modified }` (ms), sorted for deterministic output.
// Mirrors dao/repository/RepositorySHA1Processor.java:112-114, 145
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheFile {
    entries: BTreeMap<String, FileAttributes>,
}

impl LocalStore {
    /// Reads a SHA-1 cache. A missing or corrupt file gives an empty cache (a corrupt one with a
    /// warning): a cache can always be rebuilt.
    pub fn read_sha1_cache(&self, name: &str, kind: CacheKind) -> Sha1Cache {
        let path = self.sha1_cache_path(name, kind);
        match toml_file::read::<CacheFile>(&path, "cache") {
            Ok(file) => Sha1Cache {
                entries: file
                    .unwrap_or_default()
                    .entries
                    .into_iter()
                    .map(|(path, attributes)| (PathBuf::from(path), attributes))
                    .collect(),
            },
            Err(err) => {
                tracing::warn!("Ignoring corrupt SHA-1 cache: {}", err.detail());
                Sha1Cache::default()
            }
        }
    }

    /// Writes a SHA-1 cache atomically. Paths that are not valid UTF-8 are skipped.
    pub fn write_sha1_cache(
        &self,
        name: &str,
        kind: CacheKind,
        cache: &Sha1Cache,
    ) -> Result<(), Error> {
        let entries = cache
            .entries
            .iter()
            .filter_map(|(path, attributes)| {
                let key = path.to_str();
                if key.is_none() {
                    tracing::warn!(
                        "Skipping non-UTF-8 path in the SHA-1 cache: {}",
                        path.display()
                    );
                }
                Some((key?.to_owned(), attributes.clone()))
            })
            .collect();
        let file = CacheFile { entries };
        let _guard = self.write_lock.lock();
        toml_file::write(&self.sha1_cache_path(name, kind), "cache", &file)
    }
}
