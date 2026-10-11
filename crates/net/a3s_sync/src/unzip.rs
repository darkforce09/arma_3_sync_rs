//! Uncompressing downloaded `.zip` files: [`unzip_file`] for one file, [`UnzipPool`] to run
//! them in the background while the download goes on, and [`extract_to_folder`] for whole
//! archives.
//!
//! A compressed leaf `<name>` is published as `<name>.zip` with one DEFLATE entry. Only the bytes
//! of the first entry are used, written to `<name>` (the entry name is ignored, as in Java), so an
//! archive cannot choose where it writes. Rust also checks what Java does not: the extracted size
//! and SHA-1 must match the sync leaf, directory and link entries are refused, and the old file
//! is replaced only once the new one is complete.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufReader, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_foundation::error::{ErrorKind, UserFacing};
use parking_lot::Mutex;
use sha1::{Digest as _, Sha1};

use crate::download::{DownloadedZip, ZIP_EXTENSION, ZipSink};
use crate::events::{EventError, EventSender, PercentCounter, Phase, SyncEvent};
use crate::plan::PART_EXTENSION;

/// Bytes extracted between two cancellation checks.
pub const ZIP_CHUNK: usize = 64 * 1024;

/// Mask and value of a symbolic link in a Unix mode (`S_IFMT`, `S_IFLNK`).
const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;

/// What [`unzip_file`] extracted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnzipStats {
    /// Bytes written to the target.
    pub bytes: u64,
}

/// Why an archive's content was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArchiveProblem {
    /// The archive has no entry, and the leaf is not an empty file.
    Empty,

    /// The first entry is a directory.
    Directory,

    /// The first entry is a symbolic link (never created).
    Symlink,

    /// The archive or its compressed stream is invalid or unsupported.
    Corrupt(String),

    /// The entry inflates to more than the leaf size (a zip bomb guard).
    TooLarge {
        /// The leaf size.
        expected: u64,
    },

    /// The entry inflates to fewer bytes than the leaf size.
    SizeMismatch {
        /// The leaf size.
        expected: u64,
        /// Bytes extracted.
        actual: u64,
    },

    /// The extracted content does not have the leaf's SHA-1.
    Sha1Mismatch {
        /// The leaf SHA-1.
        expected: String,
        /// SHA-1 of the extracted content.
        actual: String,
    },
}

impl std::fmt::Display for ArchiveProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("The archive has no entry"),
            Self::Directory => f.write_str("The archive entry is a directory"),
            Self::Symlink => f.write_str("The archive entry is a symbolic link"),
            Self::Corrupt(message) => f.write_str(message),
            Self::TooLarge { expected } => write!(
                f,
                "Uncompressed size exceeds the expected size of {expected} bytes"
            ),
            Self::SizeMismatch { expected, actual } => write!(
                f,
                "Uncompressed size {actual} differs from the expected size of {expected} bytes"
            ),
            Self::Sha1Mismatch { expected, actual } => {
                write!(f, "SHA-1 mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

/// Uncompressing one file failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UnzipError {
    /// The `.zip` file does not exist (or is not a regular file).
    // Mirrors dao/zip/ZipDAO.java:44-45
    #[error("File not found on disk: {}", path.display())]
    NotFound {
        /// Absolute path of the archive.
        path: PathBuf,
    },

    /// The archive's name does not end with `.zip`.
    // Mirrors dao/zip/ZipDAO.java:46-47
    #[error("Can not unzip file - bad file extension")]
    BadExtension {
        /// Absolute path of the archive.
        path: PathBuf,
    },

    /// The target is not the archive path without `.zip`; nothing was written.
    #[error("Failed to unzip file: {}\nUnexpected target: {}", path.display(), target.display())]
    UnsafeTarget {
        /// Absolute path of the archive.
        path: PathBuf,
        /// The refused target.
        target: PathBuf,
    },

    /// The archive content was refused.
    // Mirrors dao/zip/ZipDAO.java:78-84
    #[error("Failed to unzip file: {}\n{problem}", path.display())]
    Archive {
        /// Absolute path of the archive.
        path: PathBuf,
        /// What is wrong.
        problem: ArchiveProblem,
    },

    /// Writing the target failed.
    #[error("Failed to unzip file: {}\n{source}", path.display())]
    Io {
        /// Absolute path of the archive.
        path: PathBuf,
        /// The I/O error.
        #[source]
        source: io::Error,
    },

    /// Cancelled; the archive is kept for a later run.
    #[error("Cancelled")]
    Cancelled,
}

impl UnzipError {
    /// This error inside an event, for the leaf at `relative_path`.
    pub fn to_event_error(&self, relative_path: &str) -> EventError {
        EventError::new(self.kind(), self.to_string()).with_path(relative_path)
    }
}

impl UserFacing for UnzipError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFound { .. } => ErrorKind::NotFound,
            Self::BadExtension { .. } => ErrorKind::InvalidInput,
            Self::UnsafeTarget { .. } | Self::Archive { .. } => ErrorKind::InvalidData,
            Self::Io { source, .. } => ErrorKind::from_io(source),
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    /// The message already contains the cause.
    fn user_message(&self) -> String {
        self.to_string()
    }
}

