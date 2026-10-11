//! Uploading a built repository to its server.
//!
//! [`upload_repository`] reads the five local `.a3s` files, fetches the remote sync tree over the
//! upload protocol and plans which nodes to check, upload or delete ([`plan_upload`]). Then, on
//! one upload session:
//!
//! 1. the remote files of unchanged nodes are checked for existence; missing ones are
//!    re-uploaded;
//! 2. new and changed files are uploaded (sequentially, or for FTP and SFTP on up to
//!    [`Repository::parallel_upload_connections`] connections, directories first);
//! 3. the metadata is uploaded, so clients never see new metadata before the files it lists.
//!    It is the Java round trip of the local files ([`java_roundtrip_bytes`]), uploaded in the
//!    order `sync`, `serverinfo`, `changelogs`, `autoconfig`, `events`, and not interrupted by a
//!    cancel once started;
//! 4. remote nodes absent from the local tree are deleted.
//!
//! Differences from Java (none visible to clients): resume is keyed by path and the local
//! `.a3s/sync` ([`UploadResume`]); parallel workers share one queue and the first failure stops
//! them all; a folder named `racine` below the root is uploaded to its real path.

#![allow(
    clippy::result_large_err,
    reason = "the errors carry transport errors; they are returned once per operation"
)]

mod plan;
mod resume;
mod roundtrip;
mod transfer;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_repo_files::A3sFile;
use a3s_repo_files::layout::A3S_FOLDER;
use a3s_sync::{EventSender, Outcome, Phase, RemoteConnection, RemoteError, RemoteFile, SyncEvent};
use a3s_transport::{CancellationToken, RemoteKind, TransferCtx, Transport, TransportOptions};
use a3s_types::local::Repository;
use a3s_types::local::protocol::host_url;
use a3s_types::{JavaCodec as _, Protocol, ProtocolType, SyncTreeDirectory};
use parking_lot::Mutex;
use sha1::{Digest as _, Sha1};

pub use plan::{PlannedNode, UploadPlan, plan_upload};
pub use resume::UploadResume;
pub use roundtrip::{
    RoundTripError, RoundTripOverrides, hash_set_roundtrip_capacity, java_roundtrip,
    java_roundtrip_bytes,
};

pub(crate) use roundtrip::distinct_boolean;

use self::transfer::Meter;
use crate::build::carry::RawGraph;
use crate::check::{
    CheckError, check_remote_files, event_error, expected_remote_files, has_zsync, transport_kind,
};

// Mirrors dao/A3SFilesAccessor.java:26
const FILE_CORRUPTED: &str = "The file seems to be corrupted.";

/// Counts of a successful upload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadReport {
    /// Remote files of unchanged nodes checked for existence.
    pub checked: usize,

    /// Checked files that were missing (and were uploaded).
    pub missing: usize,

    /// Files uploaded.
    pub uploaded_files: usize,

    /// Bytes of the uploaded files (metadata excluded).
    pub uploaded_bytes: u64,

    /// Remote directories created.
    pub created_directories: usize,

    /// Entries skipped because a previous attempt uploaded them.
    pub skipped_by_resume: usize,

    /// Remote files and directories deleted.
    pub deleted: usize,

    /// The five `.a3s` files were uploaded (always, on success).
    pub metadata_uploaded: bool,
}

/// Why an upload failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UploadError {
    /// The repository has no upload protocol.
    // Mirrors service/administration/RepositoryUploadProcessor.java:72-78
    #[error("Please use the upload options to configure a connection.")]
    UploadProtocolNotSet,

    /// The repository has no main folder.
    // Mirrors ui/repository/AdminPanel.java:718-725
    #[error("Please set the repository main folder location then build the repository.")]
    PathNotSet,

    /// A local `.a3s` file is missing or unreadable.
    // Mirrors dao/A3SFilesAccessor.java:55-74
    #[error("{}", read_local_message(*file, source))]
    ReadLocal {
        /// The metadata file.
        file: A3sFile,
        /// Its local path.
        path: PathBuf,
        /// The read or decode error.
        #[source]
        source: a3s_repo_files::Error,
    },

    /// A local `.a3s` file holds content Java would not upload unchanged.
    #[error("Failed to read file: {}\n{reason}", file.file_name())]
    RoundTrip {
        /// The metadata file.
        file: A3sFile,
        /// What is wrong.
        reason: String,
    },

    /// A local file to upload does not exist.
    // Mirrors service/connection/ConnectionUploadProcessor.java:96-108
    #[error("File not found: {}", path.display())]
    LocalFileMissing {
        /// The absolute local path.
        path: PathBuf,
    },

    /// Connecting or fetching the remote sync tree failed.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// Checking the remote files failed.
    #[error(transparent)]
    Check(CheckError),

    /// A remote operation failed.
    #[error("{source}")]
    Transport {
        /// Path below the repository root.
        relative_path: String,
        /// The transport error.
        #[source]
        source: a3s_transport::Error,
    },

    /// The upload was cancelled.
    #[error("Cancelled")]
    Cancelled,
}

