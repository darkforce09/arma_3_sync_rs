//! Selection totals and work lists (port of `FilesSynchronizationManager`).

use super::is_safe_name;
use crate::model::{
    DeleteTask, DownloadStatus, DownloadTask, DownloadTaskKind, PlanDirectory, PlanNode, PlanTree,
};

/// What a selection of the plan amounts to.
///
/// `download_tasks` is the work list of the download (WP09 mutates the statuses);
/// `delete_tasks`, [`downloaded_compressed`], [`ReportTotals`] and `userconfig_dir` feed the
/// post-download steps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanSummary {
    /// Selected `updated` nodes, pre-order (a folder before its content). Java
    /// `listFilesToUpdate`.
    pub download_tasks: Vec<DownloadTask>,

    /// Selected `deleted` nodes, pre-order. Java `listFilesToDelete`.
    pub delete_tasks: Vec<DeleteTask>,

    /// Bytes to download for the selected updated files.
    pub total_download_size: u64,

    /// Selected nodes (folders included; the root too when selected).
    pub files_selected: usize,

    /// `updated` nodes, selected or not (folders included).
    pub files_updated: usize,

    /// `deleted` nodes that are not `updated`, selected or not (folders included).
    pub files_deleted: usize,

    /// Name (as written) of the root `userconfig` folder when the download touches it.
    pub userconfig_dir: Option<String>,
}

impl PlanSummary {
    /// Totals and work lists of the current selection of `tree`, root included.
    // Mirrors service/synchronization/FilesSynchronizationManager.java:31-90
    pub fn compute(tree: &PlanTree) -> Self {
        let mut summary = Self::default();
        summary.count_root(&tree.root);
        tree.walk(&mut |relative_path, node| summary.add_node(relative_path, node));
        summary.userconfig_dir = userconfig_dir(&summary.download_tasks);
        summary
    }

    /// Whether the download touches the root `userconfig` folder.
    pub fn is_userconfig_updated(&self) -> bool {
        self.userconfig_dir.is_some()
    }

    /// The root is counted like any folder; it has no local path, so it never becomes a task.
    fn count_root(&mut self, root: &PlanDirectory) {
        self.count(root.updated, root.deleted, root.selected);
        if root.selected && (root.updated || root.deleted) {
            tracing::warn!("The selected root folder cannot be synchronized");
        }
    }

    fn count(&mut self, updated: bool, deleted: bool, selected: bool) {
        if updated {
            self.files_updated += 1;
        } else if deleted {
            self.files_deleted += 1;
        }
        if selected {
            self.files_selected += 1;
        }
    }

    fn add_node(&mut self, relative_path: &str, node: &PlanNode) {
        self.count(node.updated(), node.deleted(), node.selected());
        // Planning rejects such trees (D-15); never build a task from one anyway.
        let safe = is_safe_name(node.name()) && relative_path.split('/').all(is_safe_name);
        if !safe && node.selected() && (node.updated() || node.deleted()) {
            tracing::warn!("Unsafe file name {relative_path:?}; not synchronized");
            return;
        }
        if node.selected() && node.updated() {
            if let Some(task) = DownloadTask::from_node(relative_path, node) {
                self.total_download_size = self
                    .total_download_size
                    .saturating_add(task.download_size());
                self.download_tasks.push(task);
            } else {
                tracing::warn!("No destination for {relative_path}; not downloaded");
            }
        } else if node.selected() && node.deleted() {
            if let Some(local_path) = node.local_path() {
                self.delete_tasks.push(DeleteTask {
                    relative_path: relative_path.to_owned(),
                    local_path,
                    is_directory: !node.is_leaf(),
                });
            } else {
                tracing::warn!("No destination for {relative_path}; not deleted");
            }
        }
    }
}

/// The first segment of the first task below (or being) a root folder named `userconfig` (any
/// case). A root file named `userconfig` doesn't count.
// Mirrors service/synchronization/FilesSynchronizationManager.java:138-169
fn userconfig_dir(tasks: &[DownloadTask]) -> Option<String> {
    tasks.iter().find_map(|task| {
        let (first, rest) = match task.relative_path.split_once('/') {
            Some((first, rest)) => (first, Some(rest)),
            None => (task.relative_path.as_str(), None),
        };
        let is_directory = rest.is_some() || matches!(task.kind, DownloadTaskKind::Directory);
        (is_directory && first.to_lowercase() == "userconfig").then(|| first.to_owned())
    })
}

/// Selects every node except the root (the CLI "select all").
// Mirrors console/CommandGeneral.java:430-441
pub fn select_all_except_root(tree: &mut PlanTree) {
    tree.walk_mut(&mut |_, node| node.set_selected(true));
}