/// Extracts the first entry of `zip.zip_path` to `zip.target_path`.
///
/// The content goes to `<target>.part`, is checked against `zip.size` and `zip.sha1`, synced
/// and renamed over the target, so the old target survives any failure. On success and on
/// failure the archive is deleted; on cancel (`cancel` is polled every [`ZIP_CHUNK`] bytes) it
/// is kept. Blocking.
// Mirrors dao/zip/ZipDAO.java:29-102
pub fn unzip_file(zip: &DownloadedZip, cancel: &AtomicBool) -> Result<UnzipStats, UnzipError> {
    let path = absolute(&zip.zip_path);
    tracing::info!("Uncompressing file: {}", path.display());
    if !std::fs::metadata(&zip.zip_path).is_ok_and(|m| m.is_file()) {
        return Err(UnzipError::NotFound { path });
    }
    let Some(expected_target) = target_of(&zip.zip_path) else {
        return Err(UnzipError::BadExtension { path });
    };
    if expected_target != zip.target_path {
        return Err(UnzipError::UnsafeTarget {
            path,
            target: zip.target_path.clone(),
        });
    }

    let part = append_to_name(&zip.target_path, PART_EXTENSION);
    let result = extract_first_entry(zip, &part, cancel);
    match result {
        Ok(bytes) => {
            remove_quietly(&zip.zip_path);
            Ok(UnzipStats { bytes })
        }
        Err(Failure::Cancelled) => {
            remove_quietly(&part);
            Err(UnzipError::Cancelled)
        }
        Err(failure) => {
            remove_quietly(&part);
            remove_quietly(&zip.zip_path);
            let error = match failure {
                Failure::Archive(problem) => UnzipError::Archive { path, problem },
                Failure::Io(source) => UnzipError::Io { path, source },
                Failure::Cancelled => UnzipError::Cancelled,
            };
            tracing::warn!("{error}");
            Err(error)
        }
    }
}

/// `zip_path` without its `.zip` suffix, or `None` when its name does not end with `.zip` or
/// nothing is left.
fn target_of(zip_path: &Path) -> Option<PathBuf> {
    let name = zip_path.file_name()?.to_str()?;
    let stem = name.strip_suffix(ZIP_EXTENSION)?;
    (!stem.is_empty()).then(|| zip_path.with_file_name(stem))
}

/// Why [`extract_first_entry`] stopped.
enum Failure {
    Archive(ArchiveProblem),
    Io(io::Error),
    Cancelled,
}

