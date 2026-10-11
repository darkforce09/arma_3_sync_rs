//! The download executor: runs a list of [`DownloadTask`]s over parallel connections.
//!
//! Replaces `service/connection/ConnectionDownloadProcessor`, the download half of
//! `dao/connection/AbstractConnexionDAO` and the zsync driver `jazsync/FileMaker`, plus the
//! progress listeners of `service/synchronization/FilesSynchronizationProcessor`.
//!
//! [`download`] creates directory tasks first, then starts `min(connections, pending files)`
//! workers, each owning one [`a3s_transport::Transport`]. Files are taken last-first (Java pops a
//! `Stack`), written to `<target>.part` and renamed once complete. Compressed leaves download
//! `<name>.zip`, which is handed to a [`ZipSink`] (the unzipper) as soon as it is complete.
//! Leaves with a zsync completion are patched from range requests over HTTP. The run emits
//! [`SyncEvent`](crate::SyncEvent)s but never `Finished`: the caller builds the final
//! [`Outcome`] with [`DownloadReport::outcome`] after unzipping.

mod error;
mod executor;
mod file;
mod monitor;
mod sink;
mod zsync;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use a3s_transport::{Throttle, TransportOptions};
use a3s_types::{Protocol, ServerInfo};

use crate::{DownloadTask, EventError, Outcome, PlanNode, PlanTree, SyncOptions};

pub use error::{DownloadError, FileError};
pub use executor::download;
pub use monitor::{WaitingCounter, average_speed, disk_usage_rate, file_percent, remaining_time};
pub use sink::{CollectZips, DownloadedZip, NoZipSink, ZipSink};

/// Suffix of files being downloaded (Java `DataAccessConstants.PART_EXTENSION`).
pub use crate::plan::PART_EXTENSION;

/// Suffix of compressed files on the server.
pub const ZIP_EXTENSION: &str = ".zip";

/// Suffix of zsync control files on the server.
pub const ZSYNC_EXTENSION: &str = ".zsync";

/// Resume retries per task after an incomplete or interrupted transfer.
pub const MAX_RESUME_RETRIES: u32 = 3;

/// Minimum interval between two `Transfer` events.
pub const TRANSFER_EVENT_INTERVAL: Duration = Duration::from_millis(100);

/// Interval of `Speed` events.
// Mirrors service/synchronization/FilesSynchronizationProcessor.java:252
pub const SPEED_INTERVAL: Duration = Duration::from_millis(500);

/// Interval of `DiskUsage` samples.
// Mirrors service/synchronization/FilesSynchronizationDiskUsageListerner.java:46
pub const DISK_USAGE_INTERVAL: Duration = Duration::from_millis(500);

/// Interval of `ConnectionWaiting` events.
// Mirrors service/synchronization/FilesSynchronizationDiskUsageListerner.java:82
pub const WAITING_TICK: Duration = Duration::from_secs(1);

/// Settings of one download run.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    /// Repository name (logs only).
    pub repository_name: String,

    /// The repository's protocol.
    pub protocol: Protocol,

    /// Connection options of every worker.
    pub transport_options: TransportOptions,

    /// Parallel connections; clamped to at least 1.
    pub connections: u32,

    /// Abort after this many failed files; clamped to at least 1.
    pub max_errors: usize,

    /// Global speed limit shared by every worker; [`Throttle::set_limit`] may be called during a
    /// run.
    pub throttle: Arc<Throttle>,

    /// Folder whose free space is sampled for `DiskUsage` events; `None` disables them.
    pub disk_usage_path: Option<PathBuf>,
}

impl DownloadRequest {
    /// The request for synchronizing with `options`: `connections` from
    /// [`SyncOptions::download_connections`], the error limit and speed limit of the options, and
    /// disk usage sampled in the destination folder.
    // Mirrors service/synchronization/FilesSynchronizationProcessor.java:65-83,172
    pub fn new(
        protocol: Protocol,
        transport_options: TransportOptions,
        options: &SyncOptions,
        server_info: Option<&ServerInfo>,
    ) -> Self {
        Self {
            repository_name: options.repository_name.clone(),
            protocol,
            transport_options,
            connections: options.download_connections(server_info),
            max_errors: options.max_errors,
            throttle: Arc::new(Throttle::new(options.max_download_speed)),
            disk_usage_path: Some(options.destination.clone()),
        }
    }
}

