//! Local folder of every node.

use std::path::Path;

use super::{AddonLocator, PlanError, absolute, validate_names};
use crate::model::{PlanDirectory, PlanNode, PlanTree};

/// Sets the `destination_path` of every node below the root (pre-order).
///
/// * Children of the root get `destination` as given.
/// * Other nodes get the absolute path of their parent folder
///   (`parent.destination_path/parent.name`).
/// * With `auto_discover`, a folder marked as addon at any depth whose name `addons` knows is
///   redirected to the folder containing the existing local copy; its content follows.
///
/// The root's destination stays `None`.
///
/// Fails with [`PlanError::UnsafeName`] (setting nothing) if a node name is not a plain path
/// component (see [`validate_names`]).
// Mirrors service/RepositoryService.java:477-511
pub fn assign_destinations(
    tree: &mut PlanTree,
    destination: &Path,
    auto_discover: bool,
    addons: &dyn AddonLocator,
) -> Result<(), PlanError> {
    validate_names(tree)?;
    tree.root.destination_path = None;
    assign_children(&mut tree.root, destination, auto_discover, addons);
    Ok(())
}

/// Assigns `child_destination` to the children of `directory`, then recurses.
fn assign_children(
    directory: &mut PlanDirectory,
    child_destination: &Path,
    auto_discover: bool,
    addons: &dyn AddonLocator,
) {
    for child in &mut directory.children {
        match child {
            PlanNode::Leaf(leaf) => leaf.destination_path = Some(child_destination.to_path_buf()),
            PlanNode::Directory(d) => {
                let mut path = child_destination.to_path_buf();
                // Mirrors service/RepositoryService.java:494-499
                if auto_discover
                    && d.mark_as_addon
                    && let Some(found) = addons.addon_parent_dir(&d.name)
                {
                    path = found;
                }
                let grandchild_destination = absolute(&path.join(&d.name));
                d.destination_path = Some(path);
                assign_children(d, &grandchild_destination, auto_discover, addons);
            }
        }
    }
}
