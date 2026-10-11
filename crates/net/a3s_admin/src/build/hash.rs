//! SHA-1 stage with the modification-time cache.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_hash::{EMPTY_FILE_SHA1, Sha1Cache, hash_files, modified_millis};
use parking_lot::Mutex;

use super::scan::LeafRef;
use super::{BuildError, BuildEvent, Progress};

/// Where a leaf's SHA-1 came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sha1Source {
    /// The file vanished before it could be hashed: the leaf keeps a null `sha1`.
    Missing,

    /// Reused from the cache.
    Cached,

    /// Computed by this build (Java's `updatedFiles`).
    Computed,
}

/// Result of the SHA-1 stage, index-aligned with the deep leaf list.
pub(crate) struct Hashes {
    pub sha1: Vec<Option<String>>,
    pub source: Vec<Sha1Source>,
}

/// Fills the SHA-1 of every leaf, from the cache when the file's mtime is unchanged.
// Mirrors dao/repository/RepositorySHA1Processor.java:53-167
pub(crate) fn hash_leaves(
    leaves: &[LeafRef],
    cache: &mut Sha1Cache,
    cancel: &AtomicBool,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Result<Hashes, BuildError> {
    // Forget files that no longer exist; entries of existing files outside the tree stay.
    cache.entries.retain(|path, _| path.exists());

    let mut hashes = Hashes {
        sha1: vec![None; leaves.len()],
        source: vec![Sha1Source::Missing; leaves.len()],
    };
    let mut misses: Vec<(usize, PathBuf)> = Vec::new();
    for (i, leaf) in leaves.iter().enumerate() {
        if !leaf.path.exists() {
            continue;
        }
        let mtime = modified_millis(&leaf.path).map_err(|source| BuildError::Hash {
            path: leaf.path.clone(),
            source,
        })?;
        // A hit needs the same mtime; the size is not checked.
        if let Some(sha1) = cache.get(&leaf.path, mtime) {
            hashes.sha1[i] = Some(sha1.to_owned());
            hashes.source[i] = Sha1Source::Cached;
        } else {
            misses.push((i, leaf.path.clone()));
        }
    }
    if misses.is_empty() {
        return Ok(hashes);
    }

    let paths: Vec<PathBuf> = misses.iter().map(|(_, p)| p.clone()).collect();
    let progress = Mutex::new(Progress::new(paths.len()));
    let results = hash_files(&paths, cache, cancel, &|_| {
        if let Some(value) = progress.lock().increment_changed() {
            on_event(BuildEvent::Progress(value));
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(BuildError::Cancelled);
    }
    for ((i, path), result) in misses.into_iter().zip(results) {
        match result {
            Ok(sha1) => {
                hashes.sha1[i] = Some(sha1);
                hashes.source[i] = Sha1Source::Computed;
            }
            Err(source) => return Err(BuildError::Hash { path, source }),
        }
    }
    Ok(hashes)
}

/// Whether a leaf's SHA-1 is the empty-file literal.
pub(crate) fn is_empty_file_sha1(sha1: Option<&str>) -> bool {
    sha1 == Some(EMPTY_FILE_SHA1)
}