/// The tasks still to do (status not `Done`), in order.
// Mirrors service/synchronization/FilesSynchronizationManager.java:171-181
pub fn resumed_tasks(tasks: &[DownloadTask]) -> Vec<&DownloadTask> {
    tasks
        .iter()
        .filter(|t| t.status != DownloadStatus::Done)
        .collect()
}

/// Bytes already downloaded by a previous attempt: the download size of the `Done` files.
// Mirrors service/synchronization/FilesSynchronizationManager.java:183-196
pub fn resumed_size(tasks: &[DownloadTask]) -> u64 {
    tasks
        .iter()
        .filter(|t| is_file(t) && t.status == DownloadStatus::Done)
        .map(DownloadTask::download_size)
        .sum()
}

/// The compressed files downloaded (`Done`): they still need uncompressing.
// Mirrors service/synchronization/FilesSynchronizationManager.java:198-211
pub fn downloaded_compressed(tasks: &[DownloadTask]) -> Vec<&DownloadTask> {
    tasks
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                DownloadTaskKind::File {
                    compressed: true,
                    ..
                }
            ) && t.status == DownloadStatus::Done
        })
        .collect()
}

fn is_file(task: &DownloadTask) -> bool {
    matches!(task.kind, DownloadTaskKind::File { .. })
}

/// Size totals of the download report, over the file tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportTotals {
    /// Uncompressed size of every file.
    pub disk_files_size: u64,

    /// Compressed size of the compressed (not zsync-patched) files.
    pub compressed_files_size: u64,

    /// Uncompressed size of the compressed (not zsync-patched) files.
    pub uncompressed_files_size: u64,

    /// Bytes expected to transfer for the partially complete files.
    pub incomplete_expected_size: u64,

    /// Uncompressed size of the partially complete files.
    pub incomplete_disk_size: u64,

    /// Partially complete files (zsync-patched).
    pub incomplete_files: usize,

    /// Compressed files (downloaded as `.zip`).
    pub compressed_files: usize,
}

