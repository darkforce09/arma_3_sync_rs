//! The client-side synchronization plan: a tree mirroring the remote `.a3s/sync` tree, plus the
//! download and delete work items derived from it.
//!
//! Replaces `dto/sync/SyncTree*DTO` and the transient client fields of the domain sync tree
//! (`destinationPath`, `localSHA1`, `complete`, `optional`). Paths are `/`-joined names below the
//! root, without a leading `/`; the root itself has the path `""`.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use a3s_types::java_string::compare_ignore_case;
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

pub use a3s_types::constants::DownloadStatus;

/// Name of the root directory of every plan tree (`racine`, French for "root").
pub const ROOT_NAME: &str = a3s_types::SYNC_ROOT_NAME;

/// The synchronization plan of a repository.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanTree {
    /// The root directory, named [`ROOT_NAME`].
    pub root: PlanDirectory,
}

impl Default for PlanTree {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanTree {
    /// An empty tree (root only).
    pub fn new() -> Self {
        Self {
            root: PlanDirectory::new(ROOT_NAME),
        }
    }

    /// Builds the plan from a remote sync tree, with every client field at its default
    /// (unselected, not optional, no destination, `Pending`) and `changed` computed.
    // Mirrors service/ObjectDTOtransformer.java:292-341
    pub fn from_sync_tree(sync: &SyncTreeDirectory) -> Self {
        let mut root = PlanDirectory::new(ROOT_NAME);
        transform_directory(sync, &mut root);
        let mut tree = Self { root };
        tree.recompute_changed();
        tree
    }

    /// The node at `relative_path`, comparing each segment case-sensitively.
    ///
    /// Leading `/` are ignored, so `"/a"` and `"//a"` are `"a"`. `""` (or only slashes) returns
    /// `None`: the root is a [`PlanDirectory`], use [`Self::root`].
    // Mirrors dto/sync/SyncTreeNodeDTOMethods.java:40-53 (paths come from the tree structure)
    pub fn get(&self, relative_path: &str) -> Option<&PlanNode> {
        let (parent, name) = split_path(relative_path)?;
        let mut dir = &self.root;
        for segment in parent {
            match dir.child(segment)? {
                PlanNode::Directory(d) => dir = d,
                PlanNode::Leaf(_) => return None,
            }
        }
        dir.child(name)
    }

    /// Mutable variant of [`Self::get`].
    pub fn get_mut(&mut self, relative_path: &str) -> Option<&mut PlanNode> {
        let (parent, name) = split_path(relative_path)?;
        let mut dir = &mut self.root;
        for segment in parent {
            match dir.child_mut(segment)? {
                PlanNode::Directory(d) => dir = d,
                PlanNode::Leaf(_) => return None,
            }
        }
        dir.child_mut(name)
    }

    /// The directory at `relative_path`; `""` is the root. Leading `/` are ignored.
    pub fn directory_mut(&mut self, relative_path: &str) -> Option<&mut PlanDirectory> {
        if strip_leading_slashes(relative_path).is_empty() {
            return Some(&mut self.root);
        }
        match self.get_mut(relative_path)? {
            PlanNode::Directory(d) => Some(d),
            PlanNode::Leaf(_) => None,
        }
    }

    /// Visits every node pre-order (a directory before its children) with its relative path.
    /// The root is not visited.
    // Mirrors dto/sync/SyncTreeNodeDTOMethods.java:72-93
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&str, &'a PlanNode)) {
        fn rec<'a>(dir: &'a PlanDirectory, prefix: &str, f: &mut impl FnMut(&str, &'a PlanNode)) {
            for child in &dir.children {
                let path = join_path(prefix, child.name());
                f(&path, child);
                if let PlanNode::Directory(d) = child {
                    rec(d, &path, f);
                }
            }
        }
        rec(&self.root, "", f);
    }

