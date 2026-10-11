//! Planning: compares the remote sync tree with the local disk.
//!
//! Port of `RepositoryService.checkForAddons` and its helpers. [`plan_repository`] runs every
//! step in the Java order; each step is also public for tests and the UI:
//!
//! 1. [`crate::event_filter::filter`] when an event is selected;
//! 2. [`assign_destinations`]: local folder of every node (with addon auto-discovery);
//! 3. [`compute_local_sha1`]: SHA-1 of the local files, through the [`a3s_repo_files::hash::Sha1Cache`];
//! 4. [`mark_updated`]: new or changed nodes;
//! 5. [`mark_hidden`]: hidden folders;
//! 6. [`add_extra_local_files`]: local content missing remotely, as `deleted` nodes;
//! 7. [`PlanTree::recompute_changed`].
//!
//! [`PlanSummary`] then turns a selection into download and delete tasks.

mod destination;
mod extra;
mod local_state;
mod summary;

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::{Component, Path, PathBuf};

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_transport::CancellationToken;

use crate::event_filter;
use crate::events::{EventSender, Phase, SyncEvent};
use crate::model::PlanTree;
use crate::options::{SyncOptions, SyncOptionsError};
use crate::remote::RemoteMetadata;

pub use destination::assign_destinations;
pub use extra::add_extra_local_files;
pub use local_state::{HashStats, compute_local_sha1, is_hidden_path, mark_hidden, mark_updated};
pub use summary::{
    PlanSummary, ReportTotals, downloaded_compressed, resumed_size, resumed_tasks,
    select_all_except_root,
};

/// Suffix of partially downloaded files; local files whose name contains it are never extra.
// Mirrors dao/DataAccessConstants.java:22
pub const PART_EXTENSION: &str = ".part";

/// Suffix of compressed PBO files; local files whose name contains it are never extra.
// Mirrors dao/DataAccessConstants.java:26
pub const PBO_ZIP_EXTENSION: &str = ".pbo.zip";

/// Where auto-discovered addon folders live.
///
/// The addon registry will sit above `a3s_sync`, so this crate cannot depend on it; callers
/// adapt theirs to this trait.
pub trait AddonLocator: Sync {
    /// The local folder that **contains** the addon folder named `directory_name` (Java
    /// `Addon.path`), if known.
    ///
    /// `directory_name` is the name as written in the sync tree; implementations look up its
    /// lowercase form (the Java addon map is keyed by lowercase names).
    fn addon_parent_dir(&self, directory_name: &str) -> Option<PathBuf>;
}

/// An [`AddonLocator`] that knows no addon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoAddons;

impl AddonLocator for NoAddons {
    fn addon_parent_dir(&self, _directory_name: &str) -> Option<PathBuf> {
        None
    }
}

/// A map from lowercase addon folder names to the folder containing each.
// Mirrors dao/AddonDAO.java:14-18
impl<S: BuildHasher + Sync> AddonLocator for HashMap<String, PathBuf, S> {
    fn addon_parent_dir(&self, directory_name: &str) -> Option<PathBuf> {
        self.get(&directory_name.to_lowercase()).cloned()
    }
}

/// A closure receiving the name as written in the sync tree.
impl<F: Fn(&str) -> Option<PathBuf> + Sync> AddonLocator for F {
    fn addon_parent_dir(&self, directory_name: &str) -> Option<PathBuf> {
        self(directory_name)
    }
}

/// Planning failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    /// The repository has no `.a3s/sync` (not checked yet, or the check failed).
    // Mirrors exception/repository/SyncFileNotFoundException.java
    #[error("File /.a3s/sync not found on repository: {repository}")]
    SyncMissing {
        /// The repository name.
        repository: String,
    },

    /// An event is selected but the repository has no `.a3s/events`.
    // Mirrors exception/repository/EventsFileNotFoundException.java
    #[error("File /.a3s/events not found on repository: {repository}")]
    EventsMissing {
        /// The repository name.
        repository: String,
    },

    /// The selected event does not exist (Java plans an empty tree; approved deviation DV-P9).
    #[error("Event not found: {name}")]
    EventNotFound {
        /// The event name.
        name: String,
    },

    /// The destination folder is unset or missing.
    #[error(transparent)]
    Options(#[from] SyncOptionsError),

    /// A local file could not be hashed.
    #[error("{} ({source})", path.display())]
    Hash {
        /// The local file.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The plan was cancelled.
    #[error("Cancelled")]
    Cancelled,

    /// A remote node name is not a single plain path component (e.g. `..`, `/etc`, `a/b`,
    /// `C:x`): joining it to a local folder could reach outside the destination.
    #[error("Unsafe file name in the repository: {relative_path}")]
    #[non_exhaustive]
    UnsafeName {
        /// Path of the offending node in the plan tree.
        relative_path: String,
    },
}

impl UserFacing for PlanError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::SyncMissing { .. } | Self::EventsMissing { .. } | Self::EventNotFound { .. } => {
                ErrorKind::NotFound
            }
            Self::Options(e) => e.kind(),
            Self::Hash { source, .. } => ErrorKind::from_io(source),
            Self::Cancelled => ErrorKind::Cancelled,
            Self::UnsafeName { .. } => ErrorKind::InvalidData,
        }
    }

    /// The Java message; it already contains the cause.
    fn user_message(&self) -> String {
        self.to_string()
    }
}