impl ReportTotals {
    /// Totals over the file tasks of `tasks`.
    // Mirrors service/synchronization/FilesSynchronizationManager.java:105-134
    pub fn compute(tasks: &[DownloadTask]) -> Self {
        let mut totals = Self::default();
        for task in tasks {
            let DownloadTaskKind::File {
                size,
                compressed,
                compressed_size,
                complete,
                ..
            } = &task.kind
            else {
                continue;
            };
            totals.disk_files_size = totals.disk_files_size.saturating_add(*size);
            if *complete != 0.0 {
                totals.incomplete_files += 1;
                totals.incomplete_expected_size = totals
                    .incomplete_expected_size
                    .saturating_add(task.download_size());
                totals.incomplete_disk_size = totals.incomplete_disk_size.saturating_add(*size);
            } else if *compressed {
                totals.compressed_files += 1;
                totals.compressed_files_size = totals
                    .compressed_files_size
                    .saturating_add(*compressed_size);
                totals.uncompressed_files_size =
                    totals.uncompressed_files_size.saturating_add(*size);
            }
        }
        totals
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::model::PlanLeaf;

    fn file_task(path: &str, size: u64, compressed: bool, complete: f64) -> DownloadTask {
        DownloadTask {
            relative_path: path.to_owned(),
            destination_dir: PathBuf::from("/d"),
            kind: DownloadTaskKind::File {
                size,
                compressed,
                compressed_size: size / 4,
                sha1: None,
                complete,
            },
            status: DownloadStatus::Pending,
        }
    }

    fn dir_task(path: &str) -> DownloadTask {
        DownloadTask {
            relative_path: path.to_owned(),
            destination_dir: PathBuf::from("/d"),
            kind: DownloadTaskKind::Directory,
            status: DownloadStatus::Pending,
        }
    }

    fn with_status(mut task: DownloadTask, status: DownloadStatus) -> DownloadTask {
        task.status = status;
        task
    }

    fn pdir(name: &str, children: Vec<PlanNode>) -> PlanDirectory {
        let mut d = PlanDirectory::new(name);
        for c in children {
            d.add_child(c);
        }
        d
    }

    fn pleaf(name: &str, size: u64) -> PlanLeaf {
        let mut l = PlanLeaf::new(name);
        l.size = size;
        l
    }

    /// Sets destinations like the planner does (root children in `/dest`).
    fn place(dir: &mut PlanDirectory, dest: &Path) {
        for child in &mut dir.children {
            match child {
                PlanNode::Directory(d) => {
                    d.destination_path = Some(dest.to_path_buf());
                    let next = dest.join(&d.name);
                    place(d, &next);
                }
                PlanNode::Leaf(l) => l.destination_path = Some(dest.to_path_buf()),
            }
        }
    }

    #[test]
    fn a7_summary_counts_and_tasks() {
        let mut upd_dir = pdir("@a", vec![]);
        upd_dir.updated = true;
        upd_dir.deleted = true; // updated wins in the counts
        let mut big = pleaf("big.pbo", 1000);
        big.updated = true;
        big.compressed = true;
        big.compressed_size = 300;
        let mut small = pleaf("small.pbo", 10);
        small.updated = true;
        let mut gone = pleaf("gone.pbo", 0);
        gone.deleted = true;
        let mut unselected = pleaf("unselected", 5);
        unselected.updated = true;
        upd_dir.add_child(PlanNode::Leaf(big));
        upd_dir.add_child(PlanNode::Leaf(small));
        upd_dir.add_child(PlanNode::Leaf(gone));
        upd_dir.add_child(PlanNode::Leaf(unselected));
        let mut old = pdir("old", vec![]);
        old.deleted = true;

        let mut tree = PlanTree::new();
        tree.root.add_child(PlanNode::Directory(upd_dir));
        tree.root.add_child(PlanNode::Directory(old));
        place(&mut tree.root, Path::new("/dest"));
        select_all_except_root(&mut tree);
        assert!(!tree.root.selected);
        if let Some(n) = tree.get_mut("@a/unselected") {
            n.set_selected(false);
        }

        let s = PlanSummary::compute(&tree);
        assert_eq!(s.files_updated, 4); // @a, big, small, unselected
        assert_eq!(s.files_deleted, 2); // gone, old
        assert_eq!(s.files_selected, 5);
        let paths: Vec<_> = s
            .download_tasks
            .iter()
            .map(|t| t.relative_path.as_str())
            .collect();
        assert_eq!(paths, ["@a", "@a/big.pbo", "@a/small.pbo"]);
        assert_eq!(s.total_download_size, 310);
        assert_eq!(
            s.delete_tasks,
            [
                DeleteTask {
                    relative_path: "@a/gone.pbo".to_owned(),
                    local_path: PathBuf::from("/dest/@a/gone.pbo"),
                    is_directory: false,
                },
                DeleteTask {
                    relative_path: "old".to_owned(),
                    local_path: PathBuf::from("/dest/old"),
                    is_directory: true,
                },
            ]
        );
        assert_eq!(s.userconfig_dir, None);
        assert!(!s.is_userconfig_updated());

        // The root counts when selected.
        tree.root.selected = true;
        assert_eq!(PlanSummary::compute(&tree).files_selected, 6);
    }

    #[test]
    fn a7_userconfig_dir() {
        let find = |tasks: &[DownloadTask]| userconfig_dir(tasks);
        assert_eq!(
            find(&[
                file_task("@a/x", 1, false, 0.0),
                file_task("userconfig/ace/x.hpp", 1, false, 0.0)
            ]),
            Some("userconfig".to_owned())
        );
        assert_eq!(
            find(&[dir_task("UserConfig")]),
            Some("UserConfig".to_owned())
        );
        assert_eq!(find(&[file_task("a/userconfig/x", 1, false, 0.0)]), None);
        assert_eq!(find(&[file_task("userconfig", 1, false, 0.0)]), None);
        assert_eq!(find(&[]), None);
    }

    #[test]
    fn a7_resume_helpers() {
        let tasks = vec![
            with_status(dir_task("@a"), DownloadStatus::Done),
            with_status(file_task("@a/x", 100, true, 0.0), DownloadStatus::Done),
            with_status(file_task("@a/y", 80, false, 0.0), DownloadStatus::Done),
            with_status(file_task("@a/z", 50, true, 0.0), DownloadStatus::Running),
            file_task("@a/w", 10, true, 0.0),
        ];
        let resumed: Vec<_> = resumed_tasks(&tasks)
            .iter()
            .map(|t| t.relative_path.as_str())
            .collect();
        assert_eq!(resumed, ["@a/z", "@a/w"]);
        assert_eq!(resumed_size(&tasks), 25 + 80);
        let compressed: Vec<_> = downloaded_compressed(&tasks)
            .iter()
            .map(|t| t.relative_path.as_str())
            .collect();
        assert_eq!(compressed, ["@a/x"]);
    }

    #[test]
    fn a7_report_totals() {
        let tasks = vec![
            dir_task("@a"),
            file_task("@a/full", 1000, true, 0.0), // compressed, 250
            file_task("@a/part", 800, true, 37.5), // incomplete: 500 expected
            file_task("@a/plain", 64, false, 0.0), // neither
            file_task("@a/part2", 100, false, 50.0), // incomplete: 50 expected
        ];
        assert_eq!(
            ReportTotals::compute(&tasks),
            ReportTotals {
                disk_files_size: 1964,
                compressed_files_size: 250,
                uncompressed_files_size: 1000,
                incomplete_expected_size: 550,
                incomplete_disk_size: 900,
                incomplete_files: 2,
                compressed_files: 1,
            }
        );
    }
}
