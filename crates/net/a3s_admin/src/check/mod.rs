//! Repository content check: does the server hold every file the remote sync tree lists?
//!
//! [`check_repository_content`] fetches the remote metadata over the download protocol, expands
//! the sync tree into the expected remote files ([`expected_remote_files`]) and tests each with
//! [`Transport::exists`] on up to [`Repository::repository_check_connections`] connections.
//! Each connection keeps an upload session, so FTP answers from cached directory listings.

mod expected;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_repo_files::A3sFile;
use a3s_sync::{
    EventError, EventSender, Outcome, PercentCounter, Phase, RemoteConnection, RemoteError,
    RemoteFile, RemoteMetadata, SyncEvent,
};
use a3s_transport::{CancellationToken, TransferCtx, Transport, TransportOptions};
use a3s_types::local::Repository;
use a3s_types::local::protocol::host_url;
use a3s_types::{AutoConfig, Changelogs, Protocol, ServerInfo, SyncTreeDirectory};
use parking_lot::Mutex;

pub(crate) use expected::expected_files;
pub use expected::{expected_remote_files, has_zsync};

use crate::upload::{RacineNodes, tree_nodes};

/// Prefix of the message of a missing file.
// Mirrors service/connection/ConnectionCheckProcessor.java:116-117
const FILE_NOT_FOUND: &str = "File not found on repository: ";

/// Result of a content check.
#[derive(Clone, Debug, Default)]
pub struct ContentCheckReport {
    /// `sync`, `server_info`, `changelogs` and `auto_config` as fetched (`events` is `None`).
    pub metadata: RemoteMetadata,

    /// Number of remote files tested.
    pub checked: usize,

    /// Missing remote files, in plan order.
    pub missing: Vec<RemoteFile>,
}

impl ContentCheckReport {
    /// Whether every expected file exists.
    pub fn is_synchronized(&self) -> bool {
        self.missing.is_empty()
    }

    /// One message per missing file: `File not found on repository: /path`.
    pub fn error_messages(&self) -> Vec<String> {
        self.missing.iter().map(missing_message).collect()
    }

    /// The console's end-of-check lines.
    // Mirrors console/CommandGeneral.java:178-189
    pub fn console_lines(&self, repository_name: &str) -> Vec<String> {
        if self.is_synchronized() {
            return vec![format!(
                "Repository {repository_name} - repository is synchronized."
            )];
        }
        let mut lines = vec![format!(
            "Repository {repository_name} - repository is not synchronized."
        )];
        lines.extend(self.error_messages());
        lines
    }
}

fn missing_message(file: &RemoteFile) -> String {
    format!("{FILE_NOT_FOUND}{}", file.relative_file_path())
}

/// Why a content check failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CheckError {
    /// Connecting or fetching the remote metadata failed.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// Testing a remote file failed.
    #[error("{source}")]
    Exists {
        /// The file, `/`-prefixed like in messages.
        relative_path: String,
        /// The transport error.
        #[source]
        source: a3s_transport::Error,
    },

    /// The check was cancelled.
    #[error("Cancelled")]
    Cancelled,
}

impl CheckError {
    /// Whether a new connection may help.
    pub fn is_connection_lost(&self) -> bool {
        match self {
            Self::Remote(err) => err.is_connection_lost(),
            Self::Exists { source, .. } => source.is_connection_lost(),
            Self::Cancelled => false,
        }
    }

    /// Classification of the error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Remote(err) => err.kind(),
            Self::Exists { source, .. } => transport_kind(source),
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    fn from_remote(err: RemoteError) -> Self {
        if matches!(err, RemoteError::Cancelled) {
            Self::Cancelled
        } else {
            Self::Remote(err)
        }
    }
}

impl UserFacing for CheckError {
    fn kind(&self) -> ErrorKind {
        Self::kind(self)
    }

    fn user_message(&self) -> String {
        self.to_string()
    }
}

/// Classification of a transport error.
pub(crate) fn transport_kind(err: &a3s_transport::Error) -> ErrorKind {
    use a3s_transport::Error as E;
    match err {
        E::NotFound(_) => ErrorKind::NotFound,
        E::Unauthorized => ErrorKind::Unauthorized,
        E::Timeout => ErrorKind::Timeout,
        E::Cancelled => ErrorKind::Cancelled,
        E::Incomplete { .. } => ErrorKind::IncompleteTransfer,
        E::InvalidProtocol(_) | E::InvalidPath(_) => ErrorKind::InvalidInput,
        E::InsecureTlsRefused(_) | E::Tls(_) => ErrorKind::TlsValidation,
        E::Unsupported { .. } => ErrorKind::Unsupported,
        E::Io(e) => ErrorKind::from_io(e),
        other if other.is_connection_lost() => ErrorKind::ConnectionFailed,
        _ => ErrorKind::Io,
    }
}