    /// Mutable variant of [`Self::walk`], same order. A directory's children are visited after
    /// `f` has seen (and possibly modified) the directory.
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&str, &mut PlanNode)) {
        fn rec(dir: &mut PlanDirectory, prefix: &str, f: &mut impl FnMut(&str, &mut PlanNode)) {
            for child in &mut dir.children {
                let path = join_path(prefix, child.name());
                f(&path, child);
                if let PlanNode::Directory(d) = child {
                    rec(d, &path, f);
                }
            }
        }
        rec(&mut self.root, "", f);
    }

    /// Relative paths of every leaf, in [`Self::walk`] order.
    // Mirrors dto/sync/SyncTreeNodeDTOMethods.java:95-113
    pub fn leaf_paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.walk(&mut |path, node| {
            if node.is_leaf() {
                out.push(path.to_owned());
            }
        });
        out
    }

    /// Number of leaves in the whole tree.
    pub fn leaf_count(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |_, node| {
            if node.is_leaf() {
                n += 1;
            }
        });
        n
    }

    /// Resets every directory's `changed` flag (root included) and recomputes it: a directory
    /// is changed if any leaf below it is `updated` or `deleted`.
    // Mirrors service/ObjectDTOtransformer.java:302-308
    pub fn recompute_changed(&mut self) {
        fn rec(dir: &mut PlanDirectory) -> bool {
            let mut changed = false;
            for child in &mut dir.children {
                changed |= match child {
                    PlanNode::Directory(d) => rec(d),
                    PlanNode::Leaf(l) => l.updated || l.deleted,
                };
            }
            dir.changed = changed;
            changed
        }
        rec(&mut self.root);
    }

    /// Sets `selected` on the node at `relative_path` and every node below it. `""` selects the
    /// whole tree, root included. Leading `/` are ignored. Returns `false` if there is no such
    /// node.
    pub fn set_selected_recursive(&mut self, relative_path: &str, selected: bool) -> bool {
        fn rec(node: &mut PlanNode, selected: bool) {
            node.set_selected(selected);
            if let PlanNode::Directory(d) = node {
                for child in &mut d.children {
                    rec(child, selected);
                }
            }
        }
        if strip_leading_slashes(relative_path).is_empty() {
            self.root.selected = selected;
            for child in &mut self.root.children {
                rec(child, selected);
            }
            return true;
        }
        match self.get_mut(relative_path) {
            Some(node) => {
                rec(node, selected);
                true
            }
            None => false,
        }
    }
}

fn transform_directory(sync: &SyncTreeDirectory, plan: &mut PlanDirectory) {
    for node in &sync.children {
        match node {
            SyncTreeNode::Leaf(leaf) => plan.children.push(PlanNode::Leaf(transform_leaf(leaf))),
            SyncTreeNode::Directory(dir) => {
                let mut child = PlanDirectory::new(dir.name.clone());
                child.mark_as_addon = dir.mark_as_addon;
                child.updated = dir.updated;
                child.deleted = dir.deleted;
                child.hidden = dir.hidden;
                transform_directory(dir, &mut child);
                plan.children.push(PlanNode::Directory(child));
            }
        }
    }
    // Java re-sorts after every `addTreeNode`; with a stable sort the result is the same as one
    // stable sort of the input order.
    sort_children(&mut plan.children);
}

// Mirrors service/ObjectDTOtransformer.java:326-341
fn transform_leaf(leaf: &SyncTreeLeaf) -> PlanLeaf {
    PlanLeaf {
        sha1: leaf.sha1.clone(),
        size: u64::try_from(leaf.size).unwrap_or(0),
        compressed: leaf.compressed,
        compressed_size: u64::try_from(leaf.compressed_size).unwrap_or(0),
        updated: leaf.updated,
        deleted: leaf.deleted,
        ..PlanLeaf::new(leaf.name.clone())
    }
}

// Mirrors dto/sync/SyncTreeDirectoryDTO.java:99-117
fn sort_children(children: &mut [PlanNode]) {
    children.sort_by(|a, b| {
        a.is_leaf()
            .cmp(&b.is_leaf())
            .then_with(|| compare_ignore_case(a.name(), b.name()))
    });
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

/// `relative_path` without any leading `/` (`"//a/b"` -> `"a/b"`, `"/"` -> `""`).
fn strip_leading_slashes(relative_path: &str) -> &str {
    relative_path.trim_start_matches('/')
}

/// Splits a non-empty relative path into its parent segments and its last segment.
fn split_path(relative_path: &str) -> Option<(std::str::Split<'_, char>, &str)> {
    let relative_path = strip_leading_slashes(relative_path);
    if relative_path.is_empty() {
        return None;
    }
    let parent = parent_relative_path(relative_path);
    let name = file_name(relative_path);
    let mut segments = parent.split('/');
    if parent.is_empty() {
        // `"".split('/')` yields one empty segment; skip it.
        segments.next();
    }
    Some((segments, name))
}

/// Everything before the last `/` of `relative_path`, or `""`.
// Mirrors dto/sync/SyncTreeNodeDTOMethods.java:55-70
pub fn parent_relative_path(relative_path: &str) -> &str {
    relative_path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent)
}

