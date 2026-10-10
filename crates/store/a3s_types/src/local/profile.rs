use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::LauncherOptions;
use crate::constants::ModsetType;
use crate::java_string::compare_ignore_case;

/// Name of the root of every profile's addon-group tree (French for "root").
pub const PROFILE_TREE_ROOT_NAME: &str = "racine";

/// A launch profile (`fr.soe.a3s.domain.Profile`, `*.a3s.profile`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,

    /// Addon groups; the root is named [`PROFILE_TREE_ROOT_NAME`].
    #[serde(default = "TreeDirectory::new_root")]
    pub tree: TreeDirectory,

    /// Extra command-line parameters.
    #[serde(default)]
    pub additional_parameters: Option<String>,

    #[serde(default)]
    pub addon_names_by_priority: Vec<String>,

    #[serde(default)]
    pub launcher_options: LauncherOptions,

    #[serde(default)]
    pub addon_search_directories: Vec<String>,

    #[serde(default)]
    pub last_added_addon_search_directory: Option<String>,
}

impl Profile {
    /// A profile with the Java field initializer values.
    // Mirrors domain/Profile.java:15-25
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tree: TreeDirectory::new_root(),
            additional_parameters: None,
            addon_names_by_priority: Vec::new(),
            launcher_options: LauncherOptions::default(),
            addon_search_directories: Vec::new(),
            last_added_addon_search_directory: None,
        }
    }
}

/// An addon group (`fr.soe.a3s.domain.TreeDirectory`).
///
/// The Java `parent` back-reference is implied by nesting and recreated on encoding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TreeDirectory {
    pub name: String,
    pub selected: bool,
    pub marked: bool,
    pub optional: bool,

    /// Set when the group was created from a repository or event.
    pub modset_type: Option<ModsetType>,
    pub modset_repository_name: Option<String>,

    /// Children (`list`): directories first, then leaves, each sorted case-insensitively when
    /// added with [`Self::add_child`].
    pub children: Vec<TreeNode>,

    /// Runtime-only (`transient` in Java).
    #[serde(skip)]
    pub updated: bool,
}

/// An addon in a group (`fr.soe.a3s.domain.TreeLeaf`).
///
/// The deprecated Java field `duplicate` is always `false` and not exposed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TreeLeaf {
    pub name: String,
    pub selected: bool,
    pub optional: bool,
    pub workshop_id: Option<String>,
    pub dlc_app_id: Option<String>,

    /// Runtime-only (`transient` in Java).
    #[serde(skip)]
    pub missing: bool,
}

/// A node of a profile's addon-group tree (`fr.soe.a3s.domain.TreeNode`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TreeNode {
    Directory(TreeDirectory),
    Leaf(TreeLeaf),
}

impl TreeNode {
    pub fn name(&self) -> &str {
        match self {
            Self::Directory(d) => &d.name,
            Self::Leaf(l) => &l.name,
        }
    }

    pub fn is_leaf(&self) -> bool {
        matches!(self, Self::Leaf(_))
    }

    pub fn selected(&self) -> bool {
        match self {
            Self::Directory(d) => d.selected,
            Self::Leaf(l) => l.selected,
        }
    }

    pub fn set_selected(&mut self, value: bool) {
        match self {
            Self::Directory(d) => d.selected = value,
            Self::Leaf(l) => l.selected = value,
        }
    }

    pub fn optional(&self) -> bool {
        match self {
            Self::Directory(d) => d.optional,
            Self::Leaf(l) => l.optional,
        }
    }

    pub fn set_optional(&mut self, value: bool) {
        match self {
            Self::Directory(d) => d.optional = value,
            Self::Leaf(l) => l.optional = value,
        }
    }

    /// Java `compareTo`: `compareToIgnoreCase` of the names.
    // Mirrors domain/TreeDirectory.java:75-80, domain/TreeLeaf.java:49-54
    pub fn java_cmp(&self, other: &Self) -> Ordering {
        compare_ignore_case(self.name(), other.name())
    }
}

impl TreeDirectory {
    /// An empty group tree root ([`PROFILE_TREE_ROOT_NAME`]).
    pub fn new_root() -> Self {
        Self::new(PROFILE_TREE_ROOT_NAME)
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Appends `node`, then reorders the children as all directories followed by all leaves,
    /// each group stably sorted by `compareToIgnoreCase` of the name (`addTreeNode`).
    // Mirrors domain/TreeDirectory.java:31-49
    pub fn add_child(&mut self, node: TreeNode) {
        self.children.push(node);
        let (mut directories, mut leaves): (Vec<_>, Vec<_>) = std::mem::take(&mut self.children)
            .into_iter()
            .partition(|n| !n.is_leaf());
        directories.sort_by(TreeNode::java_cmp);
        leaves.sort_by(TreeNode::java_cmp);
        self.children = directories;
        self.children.append(&mut leaves);
    }

    /// Removes and returns the child at `index` (`removeTreeNode`, which removes by identity in
    /// Java). `None` if out of range.
    // Mirrors domain/TreeDirectory.java:100-102
    pub fn remove_child(&mut self, index: usize) -> Option<TreeNode> {
        (index < self.children.len()).then(|| self.children.remove(index))
    }

    pub fn directories(&self) -> impl Iterator<Item = &Self> {
        self.children.iter().filter_map(|c| match c {
            TreeNode::Directory(d) => Some(d),
            TreeNode::Leaf(_) => None,
        })
    }

    pub fn leaves(&self) -> impl Iterator<Item = &TreeLeaf> {
        self.children.iter().filter_map(|c| match c {
            TreeNode::Leaf(l) => Some(l),
            TreeNode::Directory(_) => None,
        })
    }
}

impl TreeLeaf {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
}
