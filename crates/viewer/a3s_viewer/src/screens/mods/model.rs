//! What the Mods screen shows, computed from the state when it changes: the addon groups of
//! the active profile with the status of each addon, and the addons found on this computer by
//! folder.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use a3s_addons::{GroupStatus, ScanOptions, refresh_group_status};
use a3s_foundation::format;
use a3s_types::constants::{ModsetType, RepositoryStatus};
use a3s_types::java_string::compare_ignore_case;
use a3s_types::local::{Profile, TreeDirectory, TreeNode};
use a3s_types::{SyncTreeDirectory, SyncTreeNode};
use a3s_ui::icon::icons;
use a3s_ui::theme::ProgressTone;
use a3s_ui::widgets::{CheckState, CheckTreeModel, default_tree_icon};
use a3s_viewer_context::AppState;
use a3s_viewer_context::profiles::AddonScan;
use a3s_viewer_context::sync::{ChangeKind, PlanState};

use super::edit;

/// A row of the Mods tree.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ModNode {
    /// A group or an addon of the profile's addon groups, by child indices from the root.
    Group(Vec<usize>),

    /// The addons found on this computer.
    Computer,

    /// The addons of one search folder (index into the profile's folders), or of the Arma 3
    /// folder (`None`).
    Folder(Option<usize>),

    /// An addon found on this computer, by lowercase registry key.
    Local(String),
}

/// Where an addon of a group stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModStatus {
    /// On this computer and up to date.
    Ready,

    /// Its repository has a newer version of it; the download size when the last comparison of
    /// the repository with its download folder is current.
    Update(Option<u64>),

    /// Its repository has it but it was never downloaded; its size when the repository
    /// metadata tells.
    NotInstalled(Option<u64>),

    /// Neither on this computer nor in a repository.
    Missing,

    /// Enabled in two groups at once; the game would load it twice.
    SelectedTwice,
}

impl ModStatus {
    fn pill(self) -> (String, ProgressTone) {
        match self {
            Self::Ready => ("Ready".to_owned(), ProgressTone::Success),
            Self::Update(Some(size)) => (
                format!("Update · {}", format::size(size as i64)),
                ProgressTone::Warning,
            ),
            Self::Update(None) => ("Update".to_owned(), ProgressTone::Warning),
            Self::NotInstalled(Some(size)) => (
                format!("Not installed · {}", format::size(size as i64)),
                ProgressTone::Info,
            ),
            Self::NotInstalled(None) => ("Not installed".to_owned(), ProgressTone::Info),
            Self::Missing => ("Missing".to_owned(), ProgressTone::Danger),
            Self::SelectedTwice => ("Enabled twice".to_owned(), ProgressTone::Warning),
        }
    }
}

/// What a repository contributes to the statuses.
#[derive(Debug, Default)]
struct RepositoryInfo {
    /// The last check found a newer revision.
    updated: bool,

    /// Addons new or changed above the local revision, lowercase.
    changed: BTreeSet<String>,

    /// The size of each addon of the repository, by lowercase folder name.
    sizes: HashMap<String, u64>,

    /// What downloading each new or changed addon costs, by lowercase folder name, from the
    /// current comparison with the download folder; empty without one.
    update_sizes: HashMap<String, u64>,
}

/// Everything the model is computed from, compared every frame to know when to recompute.
#[derive(Debug, PartialEq)]
struct Inputs {
    tree: TreeDirectory,
    folders: Vec<String>,
    scan: Option<usize>,
    repositories: Vec<RepositoryInputs>,
}

#[derive(Debug, PartialEq)]
struct RepositoryInputs {
    name: String,
    status: RepositoryStatus,
    revision: i32,
    default_download_location: Option<String>,
    events_download_location: BTreeMap<String, String>,
    changelogs: Option<usize>,
    sync: Option<usize>,
    server_revision: Option<i32>,
    plan: Option<PlanState>,
}

fn arc_address<T>(arc: Option<&Arc<T>>) -> Option<usize> {
    arc.map(|arc| Arc::as_ptr(arc) as usize)
}