/// How a download run ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum DownloadEnd {
    /// Every task was attempted (some may have failed, see [`DownloadReport::errors`]).
    Completed,

    /// Aborted after `limit` failed files.
    TooManyErrors {
        /// The error limit.
        limit: usize,
    },

    /// The connection to the server was lost; `SyncEvent::ConnectionLost` was emitted.
    ConnectionLost {
        /// The first connection error.
        error: DownloadError,
    },

    /// No connection could be opened, for a reason other than a lost connection.
    Failed {
        /// The first connection error.
        error: DownloadError,
    },

    /// The caller's token was cancelled.
    Cancelled,
}

/// Result of [`download`].
#[derive(Debug)]
pub struct DownloadReport {
    /// How the run ended.
    pub end: DownloadEnd,

    /// Failed files, in occurrence order.
    pub errors: Vec<FileError>,

    /// Downloaded `.zip` files, in completion order (the items given to the [`ZipSink`]).
    pub downloaded_zips: Vec<DownloadedZip>,

    /// Tasks finalized in this call (files and directories).
    pub files_done: u32,

    /// Payload bytes received in this call (files and zsync ranges, not control files).
    pub bytes_received: u64,

    /// Running mean of the speed samples (Java's report value).
    pub average_speed: u64,

    /// Maximum number of simultaneously active connections.
    pub max_active_connections: u32,

    /// Duration of the call.
    pub elapsed: Duration,
}

impl DownloadReport {
    /// The file errors as event errors.
    pub fn event_errors(&self) -> Vec<EventError> {
        self.errors.iter().map(FileError::to_event_error).collect()
    }

    /// The outcome of the synchronization, with `extra_errors` (unzip errors) appended after the
    /// download errors. `None` after a lost connection: `ConnectionLost` is the terminal event.
    // Mirrors service/connection/ConnectionDownloadProcessor.java:139-180
    pub fn outcome(&self, extra_errors: Vec<EventError>) -> Option<Outcome> {
        let mut errors = self.event_errors();
        errors.extend(extra_errors);
        Some(match &self.end {
            DownloadEnd::Completed if errors.is_empty() => Outcome::Success,
            DownloadEnd::Completed => Outcome::CompletedWithErrors { errors },
            DownloadEnd::TooManyErrors { limit } => Outcome::TooManyErrors {
                limit: *limit,
                errors,
            },
            DownloadEnd::ConnectionLost { .. } => return None,
            DownloadEnd::Failed { error } => {
                // `Outcome::Failed` carries one error: log the others so they are not lost.
                for dropped in &errors {
                    tracing::warn!("Download error: {}", dropped.message);
                }
                Outcome::Failed {
                    error: EventError::new(error.kind(), error.to_string()),
                }
            }
            DownloadEnd::Cancelled => Outcome::Cancelled,
        })
    }
}

