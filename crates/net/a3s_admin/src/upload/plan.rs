//! Which nodes of the local sync tree to check, upload or delete remotely.
//!
//! A node's remote path is Java's: it stops at the nearest ancestor named `racine`, so a folder
//! of that name below the root resets the path (`@a/racine/x.pbo` is `x.pbo` on the server,
//! where Java clients download it from). Such folders are not nodes of the plan themselves.
//! Nodes are listed in tree pre-order ("plan order"); the top-level `.a3s` is never deleted.

use std::collections::{BTreeMap, BTreeSet};

use a3s_repo_files::layout::A3S_FOLDER;
use a3s_types::{SYNC_ROOT_NAME, SyncTreeDirectory, SyncTreeNode};

/// A sync tree node selected by [`plan_upload`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedNode {
    /// `/`-joined remote path below the repository root.
    pub relative_path: String,

    /// `/`-joined path in the tree, which is also the local path below the repository folder.
    /// Equals `relative_path` unless an ancestor below the root is named `racine`.
    pub tree_path: String,

    /// A directory (else a file).
    pub is_directory: bool,

    /// The leaf's `compressed` flag (`false` for directories).
    pub compressed: bool,
}

impl PlannedNode {
    fn new(tree_path: &str, relative_path: &str, node: &SyncTreeNode) -> Self {
        let compressed = match node {
            SyncTreeNode::Directory(_) => false,
            SyncTreeNode::Leaf(leaf) => leaf.compressed,
        };
        Self {
            relative_path: relative_path.to_owned(),
            tree_path: tree_path.to_owned(),
            is_directory: matches!(node, SyncTreeNode::Directory(_)),
            compressed,
        }
    }

    /// The remote parent's path (`""` for top-level nodes) and the node's name.
    pub fn parent_and_name(&self) -> (&str, &str) {
        self.relative_path
            .rsplit_once('/')
            .unwrap_or(("", &self.relative_path))
    }

    /// The parent of [`Self::tree_path`] (`""` for top-level nodes).
    pub fn tree_parent(&self) -> &str {
        self.tree_path
            .rsplit_once('/')
            .map_or("", |(parent, _)| parent)
    }
}

/// The three node lists of an upload, each in plan order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadPlan {
    /// Unchanged nodes whose remote files are checked for existence.
    pub to_check: Vec<PlannedNode>,

    /// New or changed nodes.
    pub to_upload: Vec<PlannedNode>,

    /// Remote nodes absent from the local tree. Nodes below a deleted directory are omitted:
    /// deleting the directory removes them.
    pub to_delete: Vec<PlannedNode>,
}

/// Which nodes named `racine` (below the root) a walk leaves out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RacineNodes {
    /// Directories only (Java's deep-search node list).
    // Mirrors dto/sync/SyncTreeNodeDTOMethods.java:50-64
    Directories,

    /// Directories and leaves (`FilesUploadManager.syncToMap`).
    // Mirrors service/administration/FilesUploadManager.java:87-100
    All,
}

/// Every node of `tree` in plan order, with its tree and remote paths.
// Mirrors dto/sync/SyncTreeNodeDTOMethods.java:11-41 (`determinePath`)
pub(crate) fn tree_nodes(tree: &SyncTreeDirectory, skip: RacineNodes) -> Vec<PlannedNode> {
    fn rec(
        dir: &SyncTreeDirectory,
        tree_prefix: &str,
        remote_prefix: &str,
        skip: RacineNodes,
        out: &mut Vec<PlannedNode>,
    ) {
        let join = |prefix: &str, name: &str| {
            if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            }
        };
        for child in &dir.children {
            let tree_path = join(tree_prefix, child.name());
            let remote_path = join(remote_prefix, child.name());
            let racine = child.name() == SYNC_ROOT_NAME;
            match child {
                SyncTreeNode::Directory(d) => {
                    if racine {
                        // The remote path of everything below restarts here.
                        rec(d, &tree_path, "", skip, out);
                    } else {
                        out.push(PlannedNode::new(&tree_path, &remote_path, child));
                        rec(d, &tree_path, &remote_path, skip, out);
                    }
                }
                SyncTreeNode::Leaf(_) => {
                    if !(racine && skip == RacineNodes::All) {
                        out.push(PlannedNode::new(&tree_path, &remote_path, child));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    rec(tree, "", "", skip, &mut out);
    out
}

/// Nodes keyed by remote path, in plan order; a later node with the same remote path replaces
/// an earlier one, like Java's `HashMap.put`.
fn by_remote_path(tree: &SyncTreeDirectory) -> Vec<PlannedNode> {
    let mut nodes: Vec<PlannedNode> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for node in tree_nodes(tree, RacineNodes::All) {
        if let Some(&i) = index.get(&node.relative_path) {
            nodes[i] = node;
        } else {
            index.insert(node.relative_path.clone(), nodes.len());
            nodes.push(node);
        }
    }
    nodes
}

/// Compares the local sync tree with the remote one (`None`: no remote repository yet).
// Mirrors service/administration/FilesUploadManager.java:32-84
pub fn plan_upload(local: &SyncTreeDirectory, remote: Option<&SyncTreeDirectory>) -> UploadPlan {
    let local_nodes = by_remote_path(local);
    let mut plan = UploadPlan::default();
    let Some(remote) = remote else {
        plan.to_upload = local_nodes;
        return plan;
    };
    let remote_nodes = by_remote_path(remote);
    let leaf_sha1 = |tree: &SyncTreeDirectory, node: &PlannedNode| -> Option<String> {
        let mut dir = tree;
        let mut segments = node.tree_path.split('/').peekable();
        while let Some(segment) = segments.next() {
            match dir.child(segment)? {
                SyncTreeNode::Directory(d) if segments.peek().is_some() => dir = d,
                SyncTreeNode::Leaf(leaf) if segments.peek().is_none() => return leaf.sha1.clone(),
                _ => return None,
            }
        }
        None
    };
    let remote_by_path: BTreeMap<&str, &PlannedNode> = remote_nodes
        .iter()
        .map(|node| (node.relative_path.as_str(), node))
        .collect();
    for node in local_nodes {
        let unchanged = match remote_by_path.get(node.relative_path.as_str()) {
            Some(remote_node) if node.is_directory && remote_node.is_directory => true,
            Some(remote_node) if !node.is_directory && !remote_node.is_directory => {
                // A missing SHA-1 counts as different (Java would throw).
                let sha1 = leaf_sha1(local, &node);
                sha1.is_some() && sha1 == leaf_sha1(remote, remote_node)
            }
            _ => false,
        };
        if unchanged {
            plan.to_check.push(node);
        } else {
            plan.to_upload.push(node);
        }
    }
    let local_paths: BTreeSet<&str> = plan
        .to_check
        .iter()
        .chain(&plan.to_upload)
        .map(|n| n.relative_path.as_str())
        .collect();
    let mut deleted_dirs: Vec<String> = Vec::new();
    let mut to_delete = Vec::new();
    for node in remote_nodes {
        let path = node.relative_path.as_str();
        // The repository's metadata folder is never a sync node to delete.
        let is_metadata = path == A3S_FOLDER || path.starts_with(&format!("{A3S_FOLDER}/"));
        if is_metadata || local_paths.contains(path) {
            continue;
        }
        let below_deleted = deleted_dirs.iter().any(|dir| {
            path.strip_prefix(dir.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if below_deleted {
            continue;
        }
        if node.is_directory {
            deleted_dirs.push(node.relative_path.clone());
        }
        to_delete.push(node);
    }
    plan.to_delete = to_delete;
    plan
}
