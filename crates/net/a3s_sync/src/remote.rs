//! Repository metadata access through [`a3s_transport::Transport`].
//!
//! [`RemoteConnection::check_repository`] fetches the five `.a3s` files with the Java
//! partial-failure semantics, [`import_autoconfig`] reads an autoconfig URL, and
//! [`check_repositories`] checks several repositories with bounded concurrency.

// `RemoteError` keeps the transport error inline; errors end a check, so their size is irrelevant.
#![allow(clippy::result_large_err)]

mod file;

use std::sync::Arc;

use a3s_repo_files::A3sFile;
use a3s_transport::{TransferCtx, Transport, TransportOptions};
use a3s_types::SyncTreeDirectory;
use a3s_types::local::protocol::host_url;
use a3s_types::{AutoConfig, Changelogs, Event, Events, JavaCodec, Protocol, ServerInfo};
use futures::StreamExt as _;

pub use file::RemoteFile;

use crate::RemoteError;
use crate::events::{EventSender, Phase, SyncEvent};

/// At most this many repositories are checked at once.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:26
pub const MAX_CONCURRENT_REPOSITORY_CHECKS: usize = 4;

/// Name of the metadata folder of a repository.
const A3S_FOLDER: &str = ".a3s";

/// The remote `.a3s` metadata of a repository, as of the last check.
///
/// Fields are shared (`Arc`), so snapshots are cheap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteMetadata {
    /// `.a3s/serverinfo`.
    pub server_info: Option<Arc<ServerInfo>>,

    /// `.a3s/sync`.
    pub sync: Option<Arc<SyncTreeDirectory>>,

    /// `.a3s/changelogs`.
    pub changelogs: Option<Arc<Changelogs>>,

    /// `.a3s/events`.
    pub events: Option<Arc<Events>>,

    /// `.a3s/autoconfig`.
    pub auto_config: Option<Arc<AutoConfig>>,
}

impl RemoteMetadata {
    /// Checks that the files needed to synchronize are present: `sync`, then `serverinfo`, then
    /// `events` when an event is selected.
    // Mirrors service/synchronization/FilesCheckProcessor.java:41-54
    pub fn require_for_sync(&self, event_name: Option<&str>) -> Result<(), RemoteError> {
        if self.sync.is_none() {
            return Err(RemoteError::MissingMetadata(A3sFile::Sync));
        }
        if self.server_info.is_none() {
            return Err(RemoteError::MissingMetadata(A3sFile::ServerInfo));
        }
        if event_name.is_some() && self.events.is_none() {
            return Err(RemoteError::MissingMetadata(A3sFile::Events));
        }
        Ok(())
    }

    /// The events, without duplicate names (the first one wins); empty without `events`.
    // Mirrors service/RepositoryService.java:866-892
    pub fn events_list(&self) -> Vec<&Event> {
        let mut out: Vec<&Event> = Vec::new();
        if let Some(events) = &self.events {
            for event in &events.list {
                if !out.iter().any(|e| e.name == event.name) {
                    out.push(event);
                }
            }
        }
        out
    }

    /// The server revision, if the server info is known.
    pub fn server_revision(&self) -> Option<i32> {
        self.server_info.as_ref().map(|s| s.revision)
    }
}

/// Whether the server supports partial (range) transfers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialTransferSupport {
    /// The server answered the range probe with 206.
    Supported,

    /// The server ignored the range (any other success status).
    Unsupported,

    /// Not an HTTP repository; partial transfer doesn't apply.
    NotApplicable,
}

/// One connection to a repository.
pub struct RemoteConnection {
    transport: Transport,
    protocol: Protocol,
}

