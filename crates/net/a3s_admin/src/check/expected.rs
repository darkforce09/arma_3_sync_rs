//! The remote files a set of sync tree nodes stands for.

use a3s_repo_files::layout::{ZIP_EXTENSION, ZSYNC_EXTENSION};
use a3s_sync::RemoteFile;
use a3s_types::Protocol;

use crate::upload::PlannedNode;

/// Whether a repository's files have `.zsync` companions: its **download** protocol is HTTP(S).
// Mirrors service/ConnectionService.java:371, 421, 441 (`protocol instanceof Http`)
pub fn has_zsync(download_protocol: &Protocol) -> bool {
    download_protocol.protocol_type.is_http()
}

/// Expands nodes into remote files, in node order.
///
/// A directory is itself; a compressed leaf is only `name.zip` when
/// `compressed_pbo_files_only`, else `name`, `name.zip` and (with `with_zsync`) `name.zsync`;
/// any other leaf is `name` and (with `with_zsync`) `name.zsync`. Remote paths are the nodes'
/// remote paths ([`PlannedNode::relative_path`]).
// Mirrors service/connection/AbstractConnectionProcessor.java:33-79
pub fn expected_remote_files<'a>(
    nodes: impl IntoIterator<Item = &'a PlannedNode>,
    compressed_pbo_files_only: bool,
    with_zsync: bool,
) -> Vec<RemoteFile> {
    expected_files(nodes, compressed_pbo_files_only, with_zsync)
        .into_iter()
        .map(|(remote, _)| remote)
        .collect()
}

/// [`expected_remote_files`] with each file's path in the tree (its local path below the
/// repository folder).
pub(crate) fn expected_files<'a>(
    nodes: impl IntoIterator<Item = &'a PlannedNode>,
    compressed_pbo_files_only: bool,
    with_zsync: bool,
) -> Vec<(RemoteFile, String)> {
    let mut files = Vec::new();
    for node in nodes {
        let (parent, name) = node.parent_and_name();
        if node.is_directory {
            files.push((RemoteFile::new(name, parent, true), node.tree_path.clone()));
            continue;
        }
        let tree_parent = node.tree_parent();
        let file = |suffix: &str| {
            let file_name = format!("{name}{suffix}");
            let tree_path = if tree_parent.is_empty() {
                file_name.clone()
            } else {
                format!("{tree_parent}/{file_name}")
            };
            (RemoteFile::new(file_name, parent, false), tree_path)
        };
        if node.compressed && compressed_pbo_files_only {
            files.push(file(ZIP_EXTENSION));
            continue;
        }
        files.push(file(""));
        if node.compressed {
            files.push(file(ZIP_EXTENSION));
        }
        if with_zsync {
            files.push(file(ZSYNC_EXTENSION));
        }
    }
    files
}