impl Inputs {
    fn take(state: &AppState, profile: &Profile) -> Self {
        Self {
            tree: profile.tree.clone(),
            folders: profile.addon_search_directories.clone(),
            scan: arc_address(state.profiles.scan.as_ref()),
            repositories: state
                .client
                .repositories()
                .map(|repository| {
                    let runtime = state.repositories.runtime(&repository.name);
                    RepositoryInputs {
                        name: repository.name.clone(),
                        status: runtime.status,
                        revision: repository.revision,
                        default_download_location: repository.default_download_location.clone(),
                        events_download_location: repository
                            .events_download_location
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        changelogs: arc_address(runtime.metadata.changelogs.as_ref()),
                        sync: arc_address(runtime.metadata.sync.as_ref()),
                        server_revision: runtime.metadata.server_revision(),
                        plan: state.sync.downloads.plans.get(&repository.name).cloned(),
                    }
                })
                .collect(),
        }
    }
}

/// The Mods screen's view of the state.
#[derive(Debug)]
pub(crate) struct ModsModel {
    inputs: Inputs,

    /// The addon scan the model was computed from.
    pub(crate) scan: Option<Arc<AddonScan>>,

    /// The groups with missing flags and duplicate keys resolved.
    resolved: TreeDirectory,
    status: GroupStatus,

    /// The local addons by folder, in name order; only folders with addons.
    folders: Vec<(Option<usize>, Vec<String>)>,
    repositories: BTreeMap<String, RepositoryInfo>,
}

impl ModsModel {
    /// The model of the active profile, or `None` without one.
    pub(crate) fn take(state: &AppState) -> Option<Self> {
        let profile = state.client.active_profile()?;
        Some(Self::compute(state, Inputs::take(state, profile)))
    }

    /// Recomputes when the state changed since the model was made.
    pub(crate) fn refresh(&mut self, state: &AppState) -> bool {
        let Some(profile) = state.client.active_profile() else {
            return false;
        };
        let inputs = Inputs::take(state, profile);
        if inputs == self.inputs {
            return false;
        }
        *self = Self::compute(state, inputs);
        true
    }

    fn compute(state: &AppState, inputs: Inputs) -> Self {
        let scan = state.profiles.scan.clone();
        let mut resolved = inputs.tree.clone();
        let status = match &scan {
            Some(scan) => refresh_group_status(
                &mut resolved,
                &scan.registry,
                |name| state.client.repository(name).ok(),
                &scan.tree,
                &ScanOptions::default(),
            ),
            None => GroupStatus::default(),
        };
        let folders = scan
            .as_deref()
            .map(|scan| local_by_folder(scan, &inputs.folders))
            .unwrap_or_default();
        let repositories = state
            .client
            .repositories()
            .map(|repository| {
                let runtime = state.repositories.runtime(&repository.name);
                let changed = runtime
                    .metadata
                    .changelogs
                    .as_ref()
                    .map(|changelogs| {
                        changelogs
                            .list
                            .iter()
                            .filter(|changelog| changelog.revision > repository.revision)
                            .flat_map(|changelog| {
                                changelog.new_addons.iter().chain(&changelog.updated_addons)
                            })
                            .map(|name| name.to_lowercase())
                            .collect()
                    })
                    .unwrap_or_default();
                let mut sizes = HashMap::new();
                if let Some(sync) = &runtime.metadata.sync {
                    addon_sizes(sync, &mut sizes);
                }
                let info = RepositoryInfo {
                    updated: runtime.status == RepositoryStatus::Updated,
                    changed,
                    sizes,
                    update_sizes: update_sizes(state, &repository.name),
                };
                (repository.name.clone(), info)
            })
            .collect();
        Self {
            inputs,
            scan,
            resolved,
            status,
            folders,
            repositories,
        }
    }

    /// The profile's addon groups as stored.
    pub(crate) fn tree(&self) -> &TreeDirectory {
        &self.inputs.tree
    }

    /// The profile's addon search folders.
    pub(crate) fn folders(&self) -> &[String] {
        &self.inputs.folders
    }

    /// The local addons of `folder`.
    fn local_addons(&self, folder: Option<usize>) -> &[String] {
        self.folders
            .iter()
            .find(|(f, _)| *f == folder)
            .map_or(&[], |(_, keys)| keys.as_slice())
    }

    /// Every local addon key, in folder order.
    pub(crate) fn all_local_addons(&self) -> impl Iterator<Item = &String> {
        self.folders.iter().flat_map(|(_, keys)| keys)
    }