fn read_local_message(file: A3sFile, source: &a3s_repo_files::Error) -> String {
    let mut message = format!("Failed to read file: {}\n{source}", file.file_name());
    if matches!(
        source,
        a3s_repo_files::Error::Gzip(_) | a3s_repo_files::Error::TooLarge(_)
    ) {
        message.push('\n');
        message.push_str(FILE_CORRUPTED);
    }
    message
}

impl UploadError {
    /// Whether the connection to the server was lost (the caller may rerun the upload).
    // Mirrors service/administration/RepositoryUploadProcessor.java:157-160
    pub fn is_connection_lost(&self) -> bool {
        match self {
            Self::Remote(err) => err.is_connection_lost(),
            Self::Check(err) => err.is_connection_lost(),
            Self::Transport { source, .. } => source.is_connection_lost(),
            _ => false,
        }
    }

    /// Classification of the error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::UploadProtocolNotSet | Self::PathNotSet => ErrorKind::InvalidInput,
            Self::ReadLocal { source, .. } => match source {
                a3s_repo_files::Error::Io { source, .. } => ErrorKind::from_io(source),
                _ => ErrorKind::InvalidData,
            },
            Self::RoundTrip { .. } => ErrorKind::InvalidData,
            Self::LocalFileMissing { .. } => ErrorKind::NotFound,
            Self::Remote(err) => err.kind(),
            Self::Check(err) => err.kind(),
            Self::Transport { source, .. } => transport_kind(source),
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    fn transport(relative_path: &str, source: a3s_transport::Error) -> Self {
        if matches!(source, a3s_transport::Error::Cancelled) {
            Self::Cancelled
        } else {
            Self::Transport {
                relative_path: relative_path.to_owned(),
                source,
            }
        }
    }

    fn remote(err: RemoteError) -> Self {
        if matches!(err, RemoteError::Cancelled) {
            Self::Cancelled
        } else {
            Self::Remote(err)
        }
    }
}

impl UserFacing for UploadError {
    fn kind(&self) -> ErrorKind {
        Self::kind(self)
    }

    fn user_message(&self) -> String {
        self.to_string()
    }
}

