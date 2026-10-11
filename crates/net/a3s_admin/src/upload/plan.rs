//! Which nodes of the local sync tree to check, upload or delete remotely.
//!
//! Paths are computed from the tree structure (`a`, `a/b`, `a/b/c.pbo`), so a folder that
//! happens to be named `racine` below the root keeps its full path. Nodes are listed in tree
//! pre-order ("plan order"), the root excluded.

use std::collections::{BTreeMap, BTreeSet};

use a3s_types::{SyncTreeDirectory, SyncTreeNode};

/// A sync tree node selected by [`plan_upload`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedNode {
    /// `/`-joined path below the repository root.
    pub relative_path: String,

    /// A directory (else a file).
    pub is_directory: bool,

    /// The leaf's `compressed` flag (`false` for directories).
    pub compressed: bool,
}

impl PlannedNode {
    fn new(relative_path: &str, node: &SyncTreeNode) -> Self {
        match node {
            SyncTreeNode::Directory(_) => Self {
                relative_path: relative_path.to_owned(),
                is_directory: true,
                compressed: false,
            },
            SyncTreeNode::Leaf(leaf) => Self {
                relative_path: relative_path.to_owned(),
                is_directory: false,
                compressed: leaf.compressed,
            },
        }
    }

    /// The parent's path (`""` for top-level nodes) and the node's name.
    pub fn parent_and_name(&self) -> (&str, &str) {
        self.relative_path
            .rsplit_once('/')
            .unwrap_or(("", &self.relative_path))
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

/// Nodes of `tree` in plan order with their paths.
fn nodes(tree: &SyncTreeDirectory) -> Vec<(String, &SyncTreeNode)> {
    let mut out = Vec::new();
    tree.walk(&mut |path, node| out.push((path.to_owned(), node)));
    out
}

/// Compares the local sync tree with the remote one (`None`: no remote repository yet).
// Mirrors service/administration/FilesUploadManager.java:32-84
pub fn plan_upload(local: &SyncTreeDirectory, remote: Option<&SyncTreeDirectory>) -> UploadPlan {
    let local_nodes = nodes(local);
    let mut plan = UploadPlan::default();
    let Some(remote) = remote else {
        plan.to_upload = local_nodes
            .iter()
            .map(|(path, node)| PlannedNode::new(path, node))
            .collect();
        return plan;
    };
    let remote_nodes = nodes(remote);
    let remote_by_path: BTreeMap<&str, &SyncTreeNode> = remote_nodes
        .iter()
        .map(|(path, node)| (path.as_str(), *node))
        .collect();
    for (path, node) in &local_nodes {
        let unchanged = match (node, remote_by_path.get(path.as_str())) {
            (SyncTreeNode::Leaf(local), Some(SyncTreeNode::Leaf(remote))) => {
                // A missing SHA-1 counts as different (Java would throw).
                local.sha1.is_some() && local.sha1 == remote.sha1
            }
            (SyncTreeNode::Directory(_), Some(SyncTreeNode::Directory(_))) => true,
            _ => false,
        };
        let planned = PlannedNode::new(path, node);
        if unchanged {
            plan.to_check.push(planned);
        } else {
            plan.to_upload.push(planned);
        }
    }
    let local_paths: BTreeSet<&str> = local_nodes.iter().map(|(p, _)| p.as_str()).collect();
    let mut deleted_dirs: Vec<&str> = Vec::new();
    for (path, node) in &remote_nodes {
        if local_paths.contains(path.as_str()) {
            continue;
        }
        let below_deleted = deleted_dirs.iter().any(|dir| {
            path.strip_prefix(dir)
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if below_deleted {
            continue;
        }
        if matches!(node, SyncTreeNode::Directory(_)) {
            deleted_dirs.push(path);
        }
        plan.to_delete.push(PlannedNode::new(path, node));
    }
    plan
}