/// The path of a tree node on the server.
///
/// Java stops a node's path at its nearest ancestor named `racine`, so a folder of that name
/// below the root restarts the remote path: `@a/racine/x.pbo` is fetched as `x.pbo`. The local
/// path keeps the whole tree path.
// Mirrors dto/sync/SyncTreeNodeDTOMethods.java:11-41, dao/connection/AbstractConnexionDAO.java:262-280
pub fn remote_relative_path(relative_path: &str) -> &str {
    let relative_path = strip_leading_slashes(relative_path);
    let mut start = 0;
    let mut offset = 0;
    for segment in parent_relative_path(relative_path).split('/') {
        offset += segment.len() + 1;
        if segment == ROOT_NAME {
            start = offset;
        }
    }
    &relative_path[start..]
}

/// The last segment of `relative_path` (the whole path if it has no `/`).
pub fn file_name(relative_path: &str) -> &str {
    relative_path
        .rsplit_once('/')
        .map_or(relative_path, |(_, name)| name)
}

/// A node of the plan tree.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanNode {
    /// A folder.
    Directory(PlanDirectory),

    /// A file.
    Leaf(PlanLeaf),
}

impl PlanNode {
    /// Name of the file or folder.
    pub fn name(&self) -> &str {
        match self {
            Self::Directory(d) => &d.name,
            Self::Leaf(l) => &l.name,
        }
    }

    /// Whether this is a file.
    pub fn is_leaf(&self) -> bool {
        matches!(self, Self::Leaf(_))
    }

    /// Whether the node is selected for download.
    pub fn selected(&self) -> bool {
        match self {
            Self::Directory(d) => d.selected,
            Self::Leaf(l) => l.selected,
        }
    }

    /// Selects or deselects this node only (see [`PlanTree::set_selected_recursive`]).
    pub fn set_selected(&mut self, value: bool) {
        match self {
            Self::Directory(d) => d.selected = value,
            Self::Leaf(l) => l.selected = value,
        }
    }

    /// Whether the node is new or changed locally.
    pub fn updated(&self) -> bool {
        match self {
            Self::Directory(d) => d.updated,
            Self::Leaf(l) => l.updated,
        }
    }

    /// Whether the node is extra content to delete locally.
    pub fn deleted(&self) -> bool {
        match self {
            Self::Directory(d) => d.deleted,
            Self::Leaf(l) => l.deleted,
        }
    }

    /// Local folder that contains this node, once planned.
    pub fn destination_path(&self) -> Option<&Path> {
        match self {
            Self::Directory(d) => d.destination_path.as_deref(),
            Self::Leaf(l) => l.destination_path.as_deref(),
        }
    }

    /// `destination_path/name`, or `None` while the destination is unset.
    pub fn local_path(&self) -> Option<PathBuf> {
        match self {
            Self::Directory(d) => d.local_path(),
            Self::Leaf(l) => l.local_path(),
        }
    }

    /// Download state of the node.
    pub fn download_status(&self) -> DownloadStatus {
        match self {
            Self::Directory(d) => d.download_status,
            Self::Leaf(l) => l.download_status,
        }
    }

    /// Text shown in the tree view.
    pub fn display_name(&self) -> Cow<'_, str> {
        match self {
            Self::Directory(d) => d.display_name(),
            Self::Leaf(l) => Cow::Borrowed(&l.name),
        }
    }
}

/// A folder of the plan (`SyncTreeDirectoryDTO`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanDirectory {
    /// Folder name.
    pub name: String,

    /// Local folder that contains this directory (Java `destinationPath`).
    pub destination_path: Option<PathBuf>,

    /// The folder contains an `addons` sub-folder (an Arma 3 mod).
    pub mark_as_addon: bool,

    /// Missing locally.
    pub updated: bool,

    /// Extra local folder to delete.
    pub deleted: bool,

    /// Hidden by the repository or the user.
    pub hidden: bool,

    /// Optional in the selected event.
    pub optional: bool,

    /// Some leaf below is `updated` or `deleted` (see [`PlanTree::recompute_changed`]).
    pub changed: bool,

    /// Selected for download.
    pub selected: bool,

    /// Download state.
    pub download_status: DownloadStatus,

    /// Children: directories first, then leaves, each group sorted with
    /// [`compare_ignore_case`].
    pub children: Vec<PlanNode>,
}

