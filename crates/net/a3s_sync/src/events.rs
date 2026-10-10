//! Progress and outcome events of check, download, uncompress and upload.
//!
//! One [`SyncEvent`] enum replaces the Java observer interfaces (`controller/Observer*`). An
//! [`EventSender`] delivers them to a std or tokio channel, a closure (e.g. an egui repaint), or
//! nowhere.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_error::ErrorKind;
use a3s_repo_files::A3sFile;

use crate::RemoteError;

/// After this long without throughput the UI shows [`Phase::WaitingForServer`].
// Mirrors ui/repository/workers/AddonsDownloader.java:376-383
pub const WAITING_FOR_SERVER_THRESHOLD: Duration = Duration::from_secs(15);

/// What the engine is doing (the Java status labels and `ObserverText` texts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Phase {
    /// Fetching `.a3s` metadata.
    CheckingRepository,

    /// Comparing local files with the sync tree.
    CheckingFiles,

    /// Computing zsync completion of local files.
    DeterminingCompletion,

    /// Downloading files.
    Downloading,

    /// Connections are open but no data arrived for [`WAITING_FOR_SERVER_THRESHOLD`].
    WaitingForServer,

    /// The download is paused.
    Paused,

    /// Uncompressing downloaded `.zip` files.
    Uncompressing,

    /// Deleting extra local files.
    DeletingExtraFiles,

    /// Updating the `userconfig` folder.
    UpdatingUserconfig,

    /// Upload: checking which remote files exist.
    CheckingRemoteFiles,

    /// Upload: uploading files.
    Uploading,

    /// Upload: deleting extra remote files.
    DeletingRemoteFiles,
}

impl Phase {
    /// The label shown to the user.
    // Mirrors ui/repository/workers/AddonsChecker.java:119, AddonsDownloader.java:150,185,380,393,
    // service/ConnectionService.java:418,429,437
    pub fn label(self) -> &'static str {
        match self {
            Self::CheckingRepository => "Checking repository...",
            Self::CheckingFiles => "Checking files...",
            Self::DeterminingCompletion => "Determining file completion...",
            Self::Downloading => "Downloading...",
            Self::WaitingForServer => "Waiting for server...",
            Self::Paused => "Paused...",
            Self::Uncompressing => "Uncompressing...",
            Self::DeletingExtraFiles => "Deleting extra local files...",
            Self::UpdatingUserconfig => "Updating userconfig...",
            Self::CheckingRemoteFiles => "Checking remote files...",
            Self::Uploading => "Uploading files...",
            Self::DeletingRemoteFiles => "Deleting extra remote files...",
        }
    }
}

/// A progress or outcome notification.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SyncEvent {
    /// A new phase started (`ObserverText`, status labels).
    Phase(Phase),

    /// A metadata file was fetched and decoded; `bytes` is its compressed size.
    MetadataFetched {
        /// The file.
        file: A3sFile,
        /// Downloaded (compressed) bytes.
        bytes: u64,
    },

    /// Percentage of a phase (`ObserverCountInt`): SHA-1, completion, uncompress.
    Progress {
        /// The phase.
        phase: Phase,
        /// `0..=100`.
        percent: u8,
    },

    /// Total of the transfer (`ObserverDownload.updateTotalSize`).
    TransferTotal {
        /// Bytes to transfer.
        bytes: u64,
        /// Files to transfer.
        files: u32,
    },

    /// Transfer progress (`updateTotalSizeProgress`, `updateSingleSizeProgress`,
    /// `observerDownloadedSize` and the remaining-time observer).
    Transfer(TransferProgress),

    /// Current throughput of all connections, sampled every ≥ 0.5 s (`updateSpeed`).
    Speed {
        /// Bytes per second.
        bytes_per_second: u64,
    },

    /// Local write throughput, every 0.5 s (`observerDiskUsage`).
    DiskUsage {
        /// Bytes per second.
        bytes_per_second: u64,
    },

    /// Number of connections transferring (`updateActiveConnections`).
    ActiveConnections {
        /// Active connections.
        active: u32,
    },

    /// Seconds without throughput (`observerConnectionWaiting`).
    ConnectionWaiting {
        /// Seconds.
        seconds: u32,
    },

    /// A file transfer started.
    FileStarted {
        /// Path below the repository root.
        relative_path: String,
    },

    /// A file transfer finished.
    FileDone {
        /// Path below the repository root.
        relative_path: String,
    },

    /// A file transfer failed (the synchronization may continue).
    FileFailed {
        /// Path below the repository root.
        relative_path: String,
        /// The error.
        error: EventError,
    },

    /// Upload: index of the last file uploaded (`ObserverUpload.updateLastIndexFileUploaded`).
    LastIndexTransferred {
        /// Index.
        index: u32,
    },

    /// The connection to the server was lost (`updateConnectionLost`).
    ConnectionLost,

    /// The operation ended.
    Finished(Outcome),
}

