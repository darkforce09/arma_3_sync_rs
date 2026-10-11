//! Building a repository from a local folder, exactly like Arma3Sync's Java builder.
//!
//! [`build_repository`] scans the repository folder into a sync tree, hashes the files (reusing
//! the SHA-1 cache), derives the revision, server info, autoconfig, changelogs and events,
//! generates `.zsync` files (HTTP repositories) and `.zip` files (compressed repositories), then
//! writes the five `.a3s` files. Everything that ends up in `.a3s/` is byte-compatible with the
//! Java implementation, including `String` instance sharing and `HashMap` layouts.
//!
//! Approved deviations from Java: a cancelled build returns [`BuildError::Cancelled`] and writes
//! nothing; `.zsync` files are generated in parallel and written atomically;
//! streams Java could not read back fail with [`BuildError::LimitExceeded`]; directory
//! listings are sorted by name; symbolic link loops fail the scan; the compression
//! stage's own `.ebo.zip` outputs are not sync leaves.

mod carry;
mod changelogs;
mod events;
mod hash;
mod java_hash;
mod metadata;
mod scan;
mod write;
mod zip;
mod zsync;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_repo_files::A3sFile;
use a3s_repo_files::hash::Sha1Cache;
use a3s_repo_files::layout::{A3S_FOLDER, is_compressible};
use a3s_types::local::Repository;
use a3s_types::{AutoConfig, Changelogs, Events, JavaDate, ServerInfo, SyncTreeDirectory};

use self::hash::Sha1Source;
use self::zip::ZipAction;

/// Stage text of the SHA-1 stage.
pub const STAGE_SHA1: &str = "Processing SHA1 signatures...";

/// Stage text of the `.zsync` stage (HTTP repositories).
pub const STAGE_ZSYNC: &str = "Processing *.zsync files...";

/// Stage text of the `.zsync` deletion stage (other repositories).
pub const STAGE_DELETE_ZSYNC: &str = "Deleting *.zsync files...";

/// Stage text of the compression stage (compressed repositories).
pub const STAGE_ZIP: &str = "Processing *.pbo.zip files...";

/// Stage text of the zip deletion stage (uncompressed repositories).
pub const STAGE_DELETE_ZIP: &str = "Deleting *.pbo.zip files...";

/// Options of a build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// `ServerInfo.buildDate` and the new `Changelog.buildDate`, ms since the epoch. `None` = now.
    pub build_date: Option<JavaDate>,
}

/// Progress of a build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildEvent {
    /// Verbatim Java stage text, e.g. [`STAGE_SHA1`].
    Stage(&'static str),

    /// Progress of the current stage, 0..=100.
    Progress(u8),

    /// Non-fatal warning (pruned nodes, failed deletions).
    Warning(String),
}

/// Result of a successful build. The metadata fields equal what reading the written files gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildOutcome {
    /// `.a3s/sync`.
    pub sync: SyncTreeDirectory,

    /// `.a3s/serverinfo`.
    pub server_info: ServerInfo,

    /// `.a3s/changelogs`.
    pub changelogs: Changelogs,

    /// `.a3s/autoconfig`.
    pub auto_config: AutoConfig,

    /// `.a3s/events`.
    pub events: Events,

    /// Whether the content changed since the previous build (`repositoryContentUpdated`).
    pub content_updated: bool,

    /// Relative paths whose SHA-1 was recomputed (Java `updatedFiles`), in deep-search order.
    pub rehashed: Vec<String>,

    /// Number of `.zsync` files generated.
    pub zsync_generated: usize,

    /// Number of files compressed.
    pub zipped: usize,

    /// Number of `.zip` files deleted (uncompressed repositories).
    pub zips_deleted: usize,
}

impl BuildOutcome {
    /// `repository.revision = server_info.revision`. The caller then sets the autoconfig URL
    /// and persists the repository and the SHA-1 cache.
    // Mirrors service/RepositoryService.java:917-918
    pub fn apply_to(&self, repository: &mut Repository) {
        repository.revision = self.server_info.revision;
    }
}

