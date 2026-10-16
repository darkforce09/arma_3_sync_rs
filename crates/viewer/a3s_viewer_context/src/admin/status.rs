//! The publishing status of a repository built on this computer: the revision the server holds
//! and the source folder compared with it, without hashing (the build's SHA-1 cache tells which
//! unchanged files still match).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_admin::{AdminMetadata, EventsDoc, fetch_admin_metadata};
use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_repo_files::A3sFile;
use a3s_repo_files::hash::{Sha1Cache, modified_millis};
use a3s_repo_files::layout::{A3S_FOLDER, ZIP_EXTENSION, is_builder_ignored_file, is_compressible};
use a3s_repo_files::local::CacheKind;
use a3s_sync::{EventError, RemoteConnection, RemoteError};
use a3s_transport::proxy::transport_options;
use a3s_types::local::Repository;
use a3s_types::{Changelog, Changelogs, ServerInfo, SyncTreeDirectory, SyncTreeNode};

use super::PublishRun;
use super::server::{RangeTest, ServerCheck};
use crate::error::ContextError;
use crate::event::{AppEvent, Change};
use crate::state::AppState;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskId, TaskKey, TaskKind};

/// The repository the server holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteRepository {
    /// `.a3s/serverinfo`; `None` when nothing is published yet.
    pub server_info: Option<Arc<ServerInfo>>,

    /// `.a3s/sync`; `None` when nothing is published yet.
    pub sync: Option<Arc<SyncTreeDirectory>>,
}

/// What is known about the published repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Remote {
    /// Not read yet.
    #[default]
    NotLoaded,

    /// Being read.
    Loading,

    /// Read.
    Loaded(RemoteRepository),

    /// Reading failed.
    Failed(EventError),
}

impl Remote {
    /// The published repository, when it was read.
    pub fn repository(&self) -> Option<&RemoteRepository> {
        match self {
            Self::Loaded(repository) => Some(repository),
            _ => None,
        }
    }
}

/// Mods (folders marked as addons) that differ between two contents, by folder name, each list
/// sorted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModChanges {
    /// Mods that are new.
    pub added: Vec<String>,

    /// Mods with new, changed or removed files.
    pub updated: Vec<String>,

    /// Mods that are gone.
    pub removed: Vec<String>,
}

impl ModChanges {
    /// Whether no mod changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// The source folder compared with the published repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalScan {
    /// Files the build would publish.
    pub files: usize,

    /// Their size in bytes.
    pub bytes: u64,

    /// Mod folder names, sorted, each once.
    pub mods: Vec<String>,

    /// Files that are new, changed or gone since the published revision (all when nothing is
    /// published). A file the build has not hashed since it last changed counts as changed.
    pub changed_files: usize,

    /// Bytes of the new and changed files.
    pub changed_bytes: u64,

    /// The mods those files belong to.
    pub changes: ModChanges,

    /// The revision of the last local build (`.a3s/serverinfo` in the source folder).
    pub built_revision: Option<i32>,

    /// The last builds, newest first (`.a3s/changelogs` in the source folder, which keeps ten).
    pub history: Vec<Changelog>,
}

/// What the Publishing screen knows about one repository built on this computer, for this
/// session.
#[derive(Clone, Debug, Default)]
pub struct Publishing {
    /// The published repository.
    pub remote: Remote,

    /// The last comparison of the source folder with the published repository.
    pub local: Option<Result<LocalScan, EventError>>,

    /// The events being edited, saved to the source folder's `.a3s/events` on every change.
    pub events: Option<EventsDoc>,

    /// The events differ from the uploaded ones.
    pub events_changed: bool,

    /// The running or last publish.
    pub run: Option<PublishRun>,

    /// The running or last partial transfer test of the server.
    pub range_test: Option<RangeTest>,

    /// The running or last check of the server's files.
    pub server_check: Option<ServerCheck>,
}

impl Publishing {
    /// The revision the server holds, when known.
    pub fn published_revision(&self) -> Option<i32> {
        self.remote
            .repository()
            .and_then(|remote| remote.server_info.as_ref())
            .map(|info| info.revision)
    }

    /// The revision the next publish creates: one above the published or locally built one.
    pub fn next_revision(&self) -> i32 {
        let built = self
            .local
            .as_ref()
            .and_then(|local| local.as_ref().ok())
            .and_then(|local| local.built_revision);
        self.published_revision()
            .max(built)
            .map_or(1, |revision| revision.saturating_add(1))
    }
}