/// Transfer progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferProgress {
    /// Resumed plus currently transferred bytes (Java `downloadedFilesSize`).
    pub transferred: u64,

    /// Bytes to transfer in total.
    pub total: u64,

    /// Path of the file being transferred.
    pub current_file: Option<String>,

    /// Percentage of the current file.
    pub current_file_percent: u8,

    /// Estimated remaining time. Java computes `remaining_bytes * elapsed / transferred`
    /// (`FilesSynchronizationProcessor.java:211-222`).
    pub remaining: Option<Duration>,
}

impl TransferProgress {
    /// `transferred * 100 / total` (integer division), 0 when `total` is 0, at most 100.
    // Mirrors service/synchronization/FilesSynchronizationProcessor.java:224-230
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        let percent = u128::from(self.transferred) * 100 / u128::from(self.total);
        percent.min(100) as u8
    }
}

/// How an operation ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Everything succeeded (`ObserverEnd.end`).
    Success,

    /// Finished, but some files failed (`ObserverDownload.error`).
    CompletedWithErrors {
        /// The errors.
        errors: Vec<EventError>,
    },

    /// Aborted after `limit` errors (`updateCancelTooManyErrors`).
    TooManyErrors {
        /// The error limit ([`crate::options::MAX_DOWNLOAD_ERRORS`]).
        limit: usize,
        /// The errors.
        errors: Vec<EventError>,
    },

    /// A single fatal error (`ObserverError.error`).
    Failed {
        /// The error.
        error: EventError,
    },

    /// Cancelled by the user.
    Cancelled,
}

impl Outcome {
    /// The headline of the download report, or `None` after a cancel.
    // Mirrors service/synchronization/FilesSynchronizationProcessor.java:121,127,157,200,317
    pub fn download_report_message(&self) -> Option<String> {
        match self {
            Self::Success => Some("Download finished successfully.".to_owned()),
            Self::CompletedWithErrors { .. } => Some("Download finished with errors".to_owned()),
            Self::TooManyErrors { limit, .. } => Some(format!(
                "Download has been canceled due to too many errors (at least {limit})"
            )),
            Self::Failed { .. } => Some("Download finished with error".to_owned()),
            Self::Cancelled => None,
        }
    }

    /// Whether this is [`Outcome::Success`].
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
}

/// A cloneable projection of an error, carried inside events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventError {
    /// Classification.
    pub kind: ErrorKind,

    /// The user-facing message.
    pub message: String,

    /// The affected file below the repository root, if any.
    pub relative_path: Option<String>,
}

impl EventError {
    /// An error without a path.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            relative_path: None,
        }
    }

    /// The projection of a [`RemoteError`].
    pub fn from_remote(err: &RemoteError) -> Self {
        Self {
            kind: err.kind(),
            message: err.to_string(),
            relative_path: err.relative_path(),
        }
    }

    /// Sets the affected file.
    #[must_use]
    pub fn with_path(mut self, relative_path: impl Into<String>) -> Self {
        self.relative_path = Some(relative_path.into());
        self
    }
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

type Sink = Arc<dyn Fn(SyncEvent) + Send + Sync>;

/// Where events go. Cloning is cheap; clones feed the same sink.
///
/// [`EventSender::emit`] never blocks: channels are unbounded, closures must not block, and
/// events sent after the receiver was dropped are discarded.
#[derive(Clone, Default)]
pub struct EventSender {
    sink: Option<Sink>,
}

impl EventSender {
    /// A sender that discards every event.
    pub fn noop() -> Self {
        Self { sink: None }
    }

    /// A sender that calls `f` for every event (on the emitting thread).
    pub fn from_fn(f: impl Fn(SyncEvent) + Send + Sync + 'static) -> Self {
        Self {
            sink: Some(Arc::new(f)),
        }
    }