    /// The status of the group addon at `path`; `None` for a directory or before the first
    /// scan.
    pub(crate) fn status(&self, path: &[usize]) -> Option<ModStatus> {
        self.scan.as_ref()?;
        let TreeNode::Leaf(leaf) = edit::node(&self.resolved, path)? else {
            return None;
        };
        let group = match self.resolved.children.get(*path.first()?)? {
            TreeNode::Directory(group) => Some(group),
            TreeNode::Leaf(_) => None,
        };
        let repository = group
            .filter(|group| group.modset_type.is_some())
            .and_then(|group| group.modset_repository_name.as_deref())
            .and_then(|name| self.repositories.get(name));
        let duplicated = self
            .status
            .leaf(path)
            .is_some_and(|status| status.duplicated_selection);
        let name = leaf.name.to_lowercase();
        Some(if duplicated && leaf.selected {
            ModStatus::SelectedTwice
        } else if leaf.missing {
            match repository {
                Some(repository) => ModStatus::NotInstalled(repository.sizes.get(&name).copied()),
                None => ModStatus::Missing,
            }
        } else if let Some(repository) = repository.filter(|r| {
            (r.updated && r.changed.contains(&name)) || r.update_sizes.contains_key(&name)
        }) {
            ModStatus::Update(repository.update_sizes.get(&name).copied())
        } else {
            ModStatus::Ready
        })
    }

    /// The label of `node`.
    pub(crate) fn label(&self, node: &ModNode) -> String {
        match node {
            ModNode::Group(path) => match edit::node(self.tree(), path) {
                // Mirrors dto/TreeLeafDTO.java:66-95
                Some(TreeNode::Leaf(leaf)) if leaf.optional => format!("{} (optional)", leaf.name),
                Some(node) => node.name().to_owned(),
                None => String::new(),
            },
            ModNode::Computer => "On this computer".to_owned(),
            ModNode::Folder(Some(index)) => self
                .folders()
                .get(*index)
                .map(|folder| folder_name(folder))
                .unwrap_or_default(),
            ModNode::Folder(None) => "Arma 3".to_owned(),
            ModNode::Local(key) => self
                .scan
                .as_ref()
                .and_then(|scan| scan.registry.get(key))
                .map_or_else(|| key.clone(), |addon| addon.name.clone()),
        }
    }

    fn children(&self, node: &ModNode) -> Vec<ModNode> {
        match node {
            ModNode::Group(path) => match edit::node(self.tree(), path) {
                Some(TreeNode::Directory(directory)) => (0..directory.children.len())
                    .map(|index| {
                        let mut child = path.clone();
                        child.push(index);
                        ModNode::Group(child)
                    })
                    .collect(),
                _ => Vec::new(),
            },
            ModNode::Computer => self
                .folders
                .iter()
                .map(|(folder, _)| ModNode::Folder(*folder))
                .collect(),
            ModNode::Folder(folder) => self
                .local_addons(*folder)
                .iter()
                .map(|key| ModNode::Local(key.clone()))
                .collect(),
            ModNode::Local(_) => Vec::new(),
        }
    }

    fn roots(&self) -> Vec<ModNode> {
        let mut roots: Vec<ModNode> = self
            .tree()
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| !child.is_leaf())
            .map(|(index, _)| ModNode::Group(vec![index]))
            .collect();
        if !self.folders.is_empty() {
            roots.push(ModNode::Computer);
        }
        roots
    }

    fn is_leaf(&self, node: &ModNode) -> bool {
        match node {
            ModNode::Group(path) => edit::node(self.tree(), path).is_none_or(TreeNode::is_leaf),
            ModNode::Local(_) => true,
            ModNode::Computer | ModNode::Folder(_) => false,
        }
    }

    /// Whether `node` is enabled (for a directory: all its addons are).
    pub(crate) fn is_checked(&self, node: &ModNode) -> bool {
        match node {
            ModNode::Group(path) => match edit::node(self.tree(), path) {
                Some(TreeNode::Leaf(leaf)) => leaf.selected,
                Some(TreeNode::Directory(directory)) => edit::all_leaves_selected(directory),
                None => false,
            },
            ModNode::Local(key) => edit::loose_enabled(self.tree(), key),
            ModNode::Computer | ModNode::Folder(_) => {
                let keys = self.local_keys(node);
                !keys.is_empty() && keys.iter().all(|key| edit::loose_enabled(self.tree(), key))
            }
        }
    }

    /// The local addon keys below `node`.
    pub(crate) fn local_keys(&self, node: &ModNode) -> Vec<String> {
        match node {
            ModNode::Computer => self.all_local_addons().cloned().collect(),
            ModNode::Folder(folder) => self.local_addons(*folder).to_vec(),
            ModNode::Local(key) => vec![key.clone()],
            ModNode::Group(_) => Vec::new(),
        }
    }

    fn check_state(&self, node: &ModNode) -> CheckState {
        let children = self.children(node);
        let leaf = self.is_leaf(node);
        if leaf {
            return CheckState::from_selected(self.is_checked(node));
        }
        let checked: Vec<bool> = children
            .iter()
            .map(|child| {
                self.is_checked(child)
                    || matches!(self.check_state(child), CheckState::Indeterminate)
            })
            .collect();
        let all = self.is_checked(node);
        if !all && checked.iter().any(|c| *c) {
            CheckState::Indeterminate
        } else {
            CheckState::from_selected(all)
        }
    }

    /// The top-level group of `node`, if it is in the profile's groups.
    pub(crate) fn top_group<'m>(&'m self, node: &ModNode) -> Option<&'m TreeDirectory> {
        let ModNode::Group(path) = node else {
            return None;
        };
        match self.tree().children.get(*path.first()?)? {
            TreeNode::Directory(group) => Some(group),
            TreeNode::Leaf(_) => None,
        }
    }

    /// The names of the user groups (not made from a repository or event).
    pub(crate) fn user_groups(&self) -> Vec<String> {
        self.tree()
            .directories()
            .filter(|group| group.modset_type.is_none())
            .map(|group| group.name.clone())
            .collect()
    }
}