/// The event projection of a failure.
pub(crate) fn event_error(err: &(impl UserFacing + ?Sized)) -> EventError {
    EventError::new(err.kind(), err.user_message())
}

/// Checks that the server holds every file of the remote sync tree (the admin "Check" button).
///
/// Emits `Phase(CheckingRemoteFiles)`, `Progress` after each file (values reported once),
/// `FileFailed` per missing file and `Finished`: `Success`, `CompletedWithErrors` (one error
/// per missing file), `Failed` or `Cancelled`.
// Mirrors service/administration/RepositoryCheckProcessor.java:35-90 and
// service/ConnectionService.java:349-374
pub async fn check_repository_content(
    repository: &Repository,
    options: &TransportOptions,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<ContentCheckReport, CheckError> {
    let result = run_check(repository, options, ctx, events).await;
    let outcome = match &result {
        Ok(report) if report.is_synchronized() => Outcome::Success,
        Ok(report) => Outcome::CompletedWithErrors {
            errors: report
                .missing
                .iter()
                .map(|file| {
                    EventError::new(ErrorKind::NotFound, missing_message(file))
                        .with_path(file.transport_path())
                })
                .collect(),
        },
        Err(CheckError::Cancelled) => Outcome::Cancelled,
        Err(err) => Outcome::Failed {
            error: event_error(err),
        },
    };
    events.emit(SyncEvent::Finished(outcome));
    result
}

async fn run_check(
    repository: &Repository,
    options: &TransportOptions,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<ContentCheckReport, CheckError> {
    let protocol = &repository.protocol;
    tracing::info!(
        "Checking repository content: {} on url: {}",
        repository.name,
        host_url(protocol)
    );
    let mut connection = RemoteConnection::connect(protocol, options, ctx)
        .await
        .map_err(CheckError::from_remote)?;
    let mut metadata = RemoteMetadata::default();
    let fetched = fetch_check_metadata(&mut connection, &mut metadata, ctx).await;
    let sync = match fetched {
        Ok(sync) => sync,
        Err(err) => {
            connection.close().await;
            return Err(CheckError::from_remote(err));
        }
    };
    let compressed_pbo_files_only = metadata
        .server_info
        .as_ref()
        .is_some_and(|s| s.compressed_pbo_files_only);
    let nodes = tree_nodes(&sync, RacineNodes::Directories);
    let files: Vec<RemoteFile> =
        expected_remote_files(&nodes, compressed_pbo_files_only, has_zsync(protocol))
            .into_iter()
            .filter(|f| !f.is_directory)
            .collect();
    let connections = usize::try_from(repository.repository_check_connections()).unwrap_or(1);
    let mut transport = connection.into_transport();
    events.emit(SyncEvent::Phase(Phase::CheckingRemoteFiles));
    let result = {
        let mut session = transport.upload_session();
        check_remote_files(
            &mut session,
            Some((protocol, options)),
            connections,
            &files,
            ctx,
            events,
            true,
        )
        .await
    };
    transport.close().await;
    let missing = result?;
    Ok(ContentCheckReport {
        metadata,
        checked: files.len(),
        missing: missing.into_iter().map(|i| files[i].clone()).collect(),
    })
}

/// Fetches `sync`, `serverinfo`, `changelogs` and `autoconfig`, in this order, into
/// `metadata`; the first failure is returned.
// Mirrors service/administration/RepositoryCheckProcessor.java:43-46
async fn fetch_check_metadata(
    connection: &mut RemoteConnection,
    metadata: &mut RemoteMetadata,
    ctx: &TransferCtx,
) -> Result<Arc<SyncTreeDirectory>, RemoteError> {
    let (sync, _) = connection
        .fetch_a3s::<SyncTreeDirectory>(A3sFile::Sync, ctx)
        .await?;
    let sync = Arc::new(sync);
    metadata.sync = Some(Arc::clone(&sync));
    let (server_info, _) = connection
        .fetch_a3s::<ServerInfo>(A3sFile::ServerInfo, ctx)
        .await?;
    metadata.server_info = Some(Arc::new(server_info));
    let (changelogs, _) = connection
        .fetch_a3s::<Changelogs>(A3sFile::Changelogs, ctx)
        .await?;
    metadata.changelogs = Some(Arc::new(changelogs));
    let (auto_config, _) = connection
        .fetch_a3s::<AutoConfig>(A3sFile::AutoConfig, ctx)
        .await?;
    metadata.auto_config = Some(Arc::new(auto_config));
    Ok(sync)
}

/// Shared state of the check workers.
struct CheckState<'a> {
    files: &'a [RemoteFile],
    next: AtomicUsize,
    stop: CancellationToken,
    progress: Mutex<PercentCounter>,
    missing: Mutex<Vec<usize>>,
    first_error: Mutex<Option<CheckError>>,
    events: &'a EventSender,
    report_missing: bool,
}

/// Where a worker's connection comes from.
enum WorkerConnection<'a> {
    /// The caller's connection.
    Borrowed(&'a mut Transport),

    /// A new connection, opened when the worker starts.
    Lazy(&'a Protocol, &'a TransportOptions),
}

/// Tests `files` with `exists` on `first` plus up to `connections - 1` new connections to
/// `extra` (none without it), sharing one work index. The caller keeps `first` in an upload
/// session; each new connection gets its own. Returns the indices of the missing
/// files, sorted.
///
/// The first error stops every worker and is returned. A missing file emits `FileFailed` when
/// `report_missing`. Progress is emitted after every file, each value once.
// Mirrors service/connection/ConnectionCheckProcessor.java:60-151
pub(crate) async fn check_remote_files(
    first: &mut Transport,
    extra: Option<(&Protocol, &TransportOptions)>,
    connections: usize,
    files: &[RemoteFile],
    ctx: &TransferCtx,
    events: &EventSender,
    report_missing: bool,
) -> Result<Vec<usize>, CheckError> {
    if ctx.cancel.is_cancelled() {
        return Err(CheckError::Cancelled);
    }
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let workers = if extra.is_some() {
        connections.clamp(1, files.len())
    } else {
        1
    };
    let state = CheckState {
        files,
        next: AtomicUsize::new(0),
        stop: ctx.cancel.child_token(),
        progress: Mutex::new(PercentCounter::new(files.len() as u64)),
        missing: Mutex::new(Vec::new()),
        first_error: Mutex::new(None),
        events,
        report_missing,
    };
    let mut slots = vec![WorkerConnection::Borrowed(first)];
    if let Some((protocol, options)) = extra {
        slots.extend((1..workers).map(|_| WorkerConnection::Lazy(protocol, options)));
    }
    futures::future::join_all(slots.into_iter().map(|slot| check_worker(slot, &state))).await;
    if ctx.cancel.is_cancelled() {
        return Err(CheckError::Cancelled);
    }
    if let Some(err) = state.first_error.into_inner() {
        return Err(err);
    }
    let mut missing = state.missing.into_inner();
    missing.sort_unstable();
    Ok(missing)
}

async fn check_worker(slot: WorkerConnection<'_>, state: &CheckState<'_>) {
    let ctx = TransferCtx {
        cancel: state.stop.clone(),
        ..TransferCtx::default()
    };
    match slot {
        WorkerConnection::Borrowed(transport) => check_files(transport, state, &ctx).await,
        WorkerConnection::Lazy(protocol, options) => {
            if state.next.load(Ordering::Relaxed) >= state.files.len() {
                return;
            }
            match RemoteConnection::connect(protocol, options, &ctx).await {
                Ok(connection) => {
                    let mut transport = connection.into_transport();
                    {
                        let mut session = transport.upload_session();
                        check_files(&mut session, state, &ctx).await;
                    }
                    transport.close().await;
                }
                Err(err) => record_error(state, CheckError::from_remote(err)),
            }
        }
    }
}

fn record_error(state: &CheckState<'_>, err: CheckError) {
    if state.stop.is_cancelled() || matches!(err, CheckError::Cancelled) {
        return;
    }
    let mut first = state.first_error.lock();
    if first.is_none() {
        *first = Some(err);
    }
    state.stop.cancel();
}

/// Tests files on `transport`, which the caller has in an upload session.
async fn check_files(transport: &mut Transport, state: &CheckState<'_>, ctx: &TransferCtx) {
    while !state.stop.is_cancelled() {
        let index = state.next.fetch_add(1, Ordering::Relaxed);
        let Some(file) = state.files.get(index) else {
            break;
        };
        match transport.exists(&file.transport_path(), ctx).await {
            Ok(true) => {}
            Ok(false) => {
                state.missing.lock().push(index);
                if state.report_missing {
                    state.events.emit(SyncEvent::FileFailed {
                        relative_path: file.transport_path(),
                        error: EventError::new(ErrorKind::NotFound, missing_message(file))
                            .with_path(file.transport_path()),
                    });
                }
            }
            Err(source) => {
                let err = if matches!(source, a3s_transport::Error::Cancelled) {
                    CheckError::Cancelled
                } else {
                    CheckError::Exists {
                        relative_path: file.relative_file_path(),
                        source,
                    }
                };
                record_error(state, err);
                break;
            }
        }
        let mut progress = state.progress.lock();
        if let Some(percent) = progress.increment() {
            state.events.emit(SyncEvent::Progress {
                phase: Phase::CheckingRemoteFiles,
                percent,
            });
        }
    }
}