impl RemoteConnection {
    /// Connects (and logs in, for FTP and SFTP). Returns [`RemoteError::Cancelled`] as soon as
    /// `ctx` is cancelled.
    // Mirrors service/ConnectionService.java:59-89
    pub async fn connect(
        protocol: &Protocol,
        options: &TransportOptions,
        ctx: &TransferCtx,
    ) -> Result<Self, RemoteError> {
        if ctx.cancel.is_cancelled() {
            return Err(RemoteError::Cancelled);
        }
        let transport = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(RemoteError::Cancelled),
            res = Transport::connect(protocol, options) => {
                res.map_err(|e| RemoteError::connect(protocol, e))?
            }
        };
        Ok(Self {
            transport,
            protocol: protocol.clone(),
        })
    }

    /// The protocol this connection was opened with.
    pub fn protocol(&self) -> &Protocol {
        &self.protocol
    }

    /// The underlying transport.
    pub fn transport_mut(&mut self) -> &mut Transport {
        &mut self.transport
    }

    /// Consumes the connection, returning the transport.
    pub fn into_transport(self) -> Transport {
        self.transport
    }

    /// Fetches and decodes `.a3s/<file>`; returns the value and the downloaded (compressed)
    /// size. `.a3s/sync` is decoded on a blocking thread.
    // Mirrors dao/connection/AbstractConnexionDAO.java:144-174 (in memory, no temp folder)
    pub async fn fetch_a3s<T: JavaCodec + Send + 'static>(
        &mut self,
        file: A3sFile,
        ctx: &TransferCtx,
    ) -> Result<(T, u64), RemoteError> {
        if ctx.cancel.is_cancelled() {
            return Err(RemoteError::Cancelled);
        }
        let remote = RemoteFile::new(file.file_name(), A3S_FOLDER, false);
        let bytes = self
            .transport
            .fetch(&remote.transport_path(), ctx)
            .await
            .map_err(|e| RemoteError::fetch(&self.protocol, &remote, e))?;
        let len = bytes.len() as u64;
        let decoded = if file == A3sFile::Sync {
            match tokio::task::spawn_blocking(move || a3s_repo_files::decode_a3s::<T>(&bytes)).await
            {
                Ok(decoded) => decoded,
                Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
                // The runtime is shutting down.
                Err(_) => return Err(RemoteError::Cancelled),
            }
        } else {
            a3s_repo_files::decode_a3s::<T>(&bytes)
        };
        let value = decoded.map_err(|e| RemoteError::decode(file, e))?;
        Ok((value, len))
    }

    /// Fetches `serverinfo`, `sync`, `changelogs`, `events` and `autoconfig`, in this order,
    /// into `metadata`.
    ///
    /// A fetched file replaces its field. The first failure sets its field to `None`, leaves
    /// the later fields untouched, and is returned. Cancellation returns
    /// [`RemoteError::Cancelled`] (the file in flight, if any, becomes `None`).
    ///
    /// Emits [`Phase::CheckingRepository`], then one [`SyncEvent::MetadataFetched`] per file.
    // Mirrors service/ConnectionService.java:175-224
    pub async fn check_repository(
        &mut self,
        repository_name: &str,
        metadata: &mut RemoteMetadata,
        ctx: &TransferCtx,
        events: &EventSender,
    ) -> Result<(), RemoteError> {
        tracing::info!(
            "Checking repository: {repository_name} on url: {}",
            host_url(&self.protocol)
        );
        events.emit(SyncEvent::Phase(Phase::CheckingRepository));
        let previous = metadata.server_info.clone();

        self.fetch_into(
            A3sFile::ServerInfo,
            &mut metadata.server_info,
            repository_name,
            ctx,
            events,
        )
        .await?;

        // The sync tree is always refetched; the server info is only a hint.
        // Mirrors service/ConnectionService.java:195-210
        ensure_not_cancelled(ctx)?;
        match (&previous, &metadata.server_info) {
            (Some(previous), Some(current)) if server_info_changed(previous, current) => {
                tracing::info!(
                    "Repository metadata changed (revision {} -> {}), refreshing repository layout.",
                    previous.revision,
                    current.revision
                );
            }
            _ => tracing::info!("Refreshing repository layout from the current server sync file."),
        }
        self.fetch_into(
            A3sFile::Sync,
            &mut metadata.sync,
            repository_name,
            ctx,
            events,
        )
        .await?;
        self.fetch_into(
            A3sFile::Changelogs,
            &mut metadata.changelogs,
            repository_name,
            ctx,
            events,
        )
        .await?;
        self.fetch_into(
            A3sFile::Events,
            &mut metadata.events,
            repository_name,
            ctx,
            events,
        )
        .await?;
        self.fetch_into(
            A3sFile::AutoConfig,
            &mut metadata.auto_config,
            repository_name,
            ctx,
            events,
        )
        .await
    }

    /// Fetches one file into `slot`: the value on success, `None` on failure.
    // Mirrors service/ConnectionService.java:91-171 (the `finally` blocks)
    async fn fetch_into<T: JavaCodec + Send + 'static>(
        &mut self,
        file: A3sFile,
        slot: &mut Option<Arc<T>>,
        repository_name: &str,
        ctx: &TransferCtx,
        events: &EventSender,
    ) -> Result<(), RemoteError> {
        ensure_not_cancelled(ctx)?;
        // Mirrors dao/connection/AbstractConnexionDAO.java:153-154
        tracing::info!(
            "Downloading file: /{A3S_FOLDER}/{} from repository: {repository_name}",
            file.file_name()
        );
        match self.fetch_a3s::<T>(file, ctx).await {
            Ok((value, bytes)) => {
                *slot = Some(Arc::new(value));
                events.emit(SyncEvent::MetadataFetched { file, bytes });
                Ok(())
            }
            Err(err) => {
                *slot = None;
                Err(err)
            }
        }
    }

    /// Probes whether the server answers range requests: `Range: bytes=0-1` on `.a3s/sync`.
    ///
    /// Only HTTP(S) repositories are probed; other transports return
    /// [`PartialTransferSupport::NotApplicable`].
    // Mirrors dao/connection/http/HttpDAO.java:431-446, service/ConnectionService.java:256-266
    pub async fn probe_partial_transfer(
        &mut self,
        ctx: &TransferCtx,
    ) -> Result<PartialTransferSupport, RemoteError> {
        if !probe_applies(&self.transport) {
            return Ok(PartialTransferSupport::NotApplicable);
        }
        ensure_not_cancelled(ctx)?;
        let remote = RemoteFile::new(A3sFile::Sync.file_name(), A3S_FOLDER, false);
        match self
            .transport
            .fetch_range(&remote.transport_path(), 0..2, ctx)
            .await
        {
            // Java only looks at the status: a short 206 body still means "supported".
            Ok(_) | Err(a3s_transport::Error::Incomplete { .. }) => {
                Ok(PartialTransferSupport::Supported)
            }
            Err(a3s_transport::Error::Unsupported { .. }) => {
                Ok(PartialTransferSupport::Unsupported)
            }
            Err(err) => Err(RemoteError::fetch(&self.protocol, &remote, err)),
        }
    }

    /// Closes the connection politely.
    pub async fn close(self) {
        self.transport.close().await;
    }
}