/// Forgets the publishing status of `repository`, e.g. after its source folder changed; the
/// screen reads it again.
pub fn forget(state: &mut AppState, repository: &str) {
    state.admin.repositories.remove(repository);
}

/// Compares the source folder of `repository` with the published repository.
///
/// Reads the server first when `remote` is set (or nothing was read yet). Loads the events when
/// none are being edited: the source folder's `.a3s/events` when it differs from the published
/// one, otherwise the published one. Does nothing while a publish or another refresh runs.
pub fn refresh(cx: &Cx, repository: &str, remote: bool) -> Result<Option<TaskId>, ContextError> {
    let busy = [TaskKind::Publish, TaskKind::PublishingStatus]
        .into_iter()
        .any(|kind| {
            cx.is_running(&TaskKey::repository(kind, repository))
                .is_some()
        });
    if busy {
        return Ok(None);
    }
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::PublishingStatus, repository),
        true,
        move |task| run_refresh(task, name, remote),
    )
    .map(Some)
}

async fn run_refresh(task: TaskCtx, name: String, remote: bool) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let (repository, proxy) = {
        let state = cx.state().read();
        (
            state.client.repository(&name)?.clone(),
            state.client.proxy().clone(),
        )
    };
    let read_remote = remote
        || matches!(
            cx.state().read().admin.publishing(&name).remote,
            Remote::NotLoaded | Remote::Failed(_)
        );
    let mut remote_events = None;
    if read_remote {
        set(&cx, &name, |publishing| publishing.remote = Remote::Loading);
        let fetched = fetch_remote(&task, &repository, &proxy).await;
        let remote = match fetched {
            Ok((remote, events)) => {
                remote_events = events;
                Remote::Loaded(remote)
            }
            Err(error) if error.kind == ErrorKind::Cancelled => {
                set(&cx, &name, |publishing| {
                    publishing.remote = Remote::NotLoaded;
                });
                return Err(TaskError::Cancelled);
            }
            Err(error) => Remote::Failed(error),
        };
        set(&cx, &name, |publishing| publishing.remote = remote);
    }

    let published = cx
        .state()
        .read()
        .admin
        .publishing(&name)
        .remote
        .repository()
        .and_then(|remote| remote.sync.clone());
    let store = Arc::clone(cx.state().read().client.store());
    let scanned = {
        let repository = repository.clone();
        blocking(move || {
            let cache = store.read_sha1_cache(&repository.name, CacheKind::Build);
            let scan = scan_source(&repository, published.as_deref(), &cache);
            (scan, local_events(&repository))
        })
        .await
    };
    let Some((scan, local_events)) = scanned else {
        return Err(TaskError::Cancelled);
    };
    set(&cx, &name, |publishing| {
        publishing.local = Some(scan);
        if publishing.events.is_none() {
            let (doc, changed) = choose_events(local_events, remote_events);
            publishing.events = Some(doc);
            publishing.events_changed = changed;
        }
    });
    Ok(())
}

/// Reads the published metadata over the download protocol. A missing `.a3s/sync` means
/// nothing is published yet.
async fn fetch_remote(
    task: &TaskCtx,
    repository: &Repository,
    proxy: &a3s_types::local::Proxy,
) -> Result<(RemoteRepository, Option<EventsDoc>), EventError> {
    let protocol = &repository.protocol;
    let options = transport_options(protocol, proxy)
        .map_err(|err| EventError::new(ErrorKind::InvalidInput, err.to_string()))?;
    let ctx = task.transfer_ctx();
    let mut connection = RemoteConnection::connect(protocol, &options, &ctx)
        .await
        .map_err(|err| EventError::from_remote(&err))?;
    let mut metadata = AdminMetadata::default();
    let fetched =
        fetch_admin_metadata(&mut connection, &repository.name, &mut metadata, &ctx).await;
    connection.close().await;
    match fetched {
        Ok(()) => {}
        Err(RemoteError::Cancelled) => {
            return Err(EventError::new(ErrorKind::Cancelled, "Cancelled"));
        }
        Err(err) if err.is_not_found() => {}
        Err(err) => return Err(EventError::from_remote(&err)),
    }
    let remote = if metadata.sync.is_some() {
        RemoteRepository {
            server_info: metadata.server_info,
            sync: metadata.sync,
        }
    } else {
        RemoteRepository::default()
    };
    Ok((remote, metadata.events))
}