/// Plans the synchronization of a checked repository into `options.destination`.
///
/// Blocking (file system and SHA-1 hashing on up to 8 threads): async callers run it in
/// `tokio::task::spawn_blocking`. `cache` is updated with the hashes computed; callers persist
/// it afterwards. The remote tree in `metadata` is not modified.
///
/// Emits [`Phase::CheckingFiles`], then [`SyncEvent::Progress`] while hashing.
// Mirrors service/RepositoryService.java:413-475
pub fn plan_repository(
    metadata: &RemoteMetadata,
    options: &SyncOptions,
    addons: &dyn AddonLocator,
    cache: &mut a3s_repo_files::hash::Sha1Cache,
    cancel: &CancellationToken,
    events: &EventSender,
) -> Result<PlanTree, PlanError> {
    let repository = || options.repository_name.clone();
    let Some(sync) = metadata.sync.as_deref() else {
        return Err(PlanError::SyncMissing {
            repository: repository(),
        });
    };
    let event_source = match &options.event_name {
        Some(name) => match metadata.events.as_deref() {
            Some(list) => Some((name, list)),
            None => {
                return Err(PlanError::EventsMissing {
                    repository: repository(),
                });
            }
        },
        None => None,
    };
    check_destination(&options.destination)?;

    // Mirrors ui/repository/workers/AddonsChecker.java:119
    events.emit(SyncEvent::Phase(Phase::CheckingFiles));
    let mut tree = PlanTree::from_sync_tree(sync);

    // 1. Event addons.
    if let Some((name, list)) = event_source {
        let event = event_filter::find_event(list, name)
            .ok_or_else(|| PlanError::EventNotFound { name: name.clone() })?;
        tree = event_filter::filter(tree, event);
    }
    // 2. Destination paths (rejects unsafe remote names first).
    assign_destinations(
        &mut tree,
        &options.destination,
        options.auto_discover,
        addons,
    )?;
    // 3. Local SHA-1.
    compute_local_sha1(&mut tree, cache, cancel, events)?;
    // 4. New or updated files.
    mark_updated(&mut tree);
    // 5. Hidden folders.
    mark_hidden(&mut tree, &options.hidden_folder_paths);
    // 6. Extra local files.
    add_extra_local_files(&mut tree, &options.destination, options.exact_match);
    if cancel.is_cancelled() {
        return Err(PlanError::Cancelled);
    }
    // Java computes `changed` in the DTO transform that follows.
    // Mirrors service/ObjectDTOtransformer.java:302-308
    tree.recompute_changed();
    Ok(tree)
}

/// The destination may have vanished since the options were built.
// Mirrors service/RepositoryService.java:435-441
fn check_destination(destination: &Path) -> Result<(), SyncOptionsError> {
    if destination.as_os_str().is_empty() {
        return Err(SyncOptionsError::DestinationEmpty);
    }
    if !destination.exists() {
        return Err(SyncOptionsError::DestinationMissing(
            destination.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

/// Checks that every node name below the root is a single plain path component.
///
/// A name is accepted only if `Path::new(name)` has exactly one [`Component::Normal`] equal to
/// the name, it contains none of `/`, `\`, `\0`, `:`, it does not end with `.` or a space
/// (Windows strips them, so `x.` would alias `x`), and it is not a Windows device name (`CON`,
/// `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`, any case, with or without an extension).
/// The rules apply on every platform, so a repository is planned the same way everywhere.
/// Java joins names unchecked
/// (`RepositoryService.java:487-508, 613-656`), so a hostile `.a3s/sync` could make it read,
/// write or delete outside the destination.
pub fn validate_names(tree: &PlanTree) -> Result<(), PlanError> {
    let mut unsafe_path = None;
    tree.walk(&mut |relative_path, node| {
        if unsafe_path.is_none() && !is_safe_name(node.name()) {
            unsafe_path = Some(relative_path.to_owned());
        }
    });
    match unsafe_path {
        Some(relative_path) => Err(PlanError::UnsafeName { relative_path }),
        None => Ok(()),
    }
}

/// Whether `name` is a single plain path component (see [`validate_names`]).
pub fn is_safe_name(name: &str) -> bool {
    if name.contains(['/', '\\', '\0', ':']) || name.ends_with(['.', ' ']) {
        return false;
    }
    if is_windows_device_name(name) {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(c)), None) if c == std::ffi::OsStr::new(name)
    )
}

/// Whether the part of `name` before its first `.` is a Windows device name.
fn is_windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let (prefix, digit) = stem.split_at(stem.len().min(3));
            matches!(prefix, "COM" | "LPT")
                && digit.len() == 1
                && digit.as_bytes()[0].is_ascii_digit()
                && digit != "0"
        }
    }
}