/// Writes the checked first entry of the archive to `part`, then renames it to the target.
/// Returns the bytes written.
fn extract_first_entry(
    zip: &DownloadedZip,
    part: &Path,
    cancel: &AtomicBool,
) -> Result<u64, Failure> {
    let corrupt =
        |err: &dyn std::fmt::Display| Failure::Archive(ArchiveProblem::Corrupt(err.to_string()));
    let file = File::open(&zip.zip_path).map_err(Failure::Io)?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| corrupt(&e))?;
    let mut out = File::create(part).map_err(Failure::Io)?;
    let mut hasher = Sha1::new();
    let mut total = 0u64;

    if archive.is_empty() {
        // Java writes an empty target: accept it only when the leaf is an empty file.
        let empty_leaf =
            zip.size == 0 && zip.sha1.as_deref() == Some(a3s_repo_files::hash::EMPTY_FILE_SHA1);
        if !empty_leaf {
            return Err(Failure::Archive(ArchiveProblem::Empty));
        }
    } else {
        if archive.len() > 1 {
            tracing::debug!(
                "{}: {} entries, only the first one is used",
                zip.zip_path.display(),
                archive.len()
            );
        }
        let entry = archive.by_index(0).map_err(|e| corrupt(&e))?;
        if entry.is_dir() {
            return Err(Failure::Archive(ArchiveProblem::Directory));
        }
        if entry.is_symlink() || entry.unix_mode().is_some_and(|m| m & S_IFMT == S_IFLNK) {
            return Err(Failure::Archive(ArchiveProblem::Symlink));
        }
        // One byte more than expected reveals an oversized entry without inflating it all.
        let mut limited = entry.take(zip.size.saturating_add(1));
        let mut buf = vec![0u8; ZIP_CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Cancelled);
            }
            let n = limited.read(&mut buf).map_err(|e| corrupt(&e))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > zip.size {
                return Err(Failure::Archive(ArchiveProblem::TooLarge {
                    expected: zip.size,
                }));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(Failure::Io)?;
        }
    }
    if total != zip.size {
        return Err(Failure::Archive(ArchiveProblem::SizeMismatch {
            expected: zip.size,
            actual: total,
        }));
    }
    if let Some(expected) = &zip.sha1 {
        let actual = if total == 0 {
            a3s_repo_files::hash::EMPTY_FILE_SHA1.to_owned()
        } else {
            a3s_repo_files::hash::to_hex(&hasher.finalize())
        };
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Failure::Archive(ArchiveProblem::Sha1Mismatch {
                expected: expected.clone(),
                actual,
            }));
        }
    }
    out.sync_all().map_err(Failure::Io)?;
    drop(out);
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled);
    }
    std::fs::rename(part, &zip.target_path).map_err(Failure::Io)?;
    Ok(total)
}

/// `path` with `suffix` appended to its file name.
fn append_to_name(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Removes a file, ignoring a missing one and logging other failures.
fn remove_quietly(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => tracing::warn!("Failed to delete {}: {err}", path.display()),
    }
}

/// Like Java `File.getAbsolutePath()`: no symlink resolution.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// What an [`UnzipPool`] did, returned by [`UnzipPool::finish`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnzipSummary {
    /// Files extracted successfully.
    pub extracted: usize,

    /// Failed files (cancellations excluded), in completion order.
    pub errors: Vec<EventError>,
}

/// Extracts submitted archives in the background, at most `workers` at a time.
///
/// Archives start in submission order on named threads (`unzip-{i}`) created on demand; a
/// thread ends when the queue is empty. Dropping the pool cancels it without waiting for its
/// threads.
// Mirrors dao/zip/UnZipFlowProcessor.java:24-163
pub struct UnzipPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    workers: usize,
    state: Mutex<PoolState>,
    /// Signalled whenever an extraction ends.
    changed: tokio::sync::Notify,
}

#[derive(Default)]
struct PoolState {
    queue: VecDeque<DownloadedZip>,
    /// Cancel flags of the running extractions.
    running: Vec<Arc<AtomicBool>>,
    threads: usize,
    next_thread_id: usize,
    cancelled: bool,
    /// Extractions ended (success, failure or cancel).
    ended: u64,
    extracted: usize,
    errors: Vec<EventError>,
}

impl PoolState {
    fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.running.is_empty()
    }
}

impl UnzipPool {
    /// A pool with one worker per available CPU.
    // Mirrors dao/zip/UnZipFlowProcessor.java:24-29
    pub fn new() -> Arc<Self> {
        Self::with_workers(std::thread::available_parallelism().map_or(1, std::num::NonZero::get))
    }

