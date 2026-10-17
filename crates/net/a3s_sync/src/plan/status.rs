//! What a download of the whole plan does to each file and folder of the repository: the status
//! tree the Repository screen shows, every node with its status and the bytes it needs.

use crate::model::{PlanDirectory, PlanLeaf, PlanNode, PlanTree};

/// What downloading the repository does to a file or folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeStatus {
    /// Already as in the repository.
    UpToDate,

    /// On disk, with content to download or remove.
    Update,

    /// Not on disk yet.
    NotDownloaded,

    /// Not in the repository: a download removes it (exact match only).
    Removed,
}

/// A file or folder of the repository with what a download does to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusNode {
    /// File or folder name.
    pub name: String,

    /// Path below the repository root (`a/b/c`, no leading `/`).
    pub path: String,

    /// What a download does to it.
    pub status: NodeStatus,

    /// Whether it is a folder (folders can be empty).
    pub is_folder: bool,

    /// Hidden by the repository or the user; downloads leave it alone.
    pub hidden: bool,

    /// Bytes to download for it and everything below it.
    pub download_size: u64,

    /// Files to download, itself or below it.
    pub files_to_download: usize,

    /// Files to remove, itself or below it.
    pub files_to_remove: usize,

    /// Children of a folder, in plan order (folders first, then files).
    pub children: Vec<Self>,
}

/// Every top-level file and folder of the repository with its status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusTree {
    /// Top-level nodes, in plan order.
    pub roots: Vec<StatusNode>,
}

/// How many top-level nodes have each status, and what a full download transfers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatusTotals {
    /// Top-level nodes already up to date.
    pub up_to_date: usize,

    /// Top-level nodes with content to download or remove.
    pub to_update: usize,

    /// Top-level nodes not on disk yet.
    pub not_downloaded: usize,

    /// Top-level nodes a download removes.
    pub removed: usize,

    /// Files to download.
    pub files_to_download: usize,

    /// Files to remove.
    pub files_to_remove: usize,

    /// Bytes to download.
    pub download_size: u64,
}

impl StatusTree {
    /// The status of every node of a planned tree (after `mark_updated` and
    /// `add_extra_local_files`), whatever is selected.
    ///
    /// A folder is [`NodeStatus::Removed`] when it is extra, [`NodeStatus::NotDownloaded`] when it
    /// is missing on disk, [`NodeStatus::Update`] when anything below it is not up to date, else
    /// [`NodeStatus::UpToDate`]. A file is removed when it is extra, not downloaded when it has no
    /// local copy, and to update when its local copy differs.
    pub fn from_plan(tree: &PlanTree) -> Self {
        Self {
            roots: children(&tree.root, ""),
        }
    }

    /// Counts of the top-level statuses and the totals of a full download.
    pub fn totals(&self) -> StatusTotals {
        let mut totals = StatusTotals::default();
        for node in &self.roots {
            match node.status {
                NodeStatus::UpToDate => totals.up_to_date += 1,
                NodeStatus::Update => totals.to_update += 1,
                NodeStatus::NotDownloaded => totals.not_downloaded += 1,
                NodeStatus::Removed => totals.removed += 1,
            }
            totals.files_to_download += node.files_to_download;
            totals.files_to_remove += node.files_to_remove;
            totals.download_size = totals.download_size.saturating_add(node.download_size);
        }
        totals
    }

    /// Whether a download has nothing to do.
    pub fn is_up_to_date(&self) -> bool {
        self.roots
            .iter()
            .all(|node| node.status == NodeStatus::UpToDate)
    }

    /// The node at `path` (`a/b/c`).
    pub fn get(&self, path: &str) -> Option<&StatusNode> {
        let mut nodes = &self.roots;
        let mut found = None;
        for segment in path.split('/') {
            let node = nodes.iter().find(|node| node.name == segment)?;
            nodes = &node.children;
            found = Some(node);
        }
        found
    }
}

fn children(directory: &PlanDirectory, prefix: &str) -> Vec<StatusNode> {
    directory
        .children
        .iter()
        .map(|child| {
            let path = if prefix.is_empty() {
                child.name().to_owned()
            } else {
                format!("{prefix}/{}", child.name())
            };
            match child {
                PlanNode::Directory(d) => folder(d, path),
                PlanNode::Leaf(l) => file(l, path),
            }
        })
        .collect()
}

fn folder(directory: &PlanDirectory, path: String) -> StatusNode {
    let children = children(directory, &path);
    let status = if directory.deleted {
        NodeStatus::Removed
    } else if directory.updated {
        NodeStatus::NotDownloaded
    } else if children
        .iter()
        .any(|child| child.status != NodeStatus::UpToDate)
    {
        NodeStatus::Update
    } else {
        NodeStatus::UpToDate
    };
    StatusNode {
        name: directory.name.clone(),
        status,
        is_folder: true,
        hidden: directory.hidden,
        download_size: children
            .iter()
            .map(|child| child.download_size)
            .fold(0, u64::saturating_add),
        files_to_download: children.iter().map(|child| child.files_to_download).sum(),
        files_to_remove: children.iter().map(|child| child.files_to_remove).sum(),
        path,
        children,
    }
}

fn file(leaf: &PlanLeaf, path: String) -> StatusNode {
    let status = if leaf.deleted {
        NodeStatus::Removed
    } else if leaf.updated && leaf.local_sha1.is_none() {
        NodeStatus::NotDownloaded
    } else if leaf.updated {
        NodeStatus::Update
    } else {
        NodeStatus::UpToDate
    };
    StatusNode {
        name: leaf.name.clone(),
        path,
        status,
        is_folder: false,
        hidden: false,
        download_size: if leaf.updated {
            leaf.download_size()
        } else {
            0
        },
        files_to_download: usize::from(leaf.updated && !leaf.deleted),
        files_to_remove: usize::from(leaf.deleted),
        children: Vec::new(),
    }
}
