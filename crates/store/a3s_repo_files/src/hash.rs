//! Parallel SHA-1 hashing of repository files with a modification-time cache.
//!
//! Mirrors `dao/FileAccessMethods.computeSHA1` (lowercase hex, `"0"` for empty files) and
//! `dao/repository/RepositorySHA1Processor` (at most 8 workers, cache keyed by absolute path and
//! last-modified time).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

/// SHA-1 Arma3Sync uses for empty files.
pub const EMPTY_FILE_SHA1: &str = "0";

/// Maximum number of hashing threads (`RepositorySHA1Processor`).
pub const MAX_THREADS: usize = 8;

/// Computes the Arma3Sync SHA-1 of a file.
pub fn sha1_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    if total == 0 {
        return Ok(EMPTY_FILE_SHA1.to_owned());
    }
    Ok(to_hex(&hasher.finalize()))
}

/// Lowercase hex encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Modification time in milliseconds since the Unix epoch (`File.lastModified()`).
pub fn modified_millis(path: &Path) -> io::Result<i64> {
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    })
}

/// A cached hash (`domain.repository.FileAttributes`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttributes {
    pub sha1: String,
    pub last_modified: i64,
}

/// SHA-1 cache keyed by absolute path; entries are reused while the mtime is unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sha1Cache {
    pub entries: HashMap<PathBuf, FileAttributes>,
}

impl Sha1Cache {
    /// Cached hash of `path` if its mtime still matches.
    pub fn get(&self, path: &Path, last_modified: i64) -> Option<&str> {
        self.entries
            .get(path)
            .filter(|a| a.last_modified == last_modified)
            .map(|a| a.sha1.as_str())
    }

    /// Removes entries for paths not in `keep`.
    pub fn retain_paths(&mut self, keep: &std::collections::HashSet<PathBuf>) {
        self.entries.retain(|p, _| keep.contains(p));
    }
}

/// Outcome of hashing one file.
pub type HashResult = io::Result<String>;

/// Hashes `paths` in parallel (at most [`MAX_THREADS`]), using and updating `cache`.
///
/// `on_done` is called once per file, from worker threads. Returns results in input order.
/// When `cancel` becomes true, remaining files fail with [`io::ErrorKind::Interrupted`].
pub fn hash_files(
    paths: &[PathBuf],
    cache: &mut Sha1Cache,
    cancel: &AtomicBool,
    on_done: &(dyn Fn(&Path) + Sync),
) -> Vec<HashResult> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_THREADS);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("sha1-{i}"))
        .build();
    let shared = Mutex::new(std::mem::take(cache));
    let work = || {
        paths
            .par_iter()
            .map(|path| {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                let mtime = modified_millis(path)?;
                if let Some(sha1) = shared.lock().get(path, mtime).map(str::to_owned) {
                    on_done(path);
                    return Ok(sha1);
                }
                let sha1 = sha1_file(path)?;
                shared.lock().entries.insert(
                    path.clone(),
                    FileAttributes {
                        sha1: sha1.clone(),
                        last_modified: mtime,
                    },
                );
                on_done(path);
                Ok(sha1)
            })
            .collect()
    };
    let results = match pool {
        Ok(pool) => pool.install(work),
        Err(_) => work(),
    };
    *cache = shared.into_inner();
    results
}

#[cfg(test)]
mod tests {
    use super::{EMPTY_FILE_SHA1, Sha1Cache, hash_files, sha1_file};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn known_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        let abc = dir.path().join("abc");
        std::fs::write(&empty, b"").unwrap();
        std::fs::write(&abc, b"abc").unwrap();
        assert_eq!(sha1_file(&empty).unwrap(), EMPTY_FILE_SHA1);
        assert_eq!(
            sha1_file(&abc).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );

        let mut cache = Sha1Cache::default();
        let paths = vec![empty.clone(), abc.clone()];
        let results = hash_files(&paths, &mut cache, &AtomicBool::new(false), &|_| {});
        assert_eq!(
            results[1].as_ref().unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(cache.entries.len(), 2);

        // Cached value is used while the mtime is unchanged.
        let mtime = super::modified_millis(&abc).unwrap();
        cache.entries.get_mut(&abc).unwrap().sha1 = "cached".to_owned();
        let results = hash_files(&paths, &mut cache, &AtomicBool::new(false), &|_| {});
        assert_eq!(results[1].as_ref().unwrap(), "cached");
        assert_eq!(cache.get(&abc, mtime), Some("cached"));
    }
}