    /// A pool with at most `workers` (at least 1) concurrent extractions.
    pub fn with_workers(workers: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(PoolInner {
                workers: workers.max(1),
                state: Mutex::new(PoolState::default()),
                changed: tokio::sync::Notify::new(),
            }),
        })
    }

    /// Stops the running extractions (each within [`ZIP_CHUNK`] bytes) and drops the queue;
    /// queued archives stay on disk. Does not wait. Later submissions are ignored.
    // Mirrors dao/zip/UnZipFlowProcessor.java:52-58
    pub fn cancel(&self) {
        let mut state = self.inner.state.lock();
        state.cancelled = true;
        state.queue.clear();
        for flag in &state.running {
            flag.store(true, Ordering::Relaxed);
        }
        drop(state);
        self.inner.changed.notify_one();
    }

    /// Waits until every submitted archive was handled and returns the summary.
    ///
    /// When work is pending, emits `Phase(Uncompressing)`, then `Progress` over the archives
    /// queued or running at this moment. When the pool is already idle, emits nothing.
    // Mirrors dao/zip/UnZipFlowProcessor.java:37-50,108-130,
    // service/connection/ConnectionDownloadProcessor.java:176-179
    pub async fn finish(&self, events: &EventSender) -> UnzipSummary {
        let (baseline, pending) = {
            let state = self.inner.state.lock();
            (
                state.ended,
                (state.queue.len() + state.running.len()) as u64,
            )
        };
        let mut counter = None;
        if pending > 0 {
            events.emit(SyncEvent::Phase(Phase::Uncompressing));
            counter = Some((PercentCounter::new(pending), 0u64));
        }
        loop {
            let notified = self.inner.changed.notified();
            let (idle, ended) = {
                let state = self.inner.state.lock();
                (state.is_idle(), state.ended)
            };
            if let Some((counter, seen)) = counter.as_mut() {
                while *seen < ended.saturating_sub(baseline).min(pending) {
                    *seen += 1;
                    if let Some(percent) = counter.increment() {
                        events.emit(SyncEvent::Progress {
                            phase: Phase::Uncompressing,
                            percent,
                        });
                    }
                }
            }
            if idle {
                break;
            }
            notified.await;
        }
        let mut state = self.inner.state.lock();
        UnzipSummary {
            extracted: std::mem::take(&mut state.extracted),
            errors: std::mem::take(&mut state.errors),
        }
    }
}

impl Drop for UnzipPool {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl ZipSink for UnzipPool {
    // Mirrors dao/zip/UnZipFlowProcessor.java:31-35
    fn submit(&self, zip: DownloadedZip) {
        let mut state = self.inner.state.lock();
        if state.cancelled {
            return;
        }
        state.queue.push_back(zip);
        if state.threads
            < self
                .inner
                .workers
                .min(state.queue.len() + state.running.len())
        {
            let id = state.next_thread_id;
            state.next_thread_id += 1;
            state.threads += 1;
            drop(state);
            let inner = Arc::clone(&self.inner);
            let spawned = std::thread::Builder::new()
                .name(format!("unzip-{id}"))
                .spawn(move || worker(&inner));
            if let Err(err) = spawned {
                tracing::error!("Cannot start an unzip thread: {err}");
                let mut state = self.inner.state.lock();
                state.threads -= 1;
                if state.threads == 0 {
                    // Nothing will ever take the queue: report it instead of hanging `finish`.
                    while let Some(zip) = state.queue.pop_front() {
                        state.ended += 1;
                        state.errors.push(
                            EventError::new(
                                ErrorKind::Internal,
                                format!("Cannot start an unzip thread: {err}"),
                            )
                            .with_path(zip.relative_path),
                        );
                    }
                    drop(state);
                    self.inner.changed.notify_one();
                }
            }
        }
    }

