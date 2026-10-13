//! The addon scan of the active profile: its search folders and the Arma 3 folder, walked on a
//! blocking thread, with the size of every addon found.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use a3s_addons::{AddonRegistry, ScanOptions, priority};
use a3s_repo_files::services::ClientData;
use a3s_sync::EventError;
use a3s_types::constants::GameDlc;
use a3s_types::local::{Profile, TreeDirectory};

use crate::ViewerContext;
use crate::command::Command;
use crate::error::ContextError;
use crate::event::{AppEvent, Change};
use crate::tasks::{Cx, TaskCtx, TaskError, TaskKey, TaskKind};

/// What a scan depends on: the profile, its search folders and the Arma 3 executable (whose
/// folder holds the CDLCs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanKey {
    /// The profile name.
    pub profile: String,

    /// The addon search folders, in profile order.
    pub folders: Vec<String>,

    /// The Arma 3 executable.
    pub arma3_exe_path: Option<String>,
}

impl ScanKey {
    /// The key of `profile`.
    pub fn of(profile: &Profile) -> Self {
        Self {
            profile: profile.name.clone(),
            folders: profile.addon_search_directories.clone(),
            arma3_exe_path: profile.launcher_options.arma3_exe_path.clone(),
        }
    }

    /// The key of the active profile, if there is one.
    pub fn of_active(client: &ClientData) -> Option<Self> {
        client.active_profile().map(Self::of)
    }
}

/// The addons found for a profile.
#[derive(Clone, Debug)]
pub struct AddonScan {
    /// What was scanned.
    pub key: ScanKey,

    /// Every mod, then every CDLC, under its symbolic key.
    pub registry: AddonRegistry,

    /// The available-addons tree.
    pub tree: TreeDirectory,

    /// The bytes on disk of each addon folder, by lowercase registry key.
    pub sizes: HashMap<String, u64>,

    /// The CDLCs present in the Arma 3 folder.
    pub dlcs: Vec<GameDlc>,
}

impl AddonScan {
    /// The bytes on disk of the addon with registry key `key` (any case); 0 when unknown.
    pub fn size(&self, key: &str) -> u64 {
        self.sizes.get(&key.to_lowercase()).copied().unwrap_or(0)
    }
}

/// Asks for a scan when the last one is stale; returns whether it asked.
///
/// The last scan is stale when it does not match the active profile any more (first start,
/// another profile, changed folders or Arma 3 executable) and no scan runs or was asked for.
/// Cheap; the UI calls it every frame.
pub fn ensure_scan(viewer: &ViewerContext) -> Result<bool, ContextError> {
    let key = {
        let state = viewer.read();
        let Some(key) = ScanKey::of_active(&state.client) else {
            return Ok(false);
        };
        let fresh = state.profiles.scan.as_ref().is_some_and(|s| s.key == key);
        if fresh || state.profiles.scan_requested.is_some() {
            return Ok(false);
        }
        key
    };
    if viewer
        .is_running(&TaskKey::new(TaskKind::AddonScan))
        .is_some()
    {
        return Ok(false);
    }
    viewer.cx().state().write().profiles.scan_requested = Some(key);
    viewer.send(Command::Profiles(super::Command::ScanAddons))?;
    Ok(true)
}

/// Starts the scan task; a running scan is left alone.
pub(super) fn start(cx: &Cx) -> Result<(), ContextError> {
    match cx.spawn_task(TaskKey::new(TaskKind::AddonScan), true, run) {
        Ok(_) | Err(ContextError::TaskBusy { .. }) => Ok(()),
        Err(err) => {
            cx.state().write().profiles.scan_requested = None;
            Err(err)
        }
    }
}

async fn run(task: TaskCtx) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let result = scan_active(&task).await;
    cx.state().write().profiles.scan_requested = None;
    cx.emit(AppEvent::Changed(Change::AddonFiles));
    result
}

async fn scan_active(task: &TaskCtx) -> Result<(), TaskError> {
    let cx = &task.cx;
    let Some(profile) = cx.state().read().client.active_profile().cloned() else {
        cx.state().write().profiles.scan = None;
        return Ok(());
    };
    let scan = tokio::task::spawn_blocking(move || scan_profile(&profile))
        .await
        .map_err(|err| {
            TaskError::Message(EventError::new(
                a3s_foundation::error::ErrorKind::Internal,
                format!("The addon scan stopped: {err}"),
            ))
        })?;
    if task.cancel.is_cancelled() {
        return Err(TaskError::Cancelled);
    }
    let mut state = cx.state().write();
    // The launch order lists every addon found, as the Java client keeps it after a scan.
    // Mirrors ui/main/AddonsPanel.java:731-738 and service/AddonService.java:374-417
    let registry = &scan.registry;
    let profile_name = scan.key.profile.clone();
    state.client.update_active_profile(|profile| {
        profile.name == profile_name
            && priority::sync_addon_priorities(&mut profile.addon_names_by_priority, registry)
    })?;
    state.profiles.scan = Some(Arc::new(scan));
    Ok(())
}

/// Scans `profile` and measures every addon found. Blocking.
fn scan_profile(profile: &Profile) -> AddonScan {
    let result = a3s_addons::scan(profile, &ScanOptions::default());
    let sizes = result
        .registry
        .addons()
        .map(|addon| {
            let folder = Path::new(&addon.path).join(&addon.name);
            (addon.key.to_lowercase(), folder_size(&folder))
        })
        .collect();
    AddonScan {
        key: ScanKey::of(profile),
        registry: result.registry,
        tree: result.tree,
        sizes,
        dlcs: result.dlcs,
    }
}

/// The addon scan of `profile`: `last` when it was made for the profile as it is now, else a
/// new scan, which is not stored. Blocking when it scans.
pub(crate) fn reuse_or_scan(last: Option<Arc<AddonScan>>, profile: &Profile) -> Arc<AddonScan> {
    match last {
        Some(scan) if scan.key == ScanKey::of(profile) => scan,
        _ => Arc::new(scan_profile(profile)),
    }
}

/// The total size of the files below `folder`, without following links; unreadable entries
/// count as empty.
fn folder_size(folder: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata().map_or(0, |meta| meta.len());
            }
        }
    }
    total
}
