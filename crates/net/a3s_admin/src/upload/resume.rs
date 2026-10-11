//! Resuming an interrupted upload.
//!
//! Java keeps a transient index into its upload list, which counts failed and cancelled files
//! as done and skips the wrong files when the list changes between attempts. Instead,
//! [`UploadResume`] records the entries uploaded successfully together with the local file's
//! length and modification time, and is only valid for the upload target and the local
//! `.a3s/sync` it was recorded against.

use std::collections::BTreeMap;
use std::path::Path;

/// Files and directories uploaded by previous attempts of an upload. Held by the caller in
/// memory, like the transient upload index of Java's `Repository`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadResume {
    /// Key of the upload the entries belong to: target and local `.a3s/sync` (see
    /// [`Self::begin`]).
    key: Option<[u8; 20]>,

    /// Remote path → `(local length, local modification time in ms)`; `(0, 0)` for directories.
    entries: BTreeMap<String, (u64, i64)>,
}

impl UploadResume {
    /// An empty resume state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries recorded as uploaded.
    pub fn completed(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        self.key = None;
        self.entries.clear();
    }

    /// Starts an attempt; entries recorded for another key are dropped. The key is the SHA-1 of
    /// the target (protocol type, host, port, remote path) and of the local `.a3s/sync` bytes.
    pub(crate) fn begin(&mut self, key: [u8; 20]) {
        if self.key != Some(key) {
            self.entries.clear();
            self.key = Some(key);
        }
    }

    /// Whether `transport_path` was uploaded from the same local file (unchanged length and
    /// modification time). `local` is `None` for directories.
    pub(crate) fn is_done(&self, transport_path: &str, local: Option<&Path>) -> bool {
        let Some(recorded) = self.entries.get(transport_path) else {
            return false;
        };
        match local {
            None => true,
            Some(path) => file_stamp(path).is_some_and(|stamp| stamp == *recorded),
        }
    }

    /// Records a successful upload of `transport_path` from `local` (`None` for directories).
    pub(crate) fn record(&mut self, transport_path: &str, local: Option<&Path>) {
        let stamp = match local {
            None => Some((0, 0)),
            Some(path) => file_stamp(path),
        };
        if let Some(stamp) = stamp {
            self.entries.insert(transport_path.to_owned(), stamp);
        }
    }
}

/// `(length, modification time in ms)` of a local file.
fn file_stamp(path: &Path) -> Option<(u64, i64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some((
        metadata.len(),
        i64::try_from(modified.as_millis()).unwrap_or(i64::MAX),
    ))
}