impl PlanDirectory {
    /// An empty directory with every flag at its default.
    // Mirrors dto/sync/SyncTreeDirectoryDTO.java:12-23
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            destination_path: None,
            mark_as_addon: false,
            updated: false,
            deleted: false,
            hidden: false,
            optional: false,
            changed: false,
            selected: false,
            download_status: DownloadStatus::Pending,
            children: Vec::new(),
        }
    }

    /// Adds a child, keeping the order: directories first, then leaves, each sorted
    /// case-insensitively (stable).
    // Mirrors dto/sync/SyncTreeDirectoryDTO.java:99-117
    pub fn add_child(&mut self, node: PlanNode) {
        self.children.push(node);
        sort_children(&mut self.children);
    }

    /// The direct child named exactly `name`.
    pub fn child(&self, name: &str) -> Option<&PlanNode> {
        self.children.iter().find(|c| c.name() == name)
    }

    /// Mutable variant of [`Self::child`].
    pub fn child_mut(&mut self, name: &str) -> Option<&mut PlanNode> {
        self.children.iter_mut().find(|c| c.name() == name)
    }

    /// `"{name} (optional)"`, else `"{name} (hidden)"`, else the name.
    // Mirrors dto/sync/SyncTreeDirectoryDTO.java:80-89
    pub fn display_name(&self) -> Cow<'_, str> {
        if self.optional {
            Cow::Owned(format!("{} (optional)", self.name))
        } else if self.hidden {
            Cow::Owned(format!("{} (hidden)", self.name))
        } else {
            Cow::Borrowed(&self.name)
        }
    }

    /// `destination_path/name`, or `None` while the destination is unset.
    // Mirrors service/ConnectionService.java:522
    pub fn local_path(&self) -> Option<PathBuf> {
        self.destination_path.as_ref().map(|d| d.join(&self.name))
    }
}

/// A file of the plan (`SyncTreeLeafDTO`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanLeaf {
    /// File name.
    pub name: String,

    /// Local folder that contains this file.
    pub destination_path: Option<PathBuf>,

    /// Remote SHA-1 (lowercase hex; `"0"` for empty files).
    pub sha1: Option<String>,

    /// SHA-1 of the local file, if any.
    pub local_sha1: Option<String>,

    /// Uncompressed size in bytes.
    pub size: u64,

    /// The server has a `<name>.zip` for this file.
    pub compressed: bool,

    /// Size of `<name>.zip` in bytes.
    pub compressed_size: u64,

    /// Missing or different locally.
    pub updated: bool,

    /// Extra local file to delete.
    pub deleted: bool,

    /// Selected for download.
    pub selected: bool,

    /// Optional in the selected event.
    pub optional: bool,

    /// Percentage (`0.0..=100.0`) of the local file that already matches, from zsync.
    pub complete: f64,

    /// Download state.
    pub download_status: DownloadStatus,
}

impl PlanLeaf {
    /// A leaf with every field at its default.
    // Mirrors dto/sync/SyncTreeLeafDTO.java:10-22
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            destination_path: None,
            sha1: None,
            local_sha1: None,
            size: 0,
            compressed: false,
            compressed_size: 0,
            updated: false,
            deleted: false,
            selected: false,
            optional: false,
            complete: 0.0,
            download_status: DownloadStatus::Pending,
        }
    }

    /// `destination_path/name`, or `None` while the destination is unset.
    // Mirrors service/ConnectionService.java:303
    pub fn local_path(&self) -> Option<PathBuf> {
        self.destination_path.as_ref().map(|d| d.join(&self.name))
    }

    /// Bytes to transfer for this file.
    pub fn download_size(&self) -> u64 {
        download_size(
            self.size,
            self.compressed,
            self.compressed_size,
            self.complete,
        )
    }
}

/// Without completion: the compressed size if compressed, else the size. With a completion
/// percentage: the missing part of the uncompressed size, truncated.
// Mirrors service/synchronization/FilesSynchronizationManager.java:92-104
fn download_size(size: u64, compressed: bool, compressed_size: u64, complete: f64) -> u64 {
    if complete == 0.0 {
        if compressed { compressed_size } else { size }
    } else {
        // Java casts the double to long, which truncates (and saturates).
        (size as f64 * (100.0 - complete) / 100.0) as u64
    }
}

