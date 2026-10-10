//! Local SHA-1, new/updated detection and hidden folders.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_hash::Sha1Cache;
use a3s_transport::CancellationToken;
use parking_lot::Mutex;

use super::{PlanError, absolute};
use crate::events::{EventSender, PercentCounter, Phase, SyncEvent};
use crate::model::{PlanNode, PlanTree};

/// What [`compute_local_sha1`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HashStats {
    /// Files whose cached hash was still valid.
    pub cache_hits: usize,

    /// Files hashed successfully.
    pub hashed: usize,

    /// Cache entries removed because their file no longer exists.
    pub pruned: usize,
}

/// Sets `local_sha1` on every leaf whose local file exists, using and updating `cache`.
///
/// First removes every cache entry whose file no longer exists (the whole cache, not only this
/// tree's files). Cached hashes are reused while the modification time is unchanged; the other
/// files are hashed in parallel. Each hashed file counts towards a
/// [`SyncEvent::Progress`] of [`Phase::CheckingFiles`]; cache hits don't.
///
/// Hashing continues past a failed file; the first failure in tree order is then returned as
/// [`PlanError::Hash`] (a remote file whose local path is a folder fails this way). Successful
/// hashes stay in the cache. Cancellation returns [`PlanError::Cancelled`] (Java silently skips
/// the remaining files; approved deviation DV-P3).
// Mirrors dao/repository/RepositorySHA1Processor.java:41-177
pub fn compute_local_sha1(
    tree: &mut PlanTree,
    cache: &mut Sha1Cache,
    cancel: &CancellationToken,
    events: &EventSender,
) -> Result<HashStats, PlanError> {
    let mut stats = HashStats::default();

    // Mirrors dao/repository/RepositorySHA1Processor.java:54-67
    let before = cache.entries.len();
    cache.entries.retain(|path, _| path.exists());
    stats.pruned = before - cache.entries.len();

    // Mirrors dao/repository/RepositorySHA1Processor.java:104-165
    let mut queued: Vec<(String, PathBuf)> = Vec::new();
    tree.walk_mut(&mut |relative_path, node| {
        let PlanNode::Leaf(leaf) = node else {
            return;
        };
        leaf.local_sha1 = None;
        let Some(local) = leaf.local_path() else {
            return;
        };
        let path = absolute(&local);
        if !path.exists() {
            return;
        }
        let cached = a3s_hash::modified_millis(&path)
            .ok()
            .and_then(|mtime| cache.get(&path, mtime));
        if let Some(sha1) = cached {
            leaf.local_sha1 = Some(sha1.to_owned());
            stats.cache_hits += 1;
        } else {
            queued.push((relative_path.to_owned(), path));
        }
    });
    if queued.is_empty() {
        return Ok(stats);
    }
    if cancel.is_cancelled() {
        return Err(PlanError::Cancelled);
    }

    let paths: Vec<PathBuf> = queued.iter().map(|(_, p)| p.clone()).collect();
    let stop = AtomicBool::new(false);
    let counter = Mutex::new(PercentCounter::new(paths.len() as u64));
    // `hash_files` only calls back for files hashed successfully.
    // Mirrors dao/repository/RepositorySHA1Processor.java:167-177
    let on_done = |_: &std::path::Path| {
        if cancel.is_cancelled() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(percent) = counter.lock().increment() {
            events.emit(SyncEvent::Progress {
                phase: Phase::CheckingFiles,
                percent,
            });
        }
    };
    let results = a3s_hash::hash_files(&paths, cache, &stop, &on_done);

    let interrupted = results
        .iter()
        .any(|r| matches!(r, Err(e) if e.kind() == std::io::ErrorKind::Interrupted));
    if interrupted || cancel.is_cancelled() {
        return Err(PlanError::Cancelled);
    }

    let mut first_error = None;
    let mut hashed: HashMap<String, String> = HashMap::with_capacity(results.len());
    for ((relative_path, path), result) in queued.into_iter().zip(results) {
        match result {
            Ok(sha1) => {
                hashed.insert(relative_path, sha1);
            }
            Err(source) => {
                if first_error.is_none() {
                    first_error = Some(PlanError::Hash { path, source });
                }
            }
        }
    }
    stats.hashed = hashed.len();
    tree.walk_mut(&mut |relative_path, node| {
        if let PlanNode::Leaf(leaf) = node
            && let Some(sha1) = hashed.remove(relative_path)
        {
            leaf.local_sha1 = Some(sha1);
        }
    });
    match first_error {
        Some(err) => Err(err),
        None => Ok(stats),
    }
}