    /// A sender feeding an unbounded std channel.
    pub fn channel() -> (Self, std::sync::mpsc::Receiver<SyncEvent>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sender = Self::from_fn(move |event| {
            // The receiver may be gone; events are best-effort.
            tx.send(event).ok();
        });
        (sender, rx)
    }

    /// A sender feeding an unbounded tokio channel.
    pub fn tokio_channel() -> (Self, tokio::sync::mpsc::UnboundedReceiver<SyncEvent>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = Self::from_fn(move |event| {
            tx.send(event).ok();
        });
        (sender, rx)
    }

    /// Delivers `event`.
    pub fn emit(&self, event: SyncEvent) {
        if let Some(sink) = &self.sink {
            sink(event);
        }
    }

    /// Whether events are discarded.
    pub fn is_noop(&self) -> bool {
        self.sink.is_none()
    }
}

impl fmt::Debug for EventSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSender")
            .field("noop", &self.is_noop())
            .finish()
    }
}

/// Turns a count of processed items into percentages, reporting each value once.
// Mirrors dao/repository/RepositorySHA1Processor.java:169-176
#[derive(Clone, Debug)]
pub struct PercentCounter {
    total: u64,
    count: u64,
    last: Option<u8>,
}

impl PercentCounter {
    /// A counter for `total` items.
    pub fn new(total: u64) -> Self {
        Self {
            total,
            count: 0,
            last: None,
        }
    }

    /// Counts one item; returns `count * 100 / total` when it differs from the last value
    /// returned. A total of 0 never reports.
    pub fn increment(&mut self) -> Option<u8> {
        if self.total == 0 {
            return None;
        }
        self.count += 1;
        let value = (u128::from(self.count) * 100 / u128::from(self.total)).min(100) as u8;
        if self.last == Some(value) {
            None
        } else {
            self.last = Some(value);
            Some(value)
        }
    }
}

/// Lets an action through at most once per interval.
#[derive(Clone, Debug)]
pub struct RateLimiter {
    interval: Duration,
    last: Option<Instant>,
}

impl RateLimiter {
    /// A limiter with the given minimum interval.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// `true` on the first call, then once `interval` has elapsed since the last `true`.
    pub fn ready(&mut self, now: Instant) -> bool {
        let ready = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= self.interval);
        if ready {
            self.last = Some(now);
        }
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a9_phase_labels() {
        let table = [
            (Phase::CheckingRepository, "Checking repository..."),
            (Phase::CheckingFiles, "Checking files..."),
            (
                Phase::DeterminingCompletion,
                "Determining file completion...",
            ),
            (Phase::Downloading, "Downloading..."),
            (Phase::WaitingForServer, "Waiting for server..."),
            (Phase::Paused, "Paused..."),
            (Phase::Uncompressing, "Uncompressing..."),
            (Phase::DeletingExtraFiles, "Deleting extra local files..."),
            (Phase::UpdatingUserconfig, "Updating userconfig..."),
            (Phase::CheckingRemoteFiles, "Checking remote files..."),
            (Phase::Uploading, "Uploading files..."),
            (Phase::DeletingRemoteFiles, "Deleting extra remote files..."),
        ];
        for (phase, label) in table {
            assert_eq!(phase.label(), label);
        }
        assert_eq!(WAITING_FOR_SERVER_THRESHOLD, Duration::from_secs(15));
    }

    #[test]
    fn a9_download_report_messages() {
        let err = EventError::new(ErrorKind::Io, "boom");
        assert_eq!(
            Outcome::Success.download_report_message().as_deref(),
            Some("Download finished successfully.")
        );
        assert_eq!(
            Outcome::CompletedWithErrors {
                errors: vec![err.clone()]
            }
            .download_report_message()
            .as_deref(),
            Some("Download finished with errors")
        );
        assert_eq!(
            Outcome::TooManyErrors {
                limit: 10,
                errors: vec![err.clone()]
            }
            .download_report_message()
            .as_deref(),
            Some("Download has been canceled due to too many errors (at least 10)")
        );
        assert_eq!(
            Outcome::Failed { error: err }
                .download_report_message()
                .as_deref(),
            Some("Download finished with error")
        );
        assert_eq!(Outcome::Cancelled.download_report_message(), None);
        assert!(Outcome::Success.is_success());
        assert!(!Outcome::Cancelled.is_success());
    }