/// Only plain HTTP(S) transports have a meaningful range probe.
fn probe_applies(transport: &Transport) -> bool {
    matches!(transport, Transport::Http(_))
}

fn ensure_not_cancelled(ctx: &TransferCtx) -> Result<(), RemoteError> {
    if ctx.cancel.is_cancelled() {
        Err(RemoteError::Cancelled)
    } else {
        Ok(())
    }
}

/// Connects, runs [`RemoteConnection::check_repository`] and closes the connection.
pub async fn check_repository(
    repository_name: &str,
    protocol: &Protocol,
    options: &TransportOptions,
    metadata: &mut RemoteMetadata,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<(), RemoteError> {
    let mut connection = RemoteConnection::connect(protocol, options, ctx).await?;
    let result = connection
        .check_repository(repository_name, metadata, ctx, events)
        .await;
    connection.close().await;
    result
}

/// Fetches and decodes the autoconfig file of an autoconfig URL.
///
/// The protocol's URL points at the folder holding the file (normally `<repository>/.a3s`).
// Mirrors service/ConnectionService.java:242-254, dao/connection/AbstractConnexionDAO.java:176-201
pub async fn import_autoconfig(
    protocol: &Protocol,
    options: &TransportOptions,
    ctx: &TransferCtx,
) -> Result<AutoConfig, RemoteError> {
    let file_name = A3sFile::AutoConfig.file_name();
    tracing::info!(
        "Importing autoconfig from url: {}/{file_name}",
        host_url(protocol)
    );
    let mut connection = RemoteConnection::connect(protocol, options, ctx).await?;
    let remote = RemoteFile::new(file_name, "", false);
    let result = connection
        .transport
        .fetch(&remote.transport_path(), ctx)
        .await;
    connection.close().await;
    let bytes = result.map_err(|e| RemoteError::fetch(protocol, &remote, e))?;
    ensure_not_cancelled(ctx)?;
    a3s_repo_files::decode_a3s(&bytes).map_err(|e| RemoteError::decode(A3sFile::AutoConfig, e))
}

/// One repository to check with [`check_repositories`].
#[derive(Debug)]
pub struct CheckJob<'a> {
    /// The repository name (for logs and results).
    pub repository_name: String,

    /// Download protocol.
    pub protocol: Protocol,

    /// Connection options.
    pub options: TransportOptions,

    /// Metadata updated in place (see [`RemoteConnection::check_repository`]).
    pub metadata: &'a mut RemoteMetadata,
}

/// Checks every job, each on its own connection, and returns the results in input order.
///
/// At most [`MAX_CONCURRENT_REPOSITORY_CHECKS`] checks run at once. Every job runs even if
/// others fail; failures are logged ([`check_error_message`]).
// Mirrors ui/main/tasks/TaskCheckRepositories.java:41-76
pub async fn check_repositories(
    jobs: Vec<CheckJob<'_>>,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Vec<(String, Result<(), RemoteError>)> {
    tracing::info!("Checking repositories...");
    // Collect the futures first: a closure in the stream type breaks the compiler's `Send`
    // inference for the returned future.
    let checks: Vec<_> = jobs
        .into_iter()
        .map(|job| check_job(job, ctx, events))
        .collect();
    let results = futures::stream::iter(checks)
        .buffered(MAX_CONCURRENT_REPOSITORY_CHECKS)
        .collect()
        .await;
    tracing::info!("Checking repositories done.");
    results
}