/// Why a build failed. Nothing is written to `.a3s/` unless the error is [`BuildError::Write`].
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    /// The repository has no main folder.
    // Mirrors exception/repository/RepositoryMainFolderLocationNotFoundException.java
    #[error("Repository {repository} main folder location is empty!")]
    PathNotSet { repository: String },

    /// The main folder does not exist.
    #[error("Repository path does not exist.")]
    PathNotFound { path: PathBuf },

    /// A previous `.a3s` file exists but cannot be read.
    #[error("Cannot read the previous .a3s/{}: {source}", file.file_name())]
    ReadMetadata {
        file: A3sFile,
        #[source]
        source: a3s_repo_files::Error,
    },

    /// Scanning the folder failed (vanished file, broken link, symbolic link loop, I/O error).
    #[error("Cannot scan {}: {source}", path.display())]
    Scan {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Hashing a file failed.
    #[error("Cannot compute the SHA-1 of {}: {source}", path.display())]
    Hash {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// An excluded folder whose hidden path Java cannot compute (`StringIndexOutOfBounds`).
    #[error("Cannot determine the hidden folder path of {path}")]
    HiddenFolder { path: String },

    /// Generating a `.zsync` file failed.
    #[error("Can't create .zsync metafile {}: {source}", path.display())]
    Zsync {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Compressing a file failed.
    // Mirrors dao/zip/ZipDAO.java:160-170
    #[error("Failed to zip file: {}\n{source}", path.display())]
    Zip {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// `.a3s/` cannot be created.
    // Mirrors exception/CreateDirectoryException.java:17-21
    #[error("Cannot create directory {}\nPlease checkout file access permissions.", path.display())]
    CreateA3sFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The stream would exceed what Java's reader accepts.
    #[error("The .a3s/{} file would be too large for Arma3Sync clients: {source}", file.file_name())]
    LimitExceeded {
        file: A3sFile,
        #[source]
        source: a3s_jserial::Error,
    },

    /// Encoding an `.a3s` file failed (e.g. a string longer than 65535 bytes).
    #[error("Cannot encode .a3s/{}: {source}", file.file_name())]
    Encode {
        file: A3sFile,
        #[source]
        source: a3s_types::Error,
    },

    /// Writing an `.a3s` file failed; the files written before it stay.
    #[error("Cannot write .a3s/{}: {source}", file.file_name())]
    Write {
        file: A3sFile,
        #[source]
        source: a3s_repo_files::Error,
    },

    /// The build was cancelled; nothing was written to `.a3s/`.
    #[error("Build cancelled.")]
    Cancelled,
}

/// Counter for `count * 100 / total` progress values.
pub(crate) struct Progress {
    count: usize,
    total: usize,
    last: Option<u8>,
}

impl Progress {
    pub(crate) fn new(total: usize) -> Self {
        Self {
            count: 0,
            total,
            last: None,
        }
    }

    /// Counts one item and returns the new percentage.
    pub(crate) fn increment(&mut self) -> u8 {
        self.count += 1;
        let value = self.count * 100 / self.total.max(1);
        u8::try_from(value.min(100)).unwrap_or(100)
    }

    /// Like [`Self::increment`], but only returns values that changed.
    // Mirrors dao/repository/RepositorySHA1Processor.java:170-177
    pub(crate) fn increment_changed(&mut self) -> Option<u8> {
        let value = self.increment();
        (self.last != Some(value)).then(|| {
            self.last = Some(value);
            value
        })
    }
}

/// `Runtime.availableProcessors()`.
pub(crate) fn cpu_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// `max(1, min(8, availableProcessors))`, also used for `.zsync` generation.
pub(crate) fn hash_threads() -> usize {
    cpu_threads().clamp(1, a3s_repo_files::hash::MAX_THREADS)
}

/// A rayon pool with named threads, or `None` (use the global pool) if it cannot be built.
pub(crate) fn worker_pool(prefix: &'static str, threads: usize) -> Option<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(move |i| format!("{prefix}-{i}"))
        .build()
        .ok()
}

/// Deletes a file if it exists; failures are logged and reported as warnings, not errors.
// Mirrors dao/FileAccessMethods.java:74-80
pub(crate) fn delete_file(path: &Path, on_event: &(dyn Fn(BuildEvent) + Sync)) {
    if path.exists()
        && let Err(err) = std::fs::remove_file(path)
    {
        let text = format!("Cannot delete {}: {err}", path.display());
        tracing::warn!("{text}");
        on_event(BuildEvent::Warning(text));
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), BuildError> {
    if cancel.load(Ordering::Relaxed) {
        Err(BuildError::Cancelled)
    } else {
        Ok(())
    }
}

/// Reads `<a3s>/<file>` if it exists.
fn read_old<T: a3s_types::JavaCodec>(a3s: &Path, file: A3sFile) -> Result<Option<T>, BuildError> {
    let path = a3s.join(file.file_name());
    if !path.exists() {
        return Ok(None);
    }
    a3s_repo_files::read_a3s(&path)
        .map(Some)
        .map_err(|source| BuildError::ReadMetadata { file, source })
}

fn now_millis() -> JavaDate {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Builds the repository at `repository.path` and writes its `.a3s` files.
///
/// `cache` is the repository's SHA-1 cache (`mapFilesForBuild`); it keeps the hashes computed so
/// far even when the build fails. The caller persists the cache and the repository afterwards
/// (see [`BuildOutcome::apply_to`]). `cancel` is checked between stages and before each file;
/// `on_event` may be called from worker threads.
// Mirrors dao/repository/RepositoryBuildProcessor.java:76-345
pub fn build_repository(
    repository: &Repository,
    cache: &mut Sha1Cache,
    options: &BuildOptions,
    cancel: &AtomicBool,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Result<BuildOutcome, BuildError> {
    // Mirrors service/RepositoryService.java:390-402
    let path_str = match repository.path.as_deref() {
        Some(p) if !p.is_empty() => p,
        _ => {
            return Err(BuildError::PathNotSet {
                repository: repository.name.clone(),
            });
        }
    };
    let path = Path::new(path_str);
    if !path.exists() {
        return Err(BuildError::PathNotFound {
            path: path.to_path_buf(),
        });
    }
    // `File.getAbsolutePath()`: absolute, symbolic links not resolved.
    let root = std::path::absolute(path).map_err(|source| BuildError::Scan {
        path: path.to_path_buf(),
        source,
    })?;
    let a3s = root.join(A3S_FOLDER);
    let build_date = options.build_date.unwrap_or_else(now_millis);

    // Previous metadata, read before anything else.
    let old_sync: Option<SyncTreeDirectory> = read_old(&a3s, A3sFile::Sync)?;
    let old_server_info: Option<ServerInfo> = read_old(&a3s, A3sFile::ServerInfo)?;
    let old_changelogs: Option<carry::RawGraph> = read_old(&a3s, A3sFile::Changelogs)?;
    let old_events: Option<carry::RawGraph> = read_old(&a3s, A3sFile::Events)?;
    check_cancel(cancel)?;

    // Scan.
    let mut sync = scan::scan(&root, &repository.excluded_files_from_build, on_event)?;
    let leaves = scan::leaves(&sync, &root);
    let mut names = Vec::with_capacity(leaves.len());
    scan::for_each_leaf_mut(&mut sync, &mut |_, leaf| names.push(leaf.name.clone()));
    let number_of_files = i64::try_from(leaves.len()).unwrap_or(i64::MAX);
    let total_files_size = scan::total_files_size(&root)?;
    check_cancel(cancel)?;

    // SHA-1.
    on_event(BuildEvent::Stage(STAGE_SHA1));
    let hashes = hash::hash_leaves(&leaves, cache, cancel, on_event)?;
    scan::for_each_leaf_mut(&mut sync, &mut |i, leaf| {
        leaf.sha1.clone_from(&hashes.sha1[i]);
    });
    let rehashed: Vec<bool> = hashes
        .source
        .iter()
        .map(|s| *s == Sha1Source::Computed)
        .collect();
    let cached_zero: Vec<bool> = hashes
        .source
        .iter()
        .zip(&hashes.sha1)
        .map(|(s, sha1)| *s == Sha1Source::Cached && hash::is_empty_file_sha1(sha1.as_deref()))
        .collect();
    check_cancel(cancel)?;

    // Revision and server info.
    let content_updated = metadata::content_updated(old_sync.as_ref(), &sync);
    let revision = metadata::revision(
        old_server_info.as_ref().map(|s| s.revision),
        content_updated,
    );
    let mut hidden_folder_paths = std::collections::BTreeSet::new();
    for folder in &repository.excluded_folders_from_sync {
        let hidden = metadata::hidden_folder_path(path_str, folder, std::path::MAIN_SEPARATOR)
            .ok_or_else(|| BuildError::HiddenFolder {
                path: folder.clone(),
            })?;
        hidden_folder_paths.insert(hidden);
    }
    let server_info = ServerInfo {
        revision,
        build_date: Some(build_date),
        number_of_files,
        total_files_size,
        hidden_folder_paths,
        number_of_connections: repository.number_of_connections,
        no_partial_file_transfer: repository.no_partial_file_transfer,
        repository_content_updated: content_updated,
        compressed_pbo_files_only: false,
    };

    // AutoConfig.
    let auto_config = metadata::AutoConfigEmit {
        repository_name: repository.name.clone(),
        protocol: repository.protocol.clone(),
        favorite_servers: repository.favorite_servers_set_to_autoconfig.clone(),
    };

    // Changelogs and events.
    let changelogs = changelogs::plan_changelogs(
        old_changelogs.as_ref(),
        old_sync.as_ref(),
        &sync,
        revision,
        build_date,
        content_updated,
    )
    .map_err(|e| BuildError::ReadMetadata {
        file: A3sFile::Changelogs,
        source: e.into(),
    })?;
    let events = events::plan_events(old_events.as_ref(), &changelogs::addon_names(&sync))
        .map_err(|e| BuildError::ReadMetadata {
            file: A3sFile::Events,
            source: e.into(),
        })?;
    check_cancel(cancel)?;

    // zsync.
    let zsync_generated = if repository.protocol.protocol_type.is_http() {
        on_event(BuildEvent::Stage(STAGE_ZSYNC));
        let jobs = zsync::plan(
            &leaves,
            &names,
            &hashes.sha1,
            &rehashed,
            &repository.protocol,
        );
        zsync::run(&jobs, cancel, on_event)?
    } else {
        on_event(BuildEvent::Stage(STAGE_DELETE_ZSYNC));
        zsync::delete_stale(&leaves, &names, on_event);
        0
    };
    check_cancel(cancel)?;

    // Zip.
    let (zipped, zips_deleted) = if repository.compressed {
        on_event(BuildEvent::Stage(STAGE_ZIP));
        let actions = zip::plan(&leaves, &names, &rehashed, on_event);
        let sizes = zip::run(&leaves, &actions, cancel, on_event)?;
        scan::for_each_leaf_mut(&mut sync, &mut |i, leaf| match actions[i] {
            ZipAction::None => {}
            ZipAction::Reuse(size) => {
                leaf.compressed = true;
                leaf.compressed_size = size;
            }
            ZipAction::Compress => {
                leaf.compressed = true;
                leaf.compressed_size = sizes[i].map_or(0, |s| i64::try_from(s).unwrap_or(i64::MAX));
            }
        });
        let zipped = actions
            .iter()
            .filter(|a| **a == ZipAction::Compress)
            .count();
        (zipped, 0)
    } else {
        on_event(BuildEvent::Stage(STAGE_DELETE_ZIP));
        let deleted = zip::delete(&leaves, &names, on_event);
        scan::for_each_leaf_mut(&mut sync, &mut |_, leaf| {
            if is_compressible(&leaf.name) {
                leaf.compressed = false;
            }
        });
        (0, deleted)
    };
    check_cancel(cancel)?;

    // Encode everything before writing anything.
    let sync_emit = write::SyncEmit::new(&sync, &cached_zero);
    let raw_sync = write::encode_checked(A3sFile::Sync, &sync_emit)?;
    let raw_server_info = write::encode_checked(A3sFile::ServerInfo, &server_info)?;
    let raw_changelogs = write::encode_checked(A3sFile::Changelogs, &changelogs)?;
    let raw_auto_config = write::encode_checked(A3sFile::AutoConfig, &auto_config)?;
    let raw_events = write::encode_checked(A3sFile::Events, &events)?;
    let outcome = BuildOutcome {
        sync: write::decode(A3sFile::Sync, &raw_sync)?,
        server_info: write::decode(A3sFile::ServerInfo, &raw_server_info)?,
        changelogs: write::decode(A3sFile::Changelogs, &raw_changelogs)?,
        auto_config: write::decode(A3sFile::AutoConfig, &raw_auto_config)?,
        events: write::decode(A3sFile::Events, &raw_events)?,
        content_updated,
        rehashed: leaves
            .iter()
            .zip(&rehashed)
            .filter(|(_, r)| **r)
            .map(|(l, _)| l.relpath.clone())
            .collect(),
        zsync_generated,
        zipped,
        zips_deleted,
    };
    check_cancel(cancel)?;

    // Write, in Java's order.
    write::create_a3s_folder(&a3s)?;
    write::write(&a3s, A3sFile::Sync, &sync_emit)?;
    write::write(&a3s, A3sFile::ServerInfo, &server_info)?;
    write::write(&a3s, A3sFile::Changelogs, &changelogs)?;
    write::write(&a3s, A3sFile::AutoConfig, &auto_config)?;
    write::write(&a3s, A3sFile::Events, &events)?;
    Ok(outcome)
}

/// Stage functions and wire helpers, exposed for this crate's tests.
#[doc(hidden)]
pub mod testing {
    pub use super::carry::RawGraph;
    pub use super::changelogs::{ChangelogsEmit, plan_changelogs};
    pub use super::events::{EventsEmit, plan_events};
    pub use super::java_hash::{Entry, JavaHashMap, spread_hash, table_size_for};
    pub use super::metadata::{AutoConfigEmit, hidden_folder_path};
    pub use super::scan::is_excluded_name;
    pub use super::write::SyncEmit;
}