/// Copies each task's status to the plan node at its `relative_path`; unknown paths are
/// ignored.
// Mirrors dao/connection/AbstractConnexionDAO.java:295,303,323,328,345 (Java mutates the nodes)
pub fn apply_statuses(tree: &mut PlanTree, tasks: &[DownloadTask]) {
    for task in tasks {
        match tree.get_mut(&task.relative_path) {
            Some(PlanNode::Directory(d)) => d.download_status = task.status,
            Some(PlanNode::Leaf(l)) => l.download_status = task.status,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use a3s_foundation::error::ErrorKind;
    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

    use super::*;
    use crate::{DownloadStatus, DownloadTaskKind};

    fn task(relative_path: &str, status: DownloadStatus) -> DownloadTask {
        DownloadTask {
            relative_path: relative_path.to_owned(),
            destination_dir: PathBuf::from("/d"),
            kind: DownloadTaskKind::Directory,
            status,
        }
    }

    #[test]
    fn apply_statuses_cases() {
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![SyncTreeNode::Directory(SyncTreeDirectory {
            name: "@a".to_owned(),
            children: vec![SyncTreeNode::Leaf(SyncTreeLeaf {
                name: "x.pbo".to_owned(),
                ..Default::default()
            })],
            ..Default::default()
        })];
        let mut tree = PlanTree::from_sync_tree(&root);
        apply_statuses(
            &mut tree,
            &[
                task("@a", DownloadStatus::Done),
                task("@a/x.pbo", DownloadStatus::Running),
                task("@b/missing", DownloadStatus::Done),
            ],
        );
        assert_eq!(
            tree.get("@a").map(PlanNode::download_status),
            Some(DownloadStatus::Done)
        );
        assert_eq!(
            tree.get("@a/x.pbo").map(PlanNode::download_status),
            Some(DownloadStatus::Running)
        );
        assert!(tree.get("@b").is_none());
    }

    fn report(end: DownloadEnd, errors: usize) -> DownloadReport {
        DownloadReport {
            end,
            errors: (0..errors)
                .map(|i| FileError {
                    relative_path: format!("f{i}"),
                    error: DownloadError::incomplete(Path::new("/d/f"), 2, 1),
                })
                .collect(),
            downloaded_zips: Vec::new(),
            files_done: 0,
            bytes_received: 0,
            average_speed: 0,
            max_active_connections: 0,
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn outcome_mapping() {
        let unzip = EventError::new(ErrorKind::InvalidData, "unzip");
        assert_eq!(
            report(DownloadEnd::Completed, 0).outcome(vec![]),
            Some(Outcome::Success)
        );
        match report(DownloadEnd::Completed, 0).outcome(vec![unzip.clone()]) {
            Some(Outcome::CompletedWithErrors { errors }) => {
                assert_eq!(errors, vec![unzip.clone()]);
            }
            other => panic!("{other:?}"),
        }
        match report(DownloadEnd::Completed, 2).outcome(vec![unzip.clone()]) {
            Some(Outcome::CompletedWithErrors { errors }) => {
                assert_eq!(errors.len(), 3);
                assert_eq!(errors[2], unzip);
                assert_eq!(errors[0].relative_path.as_deref(), Some("f0"));
            }
            other => panic!("{other:?}"),
        }
        match report(DownloadEnd::TooManyErrors { limit: 10 }, 10).outcome(vec![]) {
            Some(Outcome::TooManyErrors { limit, errors }) => {
                assert_eq!((limit, errors.len()), (10, 10));
            }
            other => panic!("{other:?}"),
        }
        let lost = DownloadEnd::ConnectionLost {
            error: DownloadError::Cancelled,
        };
        assert_eq!(report(lost, 1).outcome(vec![]), None);
        assert_eq!(
            report(DownloadEnd::Cancelled, 0).outcome(vec![]),
            Some(Outcome::Cancelled)
        );
        match report(
            DownloadEnd::Failed {
                error: DownloadError::Cancelled,
            },
            0,
        )
        .outcome(vec![])
        {
            Some(Outcome::Failed { error }) => assert_eq!(error.kind, ErrorKind::Cancelled),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn request_from_options() {
        let options = SyncOptions {
            repository_name: "WOG".to_owned(),
            event_name: None,
            destination: PathBuf::from("/dest"),
            auto_discover: false,
            exact_match: false,
            hidden_folder_paths: std::collections::BTreeSet::new(),
            client_connections: 8,
            max_download_speed: 1000,
            max_errors: crate::MAX_DOWNLOAD_ERRORS,
        };
        let server = ServerInfo {
            number_of_connections: 3,
            ..Default::default()
        };
        let protocol = Protocol::new(
            a3s_types::ProtocolType::Http,
            "127.0.0.1/repo",
            "80",
            "anonymous",
            "",
            false,
        );
        let options_t =
            a3s_transport::proxy::transport_options(&protocol, &a3s_types::local::Proxy::default())
                .expect("no proxy");
        let request = DownloadRequest::new(protocol, options_t, &options, Some(&server));
        assert_eq!(request.connections, 3);
        assert_eq!(request.max_errors, 10);
        assert_eq!(request.repository_name, "WOG");
        assert_eq!(request.disk_usage_path.as_deref(), Some(Path::new("/dest")));
    }
}