async fn check_job(
    job: CheckJob<'_>,
    ctx: &TransferCtx,
    events: &EventSender,
) -> (String, Result<(), RemoteError>) {
    let CheckJob {
        repository_name,
        protocol,
        options,
        metadata,
    } = job;
    let result =
        check_repository(&repository_name, &protocol, &options, metadata, ctx, events).await;
    if let Err(err) = &result {
        tracing::info!("{}", check_error_message(&repository_name, err));
    }
    (repository_name, result)
}

/// The log line of a failed repository check.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:57-58
pub fn check_error_message(repository_name: &str, error: &RemoteError) -> String {
    format!("Error when checking repository {repository_name}:\n{error}")
}

/// Whether the revision, file count or total size differs.
// Mirrors service/ConnectionService.java:227-238
pub fn server_info_changed(previous: &ServerInfo, current: &ServerInfo) -> bool {
    previous.revision != current.revision
        || previous.number_of_files != current.number_of_files
        || previous.total_files_size != current.total_files_size
}

#[cfg(test)]
mod tests {
    use a3s_types::{Event, Events, ProtocolType, ServerInfo};

    use super::*;

    #[test]
    fn a12_server_info_changed() {
        let base = ServerInfo {
            revision: 5,
            number_of_files: 10,
            total_files_size: 100,
            ..Default::default()
        };
        assert!(!server_info_changed(&base, &base.clone()));
        let mut other = base.clone();
        other.revision = 6;
        assert!(server_info_changed(&base, &other));
        let mut other = base.clone();
        other.number_of_files = 11;
        assert!(server_info_changed(&base, &other));
        let mut other = base.clone();
        other.total_files_size = 99;
        assert!(server_info_changed(&base, &other));
        let mut other = base.clone();
        other.number_of_connections = 9;
        assert!(!server_info_changed(&base, &other));
    }

    #[tokio::test]
    async fn a6_probe_applies_to_http_only() {
        for (protocol_type, applies) in [
            (ProtocolType::Http, true),
            (ProtocolType::Https, true),
            (ProtocolType::HttpWebdav, false),
            (ProtocolType::HttpsWebdav, false),
        ] {
            // HTTP and WebDAV transports connect lazily: no network needed.
            let protocol =
                Protocol::new(protocol_type, "127.0.0.1/repo", "80", "anonymous", "", true);
            let options = TransportOptions::from_protocol(&protocol);
            let transport = Transport::connect(&protocol, &options)
                .await
                .expect("lazy transport");
            assert_eq!(probe_applies(&transport), applies, "{protocol_type:?}");
        }
    }

    #[test]
    fn a18_require_for_sync_order() {
        let mut m = RemoteMetadata::default();
        let err = m.require_for_sync(Some("e")).expect_err("sync missing");
        assert_eq!(err.to_string(), "Remote file no found: /.a3s/sync");

        m.sync = Some(Arc::new(SyncTreeDirectory::new_root()));
        let err = m
            .require_for_sync(Some("e"))
            .expect_err("serverinfo missing");
        assert_eq!(err.to_string(), "Remote file not found: .a3s/serverinfo");

        m.server_info = Some(Arc::new(ServerInfo::default()));
        assert!(m.require_for_sync(None).is_ok());
        let err = m.require_for_sync(Some("e")).expect_err("events missing");
        assert_eq!(err.to_string(), "Remote file not found: /.a3s/events");
        assert!(matches!(err, RemoteError::MissingMetadata(A3sFile::Events)));

        m.events = Some(Arc::new(Events::default()));
        assert!(m.require_for_sync(Some("e")).is_ok());
        assert_eq!(m.server_revision(), Some(0));
    }

    #[test]
    fn a18_events_list_dedupes_by_name() {
        let event = |name: &str, description: &str| Event {
            name: name.to_owned(),
            description: Some(description.to_owned()),
            ..Default::default()
        };
        let mut m = RemoteMetadata::default();
        assert!(m.events_list().is_empty());
        assert_eq!(m.server_revision(), None);
        m.events = Some(Arc::new(Events {
            list: vec![
                event("A", "first"),
                event("B", "b"),
                event("A", "second"),
                event("a", "lowercase"),
            ],
        }));
        let list: Vec<_> = m
            .events_list()
            .into_iter()
            .map(|e| (e.name.as_str(), e.description.as_deref()))
            .collect();
        assert_eq!(
            list,
            [
                ("A", Some("first")),
                ("B", Some("b")),
                ("a", Some("lowercase"))
            ]
        );
    }
}