/// The source folder's `.a3s/events`, when it exists and can be read.
fn local_events(repository: &Repository) -> Option<EventsDoc> {
    let path = Path::new(repository.path.as_deref()?)
        .join(A3S_FOLDER)
        .join(A3sFile::Events.file_name());
    let bytes = fs::read(&path).ok()?;
    match EventsDoc::decode(&bytes) {
        Ok(doc) => Some(doc),
        Err(err) => {
            tracing::warn!("Cannot read {}: {}", path.display(), err.user_message());
            None
        }
    }
}

/// The events to edit and whether they differ from the published ones.
fn choose_events(local: Option<EventsDoc>, remote: Option<EventsDoc>) -> (EventsDoc, bool) {
    match (local, remote) {
        (Some(local), Some(remote)) => {
            let changed = local.to_events() != remote.to_events();
            (local, changed)
        }
        (Some(local), None) => {
            let changed = !local.events().is_empty();
            (local, changed)
        }
        (None, Some(remote)) => (remote, false),
        (None, None) => (EventsDoc::new(), false),
    }
}

/// Runs `f` on the publishing status of `name` and asks the UI to redraw.
pub(super) fn set(cx: &Cx, name: &str, f: impl FnOnce(&mut Publishing)) {
    {
        let mut state = cx.state().write();
        if state.client.repository(name).is_err() {
            return;
        }
        f(state.admin.publishing_mut(name));
    }
    cx.emit(AppEvent::Changed(Change::Repository));
}

/// Runs blocking work off the async threads; `None` when the runtime shuts down.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => Some(value),
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        Err(_) => None,
    }
}

/// A file of a content: its size and SHA-1 (`None`: unknown, differs from everything).
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileEntry {
    size: i64,
    sha1: Option<String>,
}

/// The files of a content by `/`-joined path, and the paths of its mod folders.
#[derive(Debug, Default)]
struct Content {
    files: BTreeMap<String, FileEntry>,
    mods: BTreeSet<String>,
}

impl Content {
    /// The mod folder `path` belongs to: its ancestor marked as a mod. Only the outermost
    /// marked folder of a branch is a mod.
    fn mod_of<'a>(&self, path: &'a str) -> Option<&'a str> {
        path.match_indices('/')
            .map(|(index, _)| &path[..index])
            .find(|prefix| self.mods.contains(*prefix))
    }

    fn of_tree(tree: &SyncTreeDirectory) -> Self {
        let mut content = Self::default();
        tree.walk(&mut |path, node| match node {
            SyncTreeNode::Directory(directory)
                if directory.mark_as_addon && content.mod_of(&format!("{path}/")).is_none() =>
            {
                content.mods.insert(path.to_owned());
            }
            SyncTreeNode::Directory(_) => {}
            SyncTreeNode::Leaf(leaf) => {
                content.files.insert(
                    path.to_owned(),
                    FileEntry {
                        size: leaf.size,
                        sha1: leaf.sha1.clone(),
                    },
                );
            }
        });
        content
    }
}

/// Files that changed from `published` to `current`, with their bytes and mods.
fn compare(published: &Content, current: &Content) -> (usize, u64, ModChanges) {
    let mut changed_files = 0;
    let mut changed_bytes = 0u64;
    let mut changed_mods: BTreeSet<&str> = BTreeSet::new();
    for (path, entry) in &current.files {
        let same = published.files.get(path).is_some_and(|old| {
            old.size == entry.size && old.sha1.is_some() && old.sha1 == entry.sha1
        });
        if !same {
            changed_files += 1;
            changed_bytes =
                changed_bytes.saturating_add(u64::try_from(entry.size).unwrap_or_default());
            if let Some(name) = current.mod_of(path) {
                changed_mods.insert(name);
            }
        }
    }
    for path in published.files.keys() {
        if !current.files.contains_key(path) {
            changed_files += 1;
            if let Some(name) = published.mod_of(path) {
                changed_mods.insert(name);
            }
        }
    }
    let mut changes = ModChanges::default();
    for path in &current.mods {
        if !published.mods.contains(path) {
            changes.added.push(folder_name(path));
        } else if changed_mods.contains(path.as_str()) {
            changes.updated.push(folder_name(path));
        }
    }
    for path in &published.mods {
        if !current.mods.contains(path) {
            changes.removed.push(folder_name(path));
        }
    }
    for list in [
        &mut changes.added,
        &mut changes.updated,
        &mut changes.removed,
    ] {
        list.sort_by_key(|name| name.to_lowercase());
    }
    (changed_files, changed_bytes, changes)
}

fn folder_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

/// The mods that differ between the published repository and a built one.
pub(super) fn built_changes(
    published: Option<&SyncTreeDirectory>,
    built: &SyncTreeDirectory,
) -> ModChanges {
    let published = published.map(Content::of_tree).unwrap_or_default();
    compare(&published, &Content::of_tree(built)).2
}