    #[test]
    fn a9_transfer_percent() {
        let p = |transferred, total| TransferProgress {
            transferred,
            total,
            ..Default::default()
        };
        assert_eq!(p(10, 0).percent(), 0);
        assert_eq!(p(50, 200).percent(), 25);
        assert_eq!(p(2, 3).percent(), 66);
        assert_eq!(p(u64::MAX, u64::MAX).percent(), 100);
    }

    #[test]
    fn a9_percent_counter() {
        let mut c = PercentCounter::new(3);
        let out: Vec<_> = (0..3).filter_map(|_| c.increment()).collect();
        assert_eq!(out, [33, 66, 100]);

        let mut c = PercentCounter::new(1000);
        let out: Vec<_> = (0..1000).filter_map(|_| c.increment()).collect();
        assert_eq!(out, (0..=100).collect::<Vec<u8>>());

        let mut c = PercentCounter::new(0);
        assert!((0..10).all(|_| c.increment().is_none()));
    }

    #[test]
    fn a9_rate_limiter() {
        let base = Instant::now();
        let mut r = RateLimiter::new(Duration::from_millis(500));
        assert!(r.ready(base));
        assert!(!r.ready(base + Duration::from_millis(100)));
        assert!(!r.ready(base + Duration::from_millis(499)));
        assert!(r.ready(base + Duration::from_millis(500)));
        assert!(!r.ready(base + Duration::from_millis(900)));
        assert!(r.ready(base + Duration::from_millis(1000)));
        // A clock going backwards doesn't fire.
        assert!(!r.ready(base));
    }

    #[test]
    fn a9_channel_preserves_order_across_clones_and_threads() {
        let (sender, rx) = EventSender::channel();
        let clone = sender.clone();
        std::thread::Builder::new()
            .name("a9-emitter".to_owned())
            .spawn(move || {
                for index in 0..100 {
                    clone.emit(SyncEvent::LastIndexTransferred { index });
                }
            })
            .expect("spawn")
            .join()
            .expect("join");
        sender.emit(SyncEvent::ConnectionLost);
        drop(sender);
        let events: Vec<_> = rx.iter().collect();
        assert_eq!(events.len(), 101);
        for (i, event) in events[..100].iter().enumerate() {
            assert_eq!(event, &SyncEvent::LastIndexTransferred { index: i as u32 });
        }
        assert_eq!(events[100], SyncEvent::ConnectionLost);
    }

    #[tokio::test]
    async fn a9_tokio_channel_and_noop() {
        let (sender, mut rx) = EventSender::tokio_channel();
        assert!(!sender.is_noop());
        sender.emit(SyncEvent::Phase(Phase::Downloading));
        assert_eq!(rx.recv().await, Some(SyncEvent::Phase(Phase::Downloading)));
        drop(rx);
        // Emitting after the receiver is gone doesn't panic.
        sender.emit(SyncEvent::ConnectionLost);

        let (sender, rx) = EventSender::channel();
        drop(rx);
        sender.emit(SyncEvent::ConnectionLost);

        let noop = EventSender::noop();
        assert!(noop.is_noop());
        assert!(EventSender::default().is_noop());
        noop.emit(SyncEvent::ConnectionLost);
        assert_eq!(format!("{noop:?}"), "EventSender { noop: true }");
    }

    #[test]
    fn a9_from_fn_and_event_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen);
        let sender = EventSender::from_fn(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        sender.emit(SyncEvent::ConnectionLost);
        sender.clone().emit(SyncEvent::ConnectionLost);
        assert_eq!(seen.load(Ordering::Relaxed), 2);

        let err = EventError::new(ErrorKind::NotFound, "missing").with_path("@a/x.pbo");
        assert_eq!(err.relative_path.as_deref(), Some("@a/x.pbo"));
        assert_eq!(err.to_string(), "missing");
        let remote = EventError::from_remote(&RemoteError::MissingMetadata(A3sFile::Sync));
        assert_eq!(remote.kind, ErrorKind::NotFound);
        assert_eq!(remote.message, "Remote file no found: /.a3s/sync");
        assert_eq!(remote.relative_path.as_deref(), Some(".a3s/sync"));
    }

    #[test]
    fn a9_static_bounds() {
        fn assert_bounds<T: Clone + Send + Sync + 'static>() {}
        assert_bounds::<SyncEvent>();
        assert_bounds::<EventSender>();
        assert_bounds::<EventError>();
        assert_bounds::<Outcome>();
    }
}