/// One file or folder to download (what `AbstractConnexionDAO.downloadFile` reads from a node).
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadTask {
    /// Path in the tree below the repository root, without a leading `/`, even when
    /// auto-discover redirected the local destination. The remote path is
    /// [`remote_relative_path`] of it.
    pub relative_path: String,

    /// Local folder; the file is `destination_dir/name`.
    pub destination_dir: PathBuf,

    /// File or folder details.
    pub kind: DownloadTaskKind,

    /// `Running` means a previous attempt started; a partial file may be resumed.
    pub status: DownloadStatus,
}

/// What a [`DownloadTask`] transfers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DownloadTaskKind {
    /// A folder to create.
    Directory,

    /// A file to download.
    File {
        /// Uncompressed size in bytes.
        size: u64,

        /// Download `<name>.zip` and uncompress it.
        compressed: bool,

        /// Size of `<name>.zip` in bytes.
        compressed_size: u64,

        /// Remote SHA-1.
        sha1: Option<String>,

        /// Completion percentage; non-zero means zsync patching.
        complete: f64,
    },
}

impl DownloadTask {
    /// The task for `node` at `relative_path`, or `None` while its destination is unset.
    // Mirrors dao/connection/AbstractConnexionDAO.java:240-339
    pub fn from_node(relative_path: &str, node: &PlanNode) -> Option<Self> {
        let destination_dir = node.destination_path()?.to_owned();
        let kind = match node {
            PlanNode::Directory(_) => DownloadTaskKind::Directory,
            PlanNode::Leaf(l) => DownloadTaskKind::File {
                size: l.size,
                compressed: l.compressed,
                compressed_size: l.compressed_size,
                sha1: l.sha1.clone(),
                complete: l.complete,
            },
        };
        Some(Self {
            relative_path: relative_path.to_owned(),
            destination_dir,
            kind,
            status: node.download_status(),
        })
    }

    /// File or folder name (last path segment).
    pub fn name(&self) -> &str {
        file_name(&self.relative_path)
    }

    /// Remote parent folder, relative to the repository root.
    pub fn parent_relative_path(&self) -> &str {
        parent_relative_path(&self.relative_path)
    }

    /// `destination_dir/name`.
    pub fn local_path(&self) -> PathBuf {
        self.destination_dir.join(self.name())
    }

    /// Bytes to transfer (0 for folders).
    pub fn download_size(&self) -> u64 {
        match &self.kind {
            DownloadTaskKind::Directory => 0,
            DownloadTaskKind::File {
                size,
                compressed,
                compressed_size,
                complete,
                ..
            } => download_size(*size, *compressed, *compressed_size, *complete),
        }
    }
}