    // Mirrors dao/zip/UnZipFlowProcessor.java:132-142
    fn is_idle(&self) -> bool {
        self.inner.state.lock().is_idle()
    }
}

/// Takes archives from the queue until it is empty.
// Mirrors dao/zip/UnZipFlowProcessor.java:60-87
fn worker(inner: &PoolInner) {
    loop {
        let (zip, flag) = {
            let mut state = inner.state.lock();
            let Some(zip) = state.queue.pop_front() else {
                state.threads -= 1;
                return;
            };
            let flag = Arc::new(AtomicBool::new(false));
            state.running.push(Arc::clone(&flag));
            (zip, flag)
        };
        let result = unzip_file(&zip, &flag);
        let mut state = inner.state.lock();
        state.running.retain(|f| !Arc::ptr_eq(f, &flag));
        state.ended += 1;
        match result {
            Ok(_) => state.extracted += 1,
            Err(UnzipError::Cancelled) => {}
            Err(err) => state.errors.push(err.to_event_error(&zip.relative_path)),
        }
        drop(state);
        inner.changed.notify_one();
    }
}

/// Extracting a whole archive failed.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ExtractError {
    /// What failed.
    pub message: String,

    /// The I/O error, if any.
    #[source]
    pub source: Option<io::Error>,
}

impl ExtractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    fn io(message: impl Into<String>, source: io::Error) -> Self {
        Self {
            message: message.into(),
            source: Some(source),
        }
    }
}

/// Extracts every entry of `zip` below `folder` (created if missing), overwriting existing
/// files.
///
/// An entry whose name is empty, absolute, has a drive prefix, a `\`, a `..` component, or is a
/// symbolic link is refused before anything of it is written: nothing is ever written outside
/// `folder`.
// Mirrors dao/FileAccessMethods.java:109-161
pub fn extract_to_folder(zip: &Path, folder: &Path) -> Result<(), ExtractError> {
    if !folder.exists() {
        std::fs::create_dir_all(folder).map_err(|e| {
            ExtractError::io(
                format!("Unable to create extraction folder: {}", folder.display()),
                e,
            )
        })?;
    }
    let root = folder.canonicalize().map_err(|e| {
        ExtractError::io(
            format!("Unable to create extraction folder: {}", folder.display()),
            e,
        )
    })?;
    if !root.is_dir() {
        return Err(ExtractError::new(format!(
            "Extraction target is not a directory: {}",
            root.display()
        )));
    }
    let file = File::open(zip)
        .map_err(|e| ExtractError::io(format!("File not found on disk: {}", zip.display()), e))?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| ExtractError::new(format!("Failed to unzip file: {}\n{e}", zip.display())))?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| {
            ExtractError::new(format!("Failed to unzip file: {}\n{e}", zip.display()))
        })?;
        let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
        if name.is_empty() {
            return Err(ExtractError::new("ZIP entry has no name."));
        }
        let unsafe_entry = || ExtractError::new(format!("Unsafe ZIP entry: {name}"));
        if entry.is_symlink() || entry.unix_mode().is_some_and(|m| m & S_IFMT == S_IFLNK) {
            return Err(unsafe_entry());
        }
        let relative = safe_entry_path(&name).ok_or_else(unsafe_entry)?;
        if entry.enclosed_name().is_none() {
            return Err(unsafe_entry());
        }
        let path = root.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| {
                ExtractError::io(
                    format!("Unable to create extracted directory: {}", path.display()),
                    e,
                )
            })?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ExtractError::io(
                    format!(
                        "Unable to create extracted file parent: {}",
                        parent.display()
                    ),
                    e,
                )
            })?;
        }
        let written = File::create(&path).and_then(|mut out| {
            io::copy(&mut entry, &mut out)?;
            out.flush()
        });
        if let Err(err) = written {
            remove_quietly(&path);
            return Err(ExtractError::io(
                format!("Failed to unzip file: {}\n{err}", zip.display()),
                err,
            ));
        }
    }
    Ok(())
}

/// The entry name as a relative path of plain components, or `None` if it could leave the
/// extraction folder (absolute, drive prefix, `..`, `\`) or names the folder itself.
fn safe_entry_path(name: &str) -> Option<PathBuf> {
    if name.contains(['\\', '\0']) || name.starts_with('/') {
        return None;
    }
    let mut out = PathBuf::new();
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        // A drive prefix (`C:`) or a `..` anywhere is refused, even when it would stay inside.
        if segment == ".." || segment.contains(':') {
            return None;
        }
        let mut components = Path::new(segment).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => out.push(segment),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}