/// Uploads the repository built at `repository.path` to `repository.upload_protocol`.
///
/// `options` are the transport options of the upload protocol. `resume` carries the files
/// uploaded by earlier, failed or cancelled attempts; it is cleared on success.
///
/// Emits the phases `CheckingRemoteFiles`, `Uploading` and `DeletingRemoteFiles`, check and
/// delete `Progress`, `TransferTotal`, `Transfer`, `Speed`, `FileStarted`, `FileDone`,
/// `LastIndexTransferred`, `ConnectionLost` (before `Finished` when the connection was lost)
/// and `Finished`.
// Mirrors service/administration/RepositoryUploadProcessor.java:61-170
pub async fn upload_repository(
    repository: &Repository,
    options: &TransportOptions,
    resume: &mut UploadResume,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<UploadReport, UploadError> {
    let started = Instant::now();
    let result = run_upload(repository, options, resume, ctx, events).await;
    match &result {
        Ok(_) => {
            resume.clear();
            tracing::info!(
                "Repository upload finished successfully in {:.3} s.",
                started.elapsed().as_secs_f64()
            );
            events.emit(SyncEvent::Finished(Outcome::Success));
        }
        Err(UploadError::Cancelled) => events.emit(SyncEvent::Finished(Outcome::Cancelled)),
        Err(err) => {
            if err.is_connection_lost() {
                events.emit(SyncEvent::ConnectionLost);
            }
            events.emit(SyncEvent::Finished(Outcome::Failed {
                error: event_error(err),
            }));
        }
    }
    result
}

/// The local metadata: the five files as Java would upload them, and the sync tree.
struct LocalMetadata {
    uploads: Vec<(A3sFile, Vec<u8>)>,
    sync: SyncTreeDirectory,
    sync_digest: [u8; 20],
}

/// Reads `<root>/.a3s/*` in Java's order and encodes their uploads.
// Mirrors service/RepositoryService.java:1394-1419, dao/repository/RepositoryDAO.java:242-288
fn read_local_metadata(
    root: &Path,
    compressed_pbo_files_only: bool,
) -> Result<LocalMetadata, UploadError> {
    let folder = root.join(A3S_FOLDER);
    let mut graphs = Vec::with_capacity(A3sFile::ALL.len());
    let mut sync_digest = [0; 20];
    for file in A3sFile::ALL {
        let path = folder.join(file.file_name());
        let read_error = |source| UploadError::ReadLocal {
            file,
            path: path.clone(),
            source,
        };
        let bytes = std::fs::read(&path).map_err(|source| {
            read_error(a3s_repo_files::Error::Io {
                path: path.clone(),
                source,
            })
        })?;
        if file == A3sFile::Sync {
            sync_digest = Sha1::digest(&bytes).into();
        }
        let graph: RawGraph = a3s_repo_files::decode_a3s(&bytes).map_err(read_error)?;
        graphs.push((file, graph));
    }
    let (_, sync_graph) = &graphs[0];
    let sync = SyncTreeDirectory::decode(&sync_graph.graph, sync_graph.root).map_err(|source| {
        UploadError::ReadLocal {
            file: A3sFile::Sync,
            path: folder.join(A3sFile::Sync.file_name()),
            source: source.into(),
        }
    })?;
    let mut uploads = Vec::with_capacity(graphs.len());
    for (file, graph) in &graphs {
        let overrides = RoundTripOverrides {
            compressed_pbo_files_only: (*file == A3sFile::ServerInfo)
                .then_some(compressed_pbo_files_only),
        };
        uploads.push((
            *file,
            java_roundtrip_bytes(*file, &graph.graph, &overrides)?,
        ));
    }
    Ok(LocalMetadata {
        uploads,
        sync,
        sync_digest,
    })
}

/// One remote file or directory to upload.
struct UploadEntry {
    remote: RemoteFile,

    /// The local file; `None` for directories.
    local: Option<PathBuf>,
    len: u64,
}

impl UploadEntry {
    fn path(&self) -> String {
        self.remote.transport_path()
    }
}

async fn run_upload(
    repository: &Repository,
    options: &TransportOptions,
    resume: &mut UploadResume,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<UploadReport, UploadError> {
    let upload_protocol = repository
        .upload_protocol
        .as_ref()
        .ok_or(UploadError::UploadProtocolNotSet)?;
    let root = match repository.path.as_deref() {
        Some(path) if !path.is_empty() => Path::new(path),
        _ => return Err(UploadError::PathNotSet),
    };
    let local = read_local_metadata(root, repository.upload_compressed_pbo_files_only)?;

    // Mirrors service/administration/RepositoryUploadProcessor.java:81-84
    let parallel = matches!(
        upload_protocol.protocol_type,
        ProtocolType::Ftp | ProtocolType::Sftp
    );
    let connections = if parallel {
        usize::try_from(repository.parallel_upload_connections())
            .unwrap_or(1)
            .max(1)
    } else {
        1
    };
    tracing::info!(
        "Uploading repository: {} on url: {}",
        repository.name,
        host_url(upload_protocol)
    );

    let mut connection = RemoteConnection::connect(upload_protocol, options, ctx)
        .await
        .map_err(UploadError::remote)?;
    if !connection.transport_mut().supports_upload() {
        let source = a3s_transport::Error::Unsupported {
            protocol: connection.transport_mut().protocol_name(),
            operation: "upload",
        };
        return Err(UploadError::Transport {
            relative_path: String::new(),
            source,
        });
    }
    // Mirrors service/ConnectionService.java:378-392 and RepositoryUploadProcessor.java:118-124
    let remote_sync = match connection
        .fetch_a3s::<SyncTreeDirectory>(A3sFile::Sync, ctx)
        .await
    {
        Ok((sync, _)) => Some(sync),
        Err(err) if err.is_not_found() => None,
        Err(err) => {
            connection.close().await;
            return Err(UploadError::remote(err));
        }
    };
    let plan = plan_upload(&local.sync, remote_sync.as_ref());
    tracing::info!(
        "Upload plan prepared: check={}, upload={}, delete={}",
        plan.to_check.len(),
        plan.to_upload.len(),
        plan.to_delete.len()
    );
    resume.begin(local.sync_digest);

    let mut transport = connection.into_transport();
    let pipeline = Pipeline {
        repository,
        upload_protocol,
        options,
        root,
        connections,
        ctx,
        events,
    };
    let result = {
        let mut session = transport.upload_session();
        pipeline
            .run(&mut session, &plan, &local.uploads, resume)
            .await
    };
    transport.close().await;
    result
}

/// The phases after planning, on connection 0's upload session.
struct Pipeline<'a> {
    repository: &'a Repository,
    upload_protocol: &'a Protocol,
    options: &'a TransportOptions,
    root: &'a Path,
    connections: usize,
    ctx: &'a TransferCtx,
    events: &'a EventSender,
}

impl Pipeline<'_> {
    // Mirrors service/ConnectionService.java:394-451
    async fn run(
        &self,
        transport: &mut Transport,
        plan: &UploadPlan,
        metadata: &[(A3sFile, Vec<u8>)],
        resume: &mut UploadResume,
    ) -> Result<UploadReport, UploadError> {
        let with_zsync = has_zsync(&self.repository.protocol);
        let only_zip = self.repository.upload_compressed_pbo_files_only;
        let mut report = UploadReport::default();

        // Check.
        self.events
            .emit(SyncEvent::Phase(Phase::CheckingRemoteFiles));
        let check_files: Vec<RemoteFile> =
            expected_remote_files(&plan.to_check, only_zip, with_zsync)
                .into_iter()
                .filter(|f| !f.is_directory)
                .collect();
        let missing = check_remote_files(
            transport,
            None,
            1,
            &check_files,
            self.ctx,
            self.events,
            false,
        )
        .await
        .map_err(|err| match err {
            CheckError::Cancelled => UploadError::Cancelled,
            other => UploadError::Check(other),
        })?;
        report.checked = check_files.len();
        report.missing = missing.len();

        // Upload.
        self.events.emit(SyncEvent::Phase(Phase::Uploading));
        let mut remote_files = expected_remote_files(&plan.to_upload, only_zip, with_zsync);
        remote_files.extend(missing.into_iter().map(|i| check_files[i].clone()));
        let mut entries = Vec::with_capacity(remote_files.len());
        for remote in remote_files {
            let local = (!remote.is_directory).then(|| self.root.join(remote.transport_path()));
            if resume.is_done(&remote.transport_path(), local.as_deref()) {
                report.skipped_by_resume += 1;
                continue;
            }
            entries.push(UploadEntry {
                remote,
                local,
                len: 0,
            });
        }
        // Mirrors service/connection/ConnectionUploadProcessor.java:95-112
        let mut total = 0u64;
        let mut files = 0u32;
        for entry in &mut entries {
            let Some(local) = &entry.local else {
                continue;
            };
            let metadata = std::fs::metadata(local)
                .ok()
                .filter(std::fs::Metadata::is_file)
                .ok_or_else(|| UploadError::LocalFileMissing {
                    path: std::path::absolute(local).unwrap_or_else(|_err| local.clone()),
                })?;
            entry.len = metadata.len();
            total += entry.len;
            files += 1;
        }
        self.events.emit(SyncEvent::TransferTotal {
            bytes: total,
            files,
        });
        let workers = if self.connections > 1 {
            self.connections
                .min(entries.iter().filter(|e| e.local.is_some()).count())
                .max(1)
        } else {
            1
        };
        let phase = FilePhase {
            pipeline: self,
            meter: Meter::new(total, workers, self.events),
            resume: Mutex::new(resume),
            uploaded_files: AtomicUsize::new(0),
            uploaded_bytes: AtomicU64::new(0),
            created_directories: AtomicUsize::new(0),
        };
        let uploaded = phase
            .meter
            .drive(phase.upload(transport, &entries, workers))
            .await;
        report.uploaded_files = phase.uploaded_files.load(Ordering::Relaxed);
        report.uploaded_bytes = phase.uploaded_bytes.load(Ordering::Relaxed);
        report.created_directories = phase.created_directories.load(Ordering::Relaxed);
        uploaded?;
        if self.ctx.cancel.is_cancelled() {
            return Err(UploadError::Cancelled);
        }

        // Metadata: a fresh context, so a cancel cannot leave a half-written set.
        // Mirrors service/connection/ConnectionUploadProcessor.java:89-93, 214-227
        let metadata_ctx = TransferCtx::new();
        for (file, bytes) in metadata {
            let path = file.relative_path();
            transport
                .upload_bytes(bytes, &path, &metadata_ctx)
                .await
                .map_err(|source| UploadError::transport(&path, source))?;
        }
        report.metadata_uploaded = true;

        // Delete.
        // Mirrors service/connection/ConnectionDeleteProcessor.java:28-47
        if self.ctx.cancel.is_cancelled() {
            return Err(UploadError::Cancelled);
        }
        self.events
            .emit(SyncEvent::Phase(Phase::DeletingRemoteFiles));
        let deletions = expected_remote_files(&plan.to_delete, false, with_zsync);
        let total = deletions.len();
        for (index, remote) in deletions.iter().enumerate() {
            if self.ctx.cancel.is_cancelled() {
                return Err(UploadError::Cancelled);
            }
            let path = remote.transport_path();
            let kind = if remote.is_directory {
                RemoteKind::Directory
            } else {
                RemoteKind::File
            };
            transport
                .delete(&path, kind, self.ctx)
                .await
                .map_err(|source| UploadError::transport(&path, source))?;
            report.deleted += 1;
            self.events.emit(SyncEvent::Progress {
                phase: Phase::DeletingRemoteFiles,
                percent: u8::try_from((index + 1) * 100 / total).unwrap_or(100),
            });
        }
        Ok(report)
    }
}