/// Sets `updated` on every node below the root.
///
/// A folder is updated when its local path does not exist; a file when its local path does not
/// exist, has no local SHA-1, or its local SHA-1 differs from the remote one (a missing remote
/// SHA-1 always differs).
// Mirrors service/RepositoryService.java:513-538
pub fn mark_updated(tree: &mut PlanTree) {
    tree.walk_mut(&mut |_, node| {
        let exists = node.local_path().is_some_and(|p| p.exists());
        match node {
            PlanNode::Directory(d) => d.updated = !exists,
            PlanNode::Leaf(l) => {
                l.updated = !exists || l.local_sha1.is_none() || l.local_sha1 != l.sha1;
            }
        }
    });
}

/// Sets (or clears) `hidden` on every folder below the root with [`is_hidden_path`].
// Mirrors service/RepositoryService.java:540-571
pub fn mark_hidden(tree: &mut PlanTree, hidden_folder_paths: &BTreeSet<String>) {
    tree.walk_mut(&mut |relative_path, node| {
        if let PlanNode::Directory(d) = node {
            d.hidden = is_hidden_path(relative_path, hidden_folder_paths);
        }
    });
}

/// Whether `relative_path` starts with one of `hidden_folder_paths` (with `\` read as `/`), as
/// a plain case-sensitive string prefix.
///
/// Java quirks kept: `"@a"` also hides `"@abc"`, and `""` hides everything.
// Mirrors service/RepositoryService.java:555-589
pub fn is_hidden_path(relative_path: &str, hidden_folder_paths: &BTreeSet<String>) -> bool {
    hidden_folder_paths
        .iter()
        .any(|hidden| relative_path.starts_with(&hidden.replace('\\', "/")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

    use super::*;
    use crate::plan::{NoAddons, assign_destinations};

    const ABC_SHA1: &str = "a9993e364706816aba3e25717850c26c9cd0d89d";

    fn leaf(name: &str, sha1: Option<&str>) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            sha1: sha1.map(str::to_owned),
            size: 3,
            ..Default::default()
        })
    }

    fn dir(name: &str, children: Vec<SyncTreeNode>) -> SyncTreeNode {
        SyncTreeNode::Directory(SyncTreeDirectory {
            name: name.to_owned(),
            children,
            ..Default::default()
        })
    }

    fn planned(children: Vec<SyncTreeNode>, base: &Path) -> PlanTree {
        let mut root = SyncTreeDirectory::new_root();
        root.children = children;
        root.sort_children();
        let mut tree = PlanTree::from_sync_tree(&root);
        assign_destinations(&mut tree, base, false, &NoAddons);
        tree
    }

    fn leaf_of<'a>(tree: &'a PlanTree, path: &str) -> &'a crate::model::PlanLeaf {
        match tree.get(path) {
            Some(PlanNode::Leaf(l)) => l,
            other => panic!("{path}: {other:?}"),
        }
    }

    fn sha1(cache: &mut Sha1Cache, tree: &mut PlanTree) -> Result<HashStats, PlanError> {
        compute_local_sha1(tree, cache, &CancellationToken::new(), &EventSender::noop())
    }

    #[test]
    fn a3_sha1_and_updated() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        std::fs::create_dir_all(base.join("d")).expect("mkdir");
        std::fs::write(base.join("d/same"), b"abc").expect("write");
        std::fs::write(base.join("d/diff"), b"xyz").expect("write");
        std::fs::write(base.join("d/empty"), b"").expect("write");
        std::fs::write(base.join("d/nosha"), b"abc").expect("write");
        let mut tree = planned(
            vec![
                dir(
                    "d",
                    vec![
                        leaf("same", Some(ABC_SHA1)),
                        leaf("diff", Some(ABC_SHA1)),
                        leaf("missing", Some(ABC_SHA1)),
                        leaf("empty", Some("0")),
                        leaf("nosha", None),
                    ],
                ),
                dir("absent", vec![]),
            ],
            base,
        );
        let mut cache = Sha1Cache::default();
        let stats = sha1(&mut cache, &mut tree).expect("hash");
        assert_eq!(
            stats,
            HashStats {
                cache_hits: 0,
                hashed: 4,
                pruned: 0
            }
        );
        assert_eq!(
            leaf_of(&tree, "d/same").local_sha1.as_deref(),
            Some(ABC_SHA1)
        );
        assert_eq!(leaf_of(&tree, "d/empty").local_sha1.as_deref(), Some("0"));
        assert_eq!(leaf_of(&tree, "d/missing").local_sha1, None);

        mark_updated(&mut tree);
        let updated = |p: &str| tree.get(p).is_some_and(PlanNode::updated);
        assert!(!updated("d/same"));
        assert!(updated("d/diff"));
        assert!(updated("d/missing"));
        assert!(!updated("d/empty"));
        assert!(updated("d/nosha"));
        assert!(!updated("d"));
        assert!(updated("absent"));
        assert_eq!(cache.entries.len(), 4);
    }

    #[test]
    fn a3_sha1_cache() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        std::fs::write(base.join("a"), b"abc").expect("write");
        std::fs::write(base.join("b"), b"abc").expect("write");
        let unrelated = base.join("unrelated");
        std::fs::write(&unrelated, b"x").expect("write");
        let a = std::path::absolute(base.join("a")).expect("absolute");
        let b = std::path::absolute(base.join("b")).expect("absolute");
        let mtime = a3s_hash::modified_millis(&a).expect("mtime");

        let mut cache = Sha1Cache::default();
        let entry = |sha1: &str, last_modified| a3s_hash::FileAttributes {
            sha1: sha1.to_owned(),
            last_modified,
        };
        cache.entries.insert(a.clone(), entry("bogus", mtime));
        cache
            .entries
            .insert(b.clone(), entry("stale", mtime - 5000));
        cache
            .entries
            .insert(base.join("deleted"), entry("gone", mtime));
        cache.entries.insert(unrelated.clone(), entry("kept", 0));

        let mut tree = planned(
            vec![leaf("a", Some(ABC_SHA1)), leaf("b", Some(ABC_SHA1))],
            base,
        );
        let stats = sha1(&mut cache, &mut tree).expect("hash");
        assert_eq!(
            stats,
            HashStats {
                cache_hits: 1,
                hashed: 1,
                pruned: 1
            }
        );
        assert_eq!(leaf_of(&tree, "a").local_sha1.as_deref(), Some("bogus"));
        assert_eq!(leaf_of(&tree, "b").local_sha1.as_deref(), Some(ABC_SHA1));
        assert_eq!(
            cache.entries.get(&b).map(|e| e.sha1.as_str()),
            Some(ABC_SHA1)
        );
        assert!(!cache.entries.contains_key(&base.join("deleted")));
        assert!(cache.entries.contains_key(&unrelated));
    }

    #[test]
    fn a3_sha1_progress_events() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        for name in ["a", "b", "c"] {
            std::fs::write(base.join(name), name).expect("write");
        }
        let children = || vec![leaf("a", None), leaf("b", None), leaf("c", None)];
        let mut tree = planned(children(), base);
        let mut cache = Sha1Cache::default();
        let (sender, rx) = EventSender::channel();
        compute_local_sha1(&mut tree, &mut cache, &CancellationToken::new(), &sender)
            .expect("hash");
        let percents: Vec<u8> = rx
            .try_iter()
            .map(|e| match e {
                SyncEvent::Progress {
                    phase: Phase::CheckingFiles,
                    percent,
                } => percent,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(percents, [33, 66, 100]);

        // All hits: no progress.
        let mut tree = planned(children(), base);
        let (sender, rx) = EventSender::channel();
        let stats = compute_local_sha1(&mut tree, &mut cache, &CancellationToken::new(), &sender)
            .expect("hash");
        assert_eq!(stats.cache_hits, 3);
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn a3_sha1_directory_error_keeps_other_hashes() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        std::fs::create_dir(base.join("isdir")).expect("mkdir");
        std::fs::write(base.join("ok"), b"abc").expect("write");
        let mut tree = planned(vec![leaf("isdir", None), leaf("ok", None)], base);
        let mut cache = Sha1Cache::default();
        let err = sha1(&mut cache, &mut tree).expect_err("directory");
        let PlanError::Hash { path, .. } = &err else {
            panic!("{err:?}")
        };
        let expected = std::path::absolute(base.join("isdir")).expect("absolute");
        assert_eq!(path, &expected);
        assert!(
            err.to_string()
                .starts_with(&format!("{} (", expected.display())),
            "{err}"
        );
        assert!(err.to_string().ends_with(')'), "{err}");
        let ok = std::path::absolute(base.join("ok")).expect("absolute");
        assert_eq!(
            cache.entries.get(&ok).map(|e| e.sha1.as_str()),
            Some(ABC_SHA1)
        );
    }

    #[test]
    fn a3_sha1_cancelled() {
        let base = tempfile::tempdir().expect("tempdir");
        let base = base.path();
        std::fs::write(base.join("a"), b"abc").expect("write");
        let mut tree = planned(vec![leaf("a", None)], base);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = compute_local_sha1(
            &mut tree,
            &mut Sha1Cache::default(),
            &cancel,
            &EventSender::noop(),
        )
        .expect_err("cancelled");
        assert!(matches!(err, PlanError::Cancelled), "{err:?}");
    }

    #[test]
    fn a4_hidden_paths() {
        let set = |items: &[&str]| -> BTreeSet<String> {
            items.iter().map(|s| (*s).to_owned()).collect()
        };
        let h = set(&["@a"]);
        assert!(is_hidden_path("@a", &h));
        assert!(is_hidden_path("@a/addons", &h));
        assert!(is_hidden_path("@abc", &h));
        assert!(!is_hidden_path("x/@a", &h));
        assert!(!is_hidden_path("@A", &h));
        assert!(is_hidden_path("mods/sub", &set(&["mods\\sub"])));
        assert!(is_hidden_path("mods/sub/x", &set(&["mods\\sub"])));
        assert!(!is_hidden_path("mods", &set(&["mods\\sub"])));
        assert!(is_hidden_path("anything", &set(&[""])));
        assert!(!is_hidden_path("anything", &set(&[])));
    }

    #[test]
    fn a4_mark_hidden_sets_and_clears() {
        let mut tree = planned(
            vec![
                dir("@a", vec![dir("addons", vec![leaf("x", None)])]),
                dir("@b", vec![]),
            ],
            Path::new("/dest"),
        );
        let hidden = |t: &PlanTree, p: &str| match t.get(p) {
            Some(PlanNode::Directory(d)) => d.hidden,
            other => panic!("{other:?}"),
        };
        mark_hidden(&mut tree, &["@a".to_owned()].into());
        assert!(hidden(&tree, "@a"));
        assert!(hidden(&tree, "@a/addons"));
        assert!(!hidden(&tree, "@b"));
        assert!(!tree.root.hidden);

        mark_hidden(&mut tree, &["@b".to_owned()].into());
        assert!(!hidden(&tree, "@a"));
        assert!(!hidden(&tree, "@a/addons"));
        assert!(hidden(&tree, "@b"));

        mark_hidden(&mut tree, &[String::new()].into());
        assert!(hidden(&tree, "@a") && hidden(&tree, "@b"));
        assert!(!tree.root.hidden, "root untouched");
    }
}