/// One extra local file or folder to delete.
// Mirrors service/ConnectionService.java:513-531
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTask {
    /// Path in the plan tree.
    pub relative_path: String,

    /// Absolute local path.
    pub local_path: PathBuf,

    /// Delete a folder (recursively) rather than a file.
    pub is_directory: bool,
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

    use super::*;

    fn wog_sync() -> SyncTreeDirectory {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/java_repos/wog_ftp/sync");
        a3s_repo_files::read_a3s(&path).expect("fixture decodes")
    }

    fn leaf(name: &str, updated: bool) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            sha1: Some("abc".to_owned()),
            size: 10,
            updated,
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

    /// `@a/addons/x.pbo` (updated), `@a/addons/y.pbo`, `@b/addons/z.pbo`, `readme.txt`.
    fn hand_built() -> SyncTreeDirectory {
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![
            dir(
                "@a",
                vec![dir(
                    "addons",
                    vec![leaf("x.pbo", true), leaf("y.pbo", false)],
                )],
            ),
            dir("@b", vec![dir("addons", vec![leaf("z.pbo", false)])]),
            leaf("readme.txt", false),
        ];
        root
    }

    fn dir_changed(tree: &PlanTree, path: &str) -> bool {
        match tree.get(path) {
            Some(PlanNode::Directory(d)) => d.changed,
            other => panic!("{path}: {other:?}"),
        }
    }

    #[test]
    fn from_sync_tree_matches_the_wog_fixture() {
        let sync = wog_sync();
        let tree = PlanTree::from_sync_tree(&sync);
        assert_eq!(tree.root.name, "racine");
        assert_eq!(tree.leaf_count(), sync.leaf_count());

        let mut expected = Vec::new();
        sync.walk(&mut |path, node| expected.push((path.to_owned(), node.name().to_owned())));
        let mut actual = Vec::new();
        tree.walk(&mut |path, node| {
            actual.push((path.to_owned(), node.name().to_owned()));
            assert_eq!(tree.get(path), Some(node), "{path}");
            assert!(!node.selected());
            assert_eq!(node.destination_path(), None);
            assert_eq!(node.download_status(), DownloadStatus::Pending);
            match node {
                PlanNode::Leaf(l) => {
                    assert!(!l.optional);
                    assert_eq!(l.local_sha1, None);
                    assert!(l.complete == 0.0);
                }
                PlanNode::Directory(d) => {
                    assert!(!d.optional);
                    let first_leaf = d.children.iter().position(PlanNode::is_leaf);
                    if let Some(first_leaf) = first_leaf {
                        assert!(d.children[first_leaf..].iter().all(PlanNode::is_leaf));
                    }
                }
            }
        });
        assert_eq!(actual, expected);

        // Leaf fields are copied.
        sync.walk(&mut |path, node| {
            if let (SyncTreeNode::Leaf(s), Some(PlanNode::Leaf(p))) = (node, tree.get(path)) {
                assert_eq!(p.sha1, s.sha1);
                assert_eq!(p.size as i64, s.size);
                assert_eq!(p.compressed, s.compressed);
                assert_eq!(p.compressed_size as i64, s.compressed_size);
                assert_eq!(p.updated, s.updated);
                assert_eq!(p.deleted, s.deleted);
            }
        });
    }

    #[test]
    fn changed_propagates_to_every_ancestor() {
        let mut tree = PlanTree::from_sync_tree(&hand_built());
        assert!(tree.root.changed);
        assert!(dir_changed(&tree, "@a"));
        assert!(dir_changed(&tree, "@a/addons"));
        assert!(!dir_changed(&tree, "@b"));
        assert!(!dir_changed(&tree, "@b/addons"));

        // Directory flags alone don't count; recompute clears and recomputes.
        if let Some(PlanNode::Leaf(l)) = tree.get_mut("@a/addons/x.pbo") {
            l.updated = false;
        }
        if let Some(PlanNode::Directory(d)) = tree.get_mut("@a") {
            d.updated = true;
        }
        if let Some(PlanNode::Leaf(l)) = tree.get_mut("@b/addons/z.pbo") {
            l.deleted = true;
        }
        tree.recompute_changed();
        assert!(tree.root.changed);
        assert!(!dir_changed(&tree, "@a"));
        assert!(!dir_changed(&tree, "@a/addons"));
        assert!(dir_changed(&tree, "@b"));
        assert!(dir_changed(&tree, "@b/addons"));

        if let Some(PlanNode::Leaf(l)) = tree.get_mut("@b/addons/z.pbo") {
            l.deleted = false;
        }
        tree.recompute_changed();
        assert!(!tree.root.changed);
    }

    #[test]
    fn walk_is_pre_order_and_children_sorted() {
        let mut sync = SyncTreeDirectory::new_root();
        // Out of Java order on purpose.
        sync.children = vec![
            leaf("b.txt", false),
            dir("Zeta", vec![]),
            leaf("A.txt", false),
            dir("alpha", vec![leaf("x", false)]),
        ];
        let tree = PlanTree::from_sync_tree(&sync);
        let mut paths = Vec::new();
        tree.walk(&mut |path, _| paths.push(path.to_owned()));
        assert_eq!(paths, ["alpha", "alpha/x", "Zeta", "A.txt", "b.txt"]);
        assert_eq!(tree.leaf_paths(), ["alpha/x", "A.txt", "b.txt"]);

        let mut tree = tree;
        let mut visited = Vec::new();
        tree.walk_mut(&mut |path, node| {
            visited.push(path.to_owned());
            node.set_selected(true);
        });
        assert_eq!(visited, paths);
        assert!(tree.get("alpha/x").is_some_and(PlanNode::selected));
        assert!(!tree.root.selected);
    }

    #[test]
    fn negative_sizes_become_zero() {
        let mut sync = SyncTreeDirectory::new_root();
        sync.children = vec![SyncTreeNode::Leaf(SyncTreeLeaf {
            name: "x".to_owned(),
            size: -5,
            compressed_size: -1,
            ..Default::default()
        })];
        let tree = PlanTree::from_sync_tree(&sync);
        let Some(PlanNode::Leaf(l)) = tree.get("x") else {
            panic!("leaf")
        };
        assert_eq!((l.size, l.compressed_size), (0, 0));
    }

    #[test]
    fn paths_and_lookups() {
        let mut tree = PlanTree::from_sync_tree(&hand_built());
        assert_eq!(
            tree.get("@a/addons/x.pbo").map(PlanNode::name),
            Some("x.pbo")
        );
        assert!(tree.get("@a/addons/X.pbo").is_none(), "case-sensitive");
        assert!(tree.get("missing").is_none());
        assert!(tree.get("readme.txt/x").is_none());
        assert!(tree.get("").is_none());
        assert!(tree.get("/").is_none());
        assert!(tree.get("//").is_none());
        for path in [
            "/@a/addons/x.pbo",
            "//@a/addons/x.pbo",
            "///@a/addons/x.pbo",
        ] {
            assert_eq!(tree.get(path).map(PlanNode::name), Some("x.pbo"), "{path}");
            assert!(tree.get_mut(path).is_some(), "{path}");
        }
        assert_eq!(
            tree.directory_mut("//").map(|d| d.name.clone()),
            Some("racine".to_owned())
        );
        assert_eq!(
            tree.directory_mut("//@a").map(|d| d.name.clone()),
            Some("@a".to_owned())
        );
        assert_eq!(
            tree.directory_mut("").map(|d| d.name.clone()),
            Some("racine".to_owned())
        );
        assert_eq!(
            tree.directory_mut("@a/addons").map(|d| d.children.len()),
            Some(2)
        );
        assert!(tree.directory_mut("readme.txt").is_none());

        assert_eq!(parent_relative_path(""), "");
        assert_eq!(parent_relative_path("a"), "");
        assert_eq!(parent_relative_path("a/b/c"), "a/b");
        assert_eq!(file_name(""), "");
        assert_eq!(file_name("a"), "a");
        assert_eq!(file_name("a/b/c"), "c");
    }

    #[test]
    fn nested_racine_folder() {
        let mut sync = SyncTreeDirectory::new_root();
        sync.children = vec![dir("@a", vec![dir("racine", vec![leaf("x", true)])])];
        let tree = PlanTree::from_sync_tree(&sync);
        assert_eq!(tree.leaf_paths(), ["@a/racine/x"]);
        assert!(tree.get("@a/racine/x").is_some());
        // Java fetches it from the repository root; the local path keeps the folder.
        assert_eq!(remote_relative_path("@a/racine/x"), "x");
        assert_eq!(remote_relative_path("racine/b/racine/c/x"), "c/x");
        assert_eq!(remote_relative_path("@a/racine"), "@a/racine");
        assert_eq!(remote_relative_path("@a/addons/x.pbo"), "@a/addons/x.pbo");
        assert_eq!(remote_relative_path("x"), "x");
        assert!(dir_changed(&tree, "@a/racine"));
    }

    #[test]
    fn add_child_ordering() {
        let mut d = PlanDirectory::new("d");
        d.add_child(PlanNode::Leaf(PlanLeaf::new("a")));
        d.add_child(PlanNode::Leaf(PlanLeaf::new("_x")));
        d.add_child(PlanNode::Leaf(PlanLeaf::new("B")));
        d.add_child(PlanNode::Leaf(PlanLeaf::new("b")));
        d.add_child(PlanNode::Directory(PlanDirectory::new("z")));
        let names: Vec<_> = d.children.iter().map(PlanNode::name).collect();
        assert_eq!(names, ["z", "_x", "a", "B", "b"]);

        // Equal names keep insertion order.
        let mut d = PlanDirectory::new("d");
        d.add_child(PlanNode::Leaf(PlanLeaf::new("b")));
        d.add_child(PlanNode::Leaf(PlanLeaf::new("B")));
        let names: Vec<_> = d.children.iter().map(PlanNode::name).collect();
        assert_eq!(names, ["b", "B"]);
    }

    #[test]
    fn display_name_precedence() {
        let mut d = PlanDirectory::new("@ace");
        assert_eq!(d.display_name(), "@ace");
        d.hidden = true;
        assert_eq!(d.display_name(), "@ace (hidden)");
        d.optional = true;
        assert_eq!(d.display_name(), "@ace (optional)");
        let node = PlanNode::Directory(d);
        assert_eq!(node.display_name(), "@ace (optional)");
        let mut l = PlanLeaf::new("x.pbo");
        l.optional = true;
        assert_eq!(PlanNode::Leaf(l).display_name(), "x.pbo");
    }

    #[test]
    fn set_selected_recursive_cases() {
        let mut tree = PlanTree::from_sync_tree(&hand_built());
        assert!(tree.set_selected_recursive("@a", true));
        assert!(tree.get("@a").is_some_and(PlanNode::selected));
        assert!(tree.get("@a/addons").is_some_and(PlanNode::selected));
        assert!(tree.get("@a/addons/x.pbo").is_some_and(PlanNode::selected));
        assert!(!tree.get("@b/addons/z.pbo").is_some_and(PlanNode::selected));
        assert!(!tree.root.selected);
        assert!(!tree.set_selected_recursive("missing", true));
        assert!(tree.set_selected_recursive("//@b", true));
        assert!(tree.get("@b/addons/z.pbo").is_some_and(PlanNode::selected));
        assert!(tree.set_selected_recursive("/@b", false));
        assert!(!tree.root.selected);

        assert!(tree.set_selected_recursive("", true));
        assert!(tree.root.selected);
        let mut all = true;
        tree.walk(&mut |_, n| all &= n.selected());
        assert!(all);
        assert!(tree.set_selected_recursive("", false));
        let mut none = true;
        tree.walk(&mut |_, n| none &= !n.selected());
        assert!(none);
    }

    #[test]
    fn download_size_table() {
        let mut l = PlanLeaf::new("x");
        l.size = 1000;
        l.compressed_size = 400;
        assert_eq!(l.download_size(), 1000);
        l.compressed = true;
        assert_eq!(l.download_size(), 400);
        l.complete = 37.5;
        assert_eq!(l.download_size(), 625);
        l.complete = 100.0;
        assert_eq!(l.download_size(), 0);
        l.size = 3;
        l.complete = 50.0;
        assert_eq!(l.download_size(), 1, "truncated");
    }

    #[test]
    fn download_task_from_node() {
        let mut l = PlanLeaf::new("x.pbo");
        l.size = 1000;
        l.compressed = true;
        l.compressed_size = 300;
        l.sha1 = Some("abc".to_owned());
        l.download_status = DownloadStatus::Running;
        let node = PlanNode::Leaf(l.clone());
        assert_eq!(DownloadTask::from_node("@a/addons/x.pbo", &node), None);
        assert_eq!(node.local_path(), None);

        l.destination_path = Some(PathBuf::from("/mods/@a/addons"));
        let node = PlanNode::Leaf(l);
        assert_eq!(
            node.local_path(),
            Some(PathBuf::from("/mods/@a/addons/x.pbo"))
        );
        let task = DownloadTask::from_node("@a/addons/x.pbo", &node).expect("destination set");
        assert_eq!(
            task,
            DownloadTask {
                relative_path: "@a/addons/x.pbo".to_owned(),
                destination_dir: PathBuf::from("/mods/@a/addons"),
                kind: DownloadTaskKind::File {
                    size: 1000,
                    compressed: true,
                    compressed_size: 300,
                    sha1: Some("abc".to_owned()),
                    complete: 0.0,
                },
                status: DownloadStatus::Running,
            }
        );
        assert_eq!(task.name(), "x.pbo");
        assert_eq!(task.parent_relative_path(), "@a/addons");
        assert_eq!(task.local_path(), PathBuf::from("/mods/@a/addons/x.pbo"));
        assert_eq!(task.download_size(), 300);

        // Auto-discover may redirect the destination; the remote path stays the tree path.
        let mut d = PlanDirectory::new("@a");
        d.destination_path = Some(PathBuf::from("/elsewhere"));
        let node = PlanNode::Directory(d);
        assert_eq!(node.local_path(), Some(PathBuf::from("/elsewhere/@a")));
        let task = DownloadTask::from_node("@a", &node).expect("destination set");
        assert_eq!(task.kind, DownloadTaskKind::Directory);
        assert_eq!(task.parent_relative_path(), "");
        assert_eq!(task.local_path(), PathBuf::from("/elsewhere/@a"));
        assert_eq!(task.download_size(), 0);
        assert_eq!(task.status, DownloadStatus::Pending);
    }
}