/// The last component of `folder`, or all of it when it has none.
pub(crate) fn folder_name(folder: &str) -> String {
    Path::new(folder).file_name().map_or_else(
        || folder.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The local addons grouped by the search folder they were found in; addons outside every
/// search folder (the CDLCs) go under the Arma 3 folder. Each list is sorted by name.
fn local_by_folder(scan: &AddonScan, folders: &[String]) -> Vec<(Option<usize>, Vec<String>)> {
    let roots: Vec<_> = folders
        .iter()
        .map(|folder| std::path::absolute(folder).unwrap_or_else(|_| folder.into()))
        .collect();
    let mut by_folder: BTreeMap<Option<usize>, Vec<(String, String)>> = BTreeMap::new();
    for addon in scan.registry.addons() {
        let path = Path::new(&addon.path);
        let folder = roots.iter().position(|root| path.starts_with(root));
        by_folder
            .entry(folder)
            .or_default()
            .push((addon.name.clone(), addon.key.to_lowercase()));
    }
    let mut out: Vec<(Option<usize>, Vec<String>)> = by_folder
        .into_iter()
        .map(|(folder, mut addons)| {
            addons.sort_by(|a, b| compare_ignore_case(&a.0, &b.0));
            (folder, addons.into_iter().map(|(_, key)| key).collect())
        })
        .collect();
    // Search folders first, in profile order; the Arma 3 folder last.
    out.sort_by_key(|(folder, _)| folder.map_or(usize::MAX, |index| index));
    out
}

/// The download size of each new or changed top-level addon of `repository`, by lowercase
/// name, from its comparison with the download folder; empty when there is none or the server
/// has a newer revision since.
fn update_sizes(state: &AppState, repository: &str) -> HashMap<String, u64> {
    let current = state
        .repositories
        .runtime(repository)
        .metadata
        .server_revision();
    let Some(PlanState {
        server_revision,
        result: Ok(plan),
    }) = state.sync.downloads.plans.get(repository)
    else {
        return HashMap::new();
    };
    if *server_revision != current {
        return HashMap::new();
    }
    plan.changes
        .iter()
        .filter(|change| change.kind != ChangeKind::Removed)
        .map(|change| (change.name.to_lowercase(), change.size))
        .collect()
}

/// The size of every directory marked as an addon, by lowercase name.
fn addon_sizes(directory: &SyncTreeDirectory, out: &mut HashMap<String, u64>) {
    for child in &directory.children {
        if let SyncTreeNode::Directory(child) = child {
            if child.mark_as_addon {
                out.insert(child.name.to_lowercase(), files_size(child));
            } else {
                addon_sizes(child, out);
            }
        }
    }
}

fn files_size(directory: &SyncTreeDirectory) -> u64 {
    directory
        .children
        .iter()
        .map(|child| match child {
            SyncTreeNode::Leaf(leaf) => u64::try_from(leaf.size).unwrap_or(0),
            SyncTreeNode::Directory(d) => files_size(d),
        })
        .sum()
}

/// The [`CheckTreeModel`] of the Mods tree: the model, narrowed by the search text and, for
/// the Enabled segment, to the enabled addons.
pub(crate) struct TreeView<'m> {
    model: &'m ModsModel,

    /// The visible rows; `None` shows everything.
    visible: Option<HashSet<ModNode>>,
}

impl<'m> TreeView<'m> {
    pub(crate) fn new(model: &'m ModsModel, search: &str, enabled_only: bool) -> Self {
        let search = search.trim().to_lowercase();
        if search.is_empty() && !enabled_only {
            return Self {
                model,
                visible: None,
            };
        }
        let mut visible = HashSet::new();
        for root in model.roots() {
            mark_visible(model, &root, &search, enabled_only, false, &mut visible);
        }
        Self {
            model,
            visible: Some(visible),
        }
    }

    fn shows(&self, node: &ModNode) -> bool {
        self.visible.as_ref().is_none_or(|v| v.contains(node))
    }
}

/// Adds `node` to `visible` when it, an ancestor (`parent_matches`) or a descendant matches;
/// returns whether it was added.
fn mark_visible(
    model: &ModsModel,
    node: &ModNode,
    search: &str,
    enabled_only: bool,
    parent_matches: bool,
    visible: &mut HashSet<ModNode>,
) -> bool {
    let matches = parent_matches || model.label(node).to_lowercase().contains(search);
    let shown = if model.is_leaf(node) {
        matches && (!enabled_only || model.is_checked(node))
    } else {
        let mut any = false;
        for child in model.children(node) {
            any |= mark_visible(model, &child, search, enabled_only, matches, visible);
        }
        any
    };
    if shown {
        visible.insert(node.clone());
    }
    shown
}

impl CheckTreeModel for TreeView<'_> {
    type Key = ModNode;

    fn roots(&self) -> Vec<ModNode> {
        self.model
            .roots()
            .into_iter()
            .filter(|node| self.shows(node))
            .collect()
    }

    fn children(&self, key: &ModNode) -> Vec<ModNode> {
        self.model
            .children(key)
            .into_iter()
            .filter(|node| self.shows(node))
            .collect()
    }

    fn is_leaf(&self, key: &ModNode) -> bool {
        self.model.is_leaf(key)
    }

    fn label(&self, key: &ModNode) -> String {
        self.model.label(key)
    }

    fn check_state(&self, key: &ModNode) -> CheckState {
        self.model.check_state(key)
    }

    fn icon(&self, key: &ModNode, expanded: bool) -> Option<&'static str> {
        Some(match key {
            ModNode::Group(path) if path.len() == 1 => match self.model.top_group(key) {
                Some(group) => match group.modset_type {
                    Some(ModsetType::Repository) => icons::CLOUD_ARROW_DOWN,
                    Some(ModsetType::Event) => icons::CALENDAR_DOTS,
                    None => default_tree_icon(false, expanded),
                },
                None => icons::PACKAGE,
            },
            ModNode::Computer => icons::DESKTOP,
            ModNode::Folder(Some(_)) => icons::HARD_DRIVE,
            ModNode::Folder(None) => icons::GAME_CONTROLLER,
            _ => default_tree_icon(self.model.is_leaf(key), expanded),
        })
    }

    fn pill(&self, key: &ModNode) -> Option<(String, ProgressTone)> {
        match key {
            ModNode::Group(path) => self.model.status(path).map(ModStatus::pill),
            _ => None,
        }
    }

    /// Where a mod from this computer is.
    fn tooltip(&self, key: &ModNode) -> Option<String> {
        let ModNode::Local(key) = key else {
            return None;
        };
        let addon = self.model.scan.as_ref()?.registry.get(key)?;
        Some(
            Path::new(&addon.path)
                .join(&addon.name)
                .display()
                .to_string(),
        )
    }

    /// A mod from this computer can be dragged into a user group.
    fn draggable(&self, key: &ModNode) -> bool {
        matches!(key, ModNode::Local(_))
    }

    fn accepts_drop(&self, target: &ModNode, dragged: &ModNode) -> bool {
        matches!(dragged, ModNode::Local(_))
            && !self.model.is_leaf(target)
            && self
                .model
                .top_group(target)
                .is_some_and(|group| group.modset_type.is_none())
    }
}
