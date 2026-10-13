//! Server keys: the `.bikey` files of the active profile's enabled mods, copied into one folder.

use std::path::{Path, PathBuf};

use a3s_addons::{ScanOptions, arma3_directory, scan};
use a3s_repo_files::services::ServiceError;
use a3s_tools::CancelToken;
use a3s_tools::bikey::{self, BikeyOutcome};
use a3s_types::local::{Profile, TreeDirectory, TreeNode};

use super::blocking;
use crate::error::ContextError;
use crate::state::AppState;
use crate::tasks::{Cx, TaskError, TaskId, TaskKey, TaskKind};

/// What the server keys tool knows.
#[derive(Clone, Debug, Default)]
pub struct KeysState {
    /// The keys of the enabled mods, from the last [`find_keys`].
    pub scan: Option<KeyScan>,

    /// The result of the last [`copy_keys`].
    pub last_copy: Option<KeyCopy>,
}

/// The keys of a profile's enabled mods.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyScan {
    /// The profile.
    pub profile: String,

    /// Each enabled mod found on disk, in tree order.
    pub mods: Vec<ModKeys>,
}

impl KeyScan {
    /// Every key found, mod by mod.
    pub fn keys(&self) -> Vec<PathBuf> {
        self.mods
            .iter()
            .flat_map(|m| m.keys.iter().cloned())
            .collect()
    }

    /// The names of the mods without a key.
    pub fn unsigned(&self) -> Vec<&str> {
        self.mods
            .iter()
            .filter(|m| m.keys.is_empty())
            .map(|m| m.name.as_str())
            .collect()
    }
}

/// One enabled mod and its keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModKeys {
    /// The mod folder name.
    pub name: String,

    /// The mod folder.
    pub folder: PathBuf,

    /// Its `.bikey` files.
    pub keys: Vec<PathBuf>,
}

/// The result of a copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCopy {
    /// Where the keys went.
    pub target: PathBuf,

    /// What was copied, or why nothing could be.
    pub result: Result<BikeyOutcome, String>,
}

/// The stored target folder of the keys.
pub fn keys_target(state: &AppState) -> Option<PathBuf> {
    state
        .client
        .configuration()
        .bikey_extract_options
        .target_directory_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Stores the target folder of the keys.
pub fn set_keys_target(cx: &Cx, folder: &Path) -> Result<(), ServiceError> {
    cx.state()
        .write()
        .client
        .set_bikey_target_directory(Some(folder.display().to_string()))
}

/// Finds the keys of the active profile's enabled mods into [`KeysState::scan`].
pub fn find_keys(cx: &Cx) -> Result<TaskId, ContextError> {
    let (name, profile) = {
        let state = cx.state().read();
        let name = state.client.active_profile_name().to_owned();
        let profile = state.client.active_profile().cloned();
        (name.clone(), profile.unwrap_or_else(|| Profile::new(name)))
    };
    cx.spawn_task(
        TaskKey::new(TaskKind::ExtractBikeys),
        true,
        move |task| async move {
            let found = blocking(&task, move |cancel| scan_keys(&profile, cancel)).await?;
            let mut scan = found.ok_or(TaskError::Cancelled)?;
            scan.profile = name;
            task.cx.state().write().tools.keys.scan = Some(scan);
            Ok(())
        },
    )
}

/// Copies the keys of the last [`find_keys`] into `target`; the result goes to
/// [`KeysState::last_copy`].
pub fn copy_keys(cx: &Cx, target: PathBuf) -> Result<TaskId, ContextError> {
    let scan = cx
        .state()
        .read()
        .tools
        .keys
        .scan
        .clone()
        .unwrap_or_default();
    cx.spawn_task(
        TaskKey::new(TaskKind::ExtractBikeys),
        true,
        move |task| async move {
            let destination = target.clone();
            let result = blocking(&task, move |cancel| copy(&scan, &destination, cancel)).await?;
            let cancelled = matches!(result, Ok(BikeyOutcome::Cancelled { .. }));
            task.cx.state().write().tools.keys.last_copy = Some(KeyCopy { target, result });
            if cancelled {
                Err(TaskError::Cancelled)
            } else {
                Ok(())
            }
        },
    )
}

fn copy(scan: &KeyScan, target: &Path, cancel: &CancelToken) -> Result<BikeyOutcome, String> {
    let keys = scan.keys();
    // The source check runs on a mod that has keys: the copy reads from it.
    let source = scan
        .mods
        .iter()
        .find(|m| !m.keys.is_empty())
        .map(|m| m.folder.display().to_string())
        .unwrap_or_default();
    if !keys.is_empty() {
        bikey::validate(&source, &target.display().to_string()).map_err(|err| err.to_string())?;
    }
    bikey::copy_bikeys(&keys, target, cancel).map_err(|err| err.to_string())
}

/// The enabled mods of `profile` found on disk and their keys; `None` once cancelled.
fn scan_keys(profile: &Profile, cancel: &CancelToken) -> Option<KeyScan> {
    let found = scan(profile, &ScanOptions::default());
    let arma3_dir = arma3_directory(profile.launcher_options.arma3_exe_path.as_deref())
        .and_then(|dir| std::path::absolute(dir).ok());
    let is_dlc = |name: &str, path: &str| {
        found.dlcs.iter().any(|dlc| dlc.java_name() == name)
            && arma3_dir.as_deref() == Some(Path::new(path))
    };
    let mut seen = Vec::new();
    let mut mods = Vec::new();
    for name in enabled_addon_names(&profile.tree) {
        let lower = name.to_lowercase();
        if seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        let Some(addon) = found.registry.get(&name) else {
            continue;
        };
        if is_dlc(&addon.name, &addon.path) {
            continue;
        }
        let folder = Path::new(&addon.path).join(&addon.name);
        let keys = bikey::find_bikeys(&folder, cancel)?;
        mods.push(ModKeys {
            name: addon.name.clone(),
            folder,
            keys,
        });
    }
    Some(KeyScan {
        profile: profile.name.clone(),
        mods,
    })
}

/// The names of every selected leaf of `tree`, depth first; group flags are ignored, as at
/// launch.
fn enabled_addon_names(tree: &TreeDirectory) -> Vec<String> {
    fn collect(directory: &TreeDirectory, names: &mut Vec<String>) {
        for node in &directory.children {
            match node {
                TreeNode::Leaf(leaf) if leaf.selected => names.push(leaf.name.clone()),
                TreeNode::Leaf(_) => {}
                TreeNode::Directory(child) => collect(child, names),
            }
        }
    }
    let mut names = Vec::new();
    collect(tree, &mut names);
    names
}