/// The file upload phase.
struct FilePhase<'a> {
    pipeline: &'a Pipeline<'a>,
    meter: Meter,
    resume: Mutex<&'a mut UploadResume>,
    uploaded_files: AtomicUsize,
    uploaded_bytes: AtomicU64,
    created_directories: AtomicUsize,
}

/// Where a parallel worker's connection comes from.
enum WorkerConnection<'a> {
    /// Connection 0.
    Borrowed(&'a mut Transport),

    /// A new connection with its own upload session.
    Lazy,
}

/// Shared state of the parallel workers.
struct WorkQueue<'a> {
    files: Vec<&'a UploadEntry>,
    next: AtomicUsize,
    stop: CancellationToken,
    first_error: Mutex<Option<UploadError>>,
}

impl FilePhase<'_> {
    // Mirrors service/connection/ConnectionUploadProcessor.java:114-195
    async fn upload(
        &self,
        transport: &mut Transport,
        entries: &[UploadEntry],
        workers: usize,
    ) -> Result<(), UploadError> {
        let ctx = self.pipeline.ctx;
        if workers <= 1 && self.pipeline.connections <= 1 {
            let worker_ctx = self.meter.worker_ctx(0, ctx.cancel.clone());
            for entry in entries {
                if ctx.cancel.is_cancelled() {
                    return Err(UploadError::Cancelled);
                }
                self.upload_entry(transport, entry, 0, &worker_ctx).await?;
            }
            return Ok(());
        }

        // Directories first, on connection 0.
        let dir_ctx = self.meter.worker_ctx(0, ctx.cancel.clone());
        for entry in entries.iter().filter(|e| e.local.is_none()) {
            if ctx.cancel.is_cancelled() {
                return Err(UploadError::Cancelled);
            }
            self.upload_entry(transport, entry, 0, &dir_ctx).await?;
        }
        let queue = WorkQueue {
            files: entries.iter().filter(|e| e.local.is_some()).collect(),
            next: AtomicUsize::new(0),
            stop: ctx.cancel.child_token(),
            first_error: Mutex::new(None),
        };
        if queue.files.is_empty() || ctx.cancel.is_cancelled() {
            return Ok(());
        }
        let mut slots = vec![WorkerConnection::Borrowed(transport)];
        slots.extend((1..workers).map(|_| WorkerConnection::Lazy));
        futures::future::join_all(
            slots
                .into_iter()
                .enumerate()
                .map(|(worker, slot)| self.worker(worker, slot, &queue)),
        )
        .await;
        if ctx.cancel.is_cancelled() {
            return Err(UploadError::Cancelled);
        }
        queue.first_error.into_inner().map_or(Ok(()), Err)
    }

    async fn worker(&self, worker: usize, slot: WorkerConnection<'_>, queue: &WorkQueue<'_>) {
        let ctx = self.meter.worker_ctx(worker, queue.stop.clone());
        let result = match slot {
            WorkerConnection::Borrowed(transport) => {
                self.work(transport, worker, queue, &ctx).await
            }
            WorkerConnection::Lazy => {
                let pipeline = self.pipeline;
                match RemoteConnection::connect(pipeline.upload_protocol, pipeline.options, &ctx)
                    .await
                {
                    Ok(connection) => {
                        let mut transport = connection.into_transport();
                        let result = {
                            let mut session = transport.upload_session();
                            self.work(&mut session, worker, queue, &ctx).await
                        };
                        transport.close().await;
                        result
                    }
                    Err(err) => Err(UploadError::remote(err)),
                }
            }
        };
        if let Err(err) = result
            && !queue.stop.is_cancelled()
            && !matches!(err, UploadError::Cancelled)
        {
            let mut first = queue.first_error.lock();
            if first.is_none() {
                *first = Some(err);
            }
            queue.stop.cancel();
        }
    }

    async fn work(
        &self,
        transport: &mut Transport,
        worker: usize,
        queue: &WorkQueue<'_>,
        ctx: &TransferCtx,
    ) -> Result<(), UploadError> {
        while !queue.stop.is_cancelled() {
            let index = queue.next.fetch_add(1, Ordering::Relaxed);
            let Some(entry) = queue.files.get(index) else {
                break;
            };
            self.upload_entry(transport, entry, worker, ctx).await?;
        }
        Ok(())
    }

    /// Creates a directory or uploads a file, then records it.
    // Mirrors service/connection/ConnectionUploadProcessor.java:114-140 and
    // dao/connection/AbstractConnexionDAO.java:393-425
    async fn upload_entry(
        &self,
        transport: &mut Transport,
        entry: &UploadEntry,
        worker: usize,
        ctx: &TransferCtx,
    ) -> Result<(), UploadError> {
        let events = self.pipeline.events;
        let path = entry.path();
        events.emit(SyncEvent::FileStarted {
            relative_path: path.clone(),
        });
        let Some(local) = &entry.local else {
            transport
                .mkdirs(&path, ctx)
                .await
                .map_err(|source| UploadError::transport(&path, source))?;
            self.resume.lock().record(&path, None);
            self.created_directories.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        self.meter.start_file(worker, &path, entry.len);
        if let Err(source) = transport.upload_file(local, &path, ctx).await {
            self.meter.abandon_file(worker);
            return Err(UploadError::transport(&path, source));
        }
        self.uploaded_files.fetch_add(1, Ordering::Relaxed);
        self.uploaded_bytes.fetch_add(entry.len, Ordering::Relaxed);
        events.emit(SyncEvent::FileDone {
            relative_path: path.clone(),
        });
        let completed = {
            let mut resume = self.resume.lock();
            resume.record(&path, Some(local));
            resume.completed()
        };
        events.emit(SyncEvent::LastIndexTransferred {
            index: u32::try_from(completed).unwrap_or(u32::MAX),
        });
        self.meter.finish_file(worker, entry.len);
        Ok(())
    }
}
