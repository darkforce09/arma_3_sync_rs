//! Upload sessions: directory and listing caches that live while an [`UploadSession`] guard
//! exists (`AbstractConnexionDAO.beginUploadSession` / `endUploadSession`).

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

use crate::Transport;

/// Counters of an upload session (`FtpDAO.uploadSessionDirectoryListingCount` and
/// `uploadSessionCachedFileCheckCount`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadSessionStats {
    /// Directory listings fetched to answer `exists` (FTP only).
    pub directory_listings: u64,

    /// `exists` calls answered from a cached directory listing (FTP only).
    pub cached_file_checks: u64,
}

/// RAII guard returned by [`Transport::upload_session`].
///
/// While it is alive the transport caches known directories (all upload protocols) and
/// directory listings (FTP). Dropping it clears the caches and logs the counters.
pub struct UploadSession<'a> {
    transport: &'a mut Transport,
}

impl<'a> UploadSession<'a> {
    pub(crate) fn new(transport: &'a mut Transport) -> Self {
        transport.session_state_mut().begin();
        Self { transport }
    }
}

impl Deref for UploadSession<'_> {
    type Target = Transport;

    fn deref(&self) -> &Transport {
        self.transport
    }
}

impl DerefMut for UploadSession<'_> {
    fn deref_mut(&mut self) -> &mut Transport {
        self.transport
    }
}

impl Drop for UploadSession<'_> {
    fn drop(&mut self) {
        let protocol = self.transport.protocol_name();
        self.transport.session_state_mut().end(protocol);
    }
}

/// Per-transport session caches. Keys are directory paths in the form chosen by the
/// protocol (repository-relative for FTP and WebDAV, absolute for SFTP).
#[derive(Debug, Default)]
pub(crate) struct SessionState {
    active: bool,
    known_dirs: HashSet<String>,
    listings: HashMap<String, HashSet<String>>,
    stats: UploadSessionStats,
}

impl SessionState {
    // Mirrors dao/connection/ftp/FtpDAO.java:58-94 (cache reset at session start).
    fn begin(&mut self) {
        self.active = true;
        self.known_dirs.clear();
        self.listings.clear();
        self.stats = UploadSessionStats::default();
    }

    // Mirrors dao/connection/ftp/FtpDAO.java:96-110.
    fn end(&mut self, protocol: &'static str) {
        if !self.active {
            return;
        }
        tracing::info!(
            protocol,
            directory_listings = self.stats.directory_listings,
            cached_file_checks = self.stats.cached_file_checks,
            "upload session closing"
        );
        self.active = false;
        self.known_dirs.clear();
        self.listings.clear();
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn stats(&self) -> UploadSessionStats {
        self.stats
    }

    pub(crate) fn is_known_dir(&self, dir: &str) -> bool {
        self.active && self.known_dirs.contains(dir)
    }

    /// Records an existing directory.
    pub(crate) fn mark_known_dir(&mut self, dir: &str) {
        if self.active {
            self.known_dirs.insert(dir.to_owned());
        }
    }

    /// Records a directory just created below `parent` with `name`.
    // Mirrors dao/connection/ftp/FtpDAO.java:654-671.
    pub(crate) fn mark_created_dir(&mut self, dir: &str, parent: &str, name: &str) {
        if self.active {
            self.known_dirs.insert(dir.to_owned());
            self.listings.remove(dir);
            self.add_entry(parent, name);
        }
    }

    /// Cached listing of `dir`, counting a cached file check when present.
    pub(crate) fn cached_listing(&mut self, dir: &str) -> Option<&HashSet<String>> {
        if !self.active {
            return None;
        }
        let listing = self.listings.get(dir)?;
        self.stats.cached_file_checks += 1;
        Some(listing)
    }

    /// Stores a freshly fetched listing of `dir` and counts it.
    pub(crate) fn store_listing(&mut self, dir: &str, names: HashSet<String>) {
        if self.active {
            self.stats.directory_listings += 1;
            self.listings.insert(dir.to_owned(), names);
        }
    }

    /// Adds `name` to the cached listing of `dir`, if that listing is cached.
    // Mirrors dao/connection/ftp/FtpDAO.java:673-681.
    pub(crate) fn add_entry(&mut self, dir: &str, name: &str) {
        if let Some(listing) = self.listings.get_mut(dir) {
            listing.insert(name.to_owned());
        }
    }

    /// Forgets a deleted entry `name` of `dir`; `path` is its full key, whose cached
    /// sub-directories and listings are dropped too.
    pub(crate) fn remove_entry(&mut self, dir: &str, name: &str, path: &str) {
        if let Some(listing) = self.listings.get_mut(dir) {
            listing.remove(name);
        }
        self.forget_tree(path);
    }

    /// A known directory vanished: forgets it, everything below it and all its ancestors
    /// (they may be gone too), so that `mkdirs` checks the whole chain again.
    pub(crate) fn forget_chain(&mut self, dir: &str) {
        self.forget_tree(dir);
        let mut ancestor = dir;
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            self.known_dirs.remove(parent);
            self.listings.remove(parent);
            ancestor = parent;
        }
        self.listings.remove("");
    }

    /// Drops `path` and everything below it from the caches (a directory that vanished).
    pub(crate) fn forget_tree(&mut self, path: &str) {
        let below = |key: &String| {
            key == path
                || key
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
        };
        self.known_dirs.retain(|k| !below(k));
        self.listings.retain(|k, _| !below(k));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::SessionState;

    #[test]
    fn forget_chain_drops_ancestors() {
        let mut state = SessionState::default();
        state.begin();
        for dir in ["k", "k/l", "k/l/m", "k2", "x"] {
            state.mark_known_dir(dir);
        }
        state.forget_chain("k/l");
        assert!(!state.is_known_dir("k"));
        assert!(!state.is_known_dir("k/l"));
        assert!(!state.is_known_dir("k/l/m"));
        assert!(state.is_known_dir("k2"));
        assert!(state.is_known_dir("x"));
    }

    #[test]
    fn caches_only_while_active() {
        let mut state = SessionState::default();
        state.mark_known_dir("a");
        assert!(!state.is_known_dir("a"));
        state.begin();
        state.mark_known_dir("a");
        state.mark_known_dir("a/b");
        state.mark_known_dir("ab");
        state.store_listing("a", HashSet::from(["x".to_owned()]));
        assert!(state.cached_listing("a").is_some_and(|l| l.contains("x")));
        state.forget_tree("a");
        assert!(!state.is_known_dir("a/b"));
        assert!(state.is_known_dir("ab"));
        assert_eq!(state.stats().cached_file_checks, 1);
        assert_eq!(state.stats().directory_listings, 1);
        state.end("FTP");
        assert!(!state.is_known_dir("ab"));
        assert!(state.cached_listing("a").is_none());
        assert_eq!(state.stats().directory_listings, 1);
    }
}