/// Compares the source folder with `published` (nothing published: `None`).
pub(super) fn scan_source(
    repository: &Repository,
    published: Option<&SyncTreeDirectory>,
    cache: &Sha1Cache,
) -> Result<LocalScan, EventError> {
    let path = repository
        .path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            EventError::new(ErrorKind::InvalidInput, "Choose the source folder first.")
        })?;
    let root = std::path::absolute(path)
        .map_err(|err| EventError::new(ErrorKind::from_io(&err), err.to_string()))?;
    if !root.is_dir() {
        return Err(EventError::new(
            ErrorKind::NotFound,
            format!("The source folder {} does not exist.", root.display()),
        ));
    }
    let mut walker = Walker {
        excluded: &repository.excluded_files_from_build,
        cache,
        content: Content::default(),
        stack: Vec::new(),
    };
    walker
        .directory(&root, "")
        .map_err(|err| EventError::new(ErrorKind::from_io(&err), err.to_string()))?;
    let current = walker.content;
    let published = published.map(Content::of_tree).unwrap_or_default();
    let (changed_files, changed_bytes, changes) = compare(&published, &current);
    let mut mods: Vec<String> = current.mods.iter().map(|path| folder_name(path)).collect();
    mods.sort_by_key(|name| name.to_lowercase());
    mods.dedup();
    let a3s = root.join(A3S_FOLDER);
    let built_revision =
        a3s_repo_files::read_a3s::<ServerInfo>(&a3s.join(A3sFile::ServerInfo.file_name()))
            .ok()
            .map(|info| info.revision);
    let mut history =
        a3s_repo_files::read_a3s::<Changelogs>(&a3s.join(A3sFile::Changelogs.file_name()))
            .map(|changelogs| changelogs.list)
            .unwrap_or_default();
    history.sort_by_key(|changelog| std::cmp::Reverse(changelog.revision));
    Ok(LocalScan {
        files: current.files.len(),
        bytes: current
            .files
            .values()
            .map(|entry| u64::try_from(entry.size).unwrap_or_default())
            .fold(0, u64::saturating_add),
        mods,
        changed_files,
        changed_bytes,
        changes,
        built_revision,
        history,
    })
}

/// Walks the source folder with the build's rules: `.a3s` folders, `.zsync` files and the
/// build's `.zip` outputs are skipped, and so are the excluded files.
// Mirrors dao/repository/RepositoryBuildProcessor.java:468-497
struct Walker<'a> {
    excluded: &'a BTreeSet<String>,
    cache: &'a Sha1Cache,
    content: Content,

    /// Canonical paths of the folders being walked, against symbolic link loops.
    stack: Vec<PathBuf>,
}

impl Walker<'_> {
    fn directory(&mut self, path: &Path, relative: &str) -> io::Result<()> {
        let canonical = fs::canonicalize(path)?;
        if self.stack.contains(&canonical) {
            return Err(io::Error::other(format!(
                "{} is a symbolic link loop",
                path.display()
            )));
        }
        let mut entries: Vec<(String, PathBuf)> = fs::read_dir(path)?
            .map(|entry| entry.map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())))
            .collect::<io::Result<_>>()?;
        entries.sort();
        if !relative.is_empty()
            && entries
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("addons"))
            && self.content.mod_of(&format!("{relative}/")).is_none()
        {
            self.content.mods.insert(relative.to_owned());
        }
        self.stack.push(canonical);
        for (name, child) in entries {
            let child_relative = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            if fs::metadata(&child).is_ok_and(|m| m.is_dir()) {
                if !name.eq_ignore_ascii_case(A3S_FOLDER) {
                    self.directory(&child, &child_relative)?;
                }
            } else if !is_skipped(&name)
                && !self.excluded.contains(child.to_string_lossy().as_ref())
            {
                let size = i64::try_from(fs::metadata(&child)?.len()).unwrap_or(i64::MAX);
                let sha1 = modified_millis(&child)
                    .ok()
                    .and_then(|modified| self.cache.get(&child, modified))
                    .map(str::to_owned);
                self.content
                    .files
                    .insert(child_relative, FileEntry { size, sha1 });
            }
        }
        self.stack.pop();
        Ok(())
    }
}

/// Whether the build leaves a file out of the repository.
fn is_skipped(name: &str) -> bool {
    is_builder_ignored_file(name)
        || name
            .strip_suffix(ZIP_EXTENSION)
            .is_some_and(is_compressible)
}