/// `std::path::absolute(path)`, or `path` itself if that fails (empty path, no current dir).
///
/// Like Java `File.getAbsolutePath()`: no symlink resolution, no `..` folding.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use a3s_types::{Event, Events, SyncTreeDirectory};

    use super::*;

    fn options(destination: &Path) -> SyncOptions {
        SyncOptions {
            repository_name: "WOG".to_owned(),
            event_name: None,
            destination: destination.to_path_buf(),
            auto_discover: true,
            exact_match: false,
            hidden_folder_paths: Default::default(),
            client_connections: 1,
            max_download_speed: 0,
            max_errors: 10,
        }
    }

    fn plan(metadata: &RemoteMetadata, options: &SyncOptions) -> Result<PlanTree, PlanError> {
        plan_repository(
            metadata,
            options,
            &NoAddons,
            &mut a3s_repo_files::hash::Sha1Cache::default(),
            &CancellationToken::new(),
            &EventSender::noop(),
        )
    }

    #[test]
    fn plan_error_messages_and_kinds() {
        let err = PlanError::SyncMissing {
            repository: "WOG".to_owned(),
        };
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = PlanError::EventsMissing {
            repository: "WOG".to_owned(),
        };
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = PlanError::EventNotFound {
            name: "op".to_owned(),
        };
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = PlanError::from(SyncOptionsError::DestinationEmpty);
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = PlanError::from(SyncOptionsError::DestinationMissing("/x".to_owned()));
        assert_eq!(err.user_message(), err.to_string());
        assert_eq!(err.kind(), ErrorKind::NotFound);
        let err = PlanError::Hash {
            path: PathBuf::from("/x/y"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(err.to_string().starts_with("/x/y ("), "{err}");
        assert_eq!(err.user_message(), err.to_string());
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        assert_eq!(PlanError::Cancelled.kind(), ErrorKind::Cancelled);
    }

    #[test]
    fn plan_repository_precondition_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing");
        let mut opts = options(&missing);
        opts.event_name = Some("e".to_owned());

        // Sync before events before destination.
        let mut metadata = RemoteMetadata::default();
        assert!(matches!(
            plan(&metadata, &opts),
            Err(PlanError::SyncMissing { repository }) if repository == "WOG"
        ));
        metadata.sync = Some(Arc::new(SyncTreeDirectory::new_root()));
        assert!(matches!(
            plan(&metadata, &opts),
            Err(PlanError::EventsMissing { .. })
        ));
        metadata.events = Some(Arc::new(Events::default()));
        assert!(matches!(
            plan(&metadata, &opts),
            Err(PlanError::Options(SyncOptionsError::DestinationMissing(p)))
                if p == missing.to_string_lossy()
        ));
        opts.destination = PathBuf::new();
        assert!(matches!(
            plan(&metadata, &opts),
            Err(PlanError::Options(SyncOptionsError::DestinationEmpty))
        ));
        opts.destination = dir.path().to_path_buf();
        assert!(matches!(
            plan(&metadata, &opts),
            Err(PlanError::EventNotFound { name }) if name == "e"
        ));
        metadata.events = Some(Arc::new(Events {
            list: vec![Event {
                name: "e".to_owned(),
                ..Default::default()
            }],
        }));
        let tree = plan(&metadata, &opts).expect("plan");
        assert!(tree.root.children.is_empty());

        // Without an event, missing events are fine.
        metadata.events = None;
        opts.event_name = None;
        assert!(plan(&metadata, &opts).is_ok());
    }

    #[test]
    fn addon_locator_impls() {
        assert_eq!(NoAddons.addon_parent_dir("@ace"), None);
        let map: HashMap<String, PathBuf> = [("@ace".to_owned(), PathBuf::from("/opt"))].into();
        assert_eq!(map.addon_parent_dir("@ACE"), Some(PathBuf::from("/opt")));
        assert_eq!(map.addon_parent_dir("@cba"), None);
        let closure = |name: &str| (name == "@ACE").then(|| PathBuf::from("/c"));
        assert_eq!(closure.addon_parent_dir("@ACE"), Some(PathBuf::from("/c")));
        assert_eq!(closure.addon_parent_dir("@ace"), None);
    }
}
