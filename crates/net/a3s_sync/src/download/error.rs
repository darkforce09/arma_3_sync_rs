//! Download errors and the retry / error-class policy (B14, B15).

use std::io;
use std::path::{Path, PathBuf};

use a3s_error::{ErrorKind, UserFacing, messages};

use super::MAX_RESUME_RETRIES;
use crate::{EventError, RemoteError};

/// A failed download of one task, or of the whole run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DownloadError {
    /// A transport error, with the Java message of a failed fetch or connection.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// A local folder could not be created.
    // Mirrors exception/CreateDirectoryException.java:12-21
    #[error("{message}")]
    CreateDirectory {
        /// The folder.
        path: PathBuf,
        /// `"Cannot create directory …\nPlease checkout file access permissions."`
        message: String,
        /// The I/O error.
        #[source]
        source: io::Error,
    },

    /// A local file operation failed (finalizing rename, sync, …).
    #[error("{message}")]
    LocalIo {
        /// The file.
        path: PathBuf,
        /// The message.
        message: String,
        /// The I/O error.
        #[source]
        source: io::Error,
    },

    /// The transfer ended with the wrong size after every resume retry.
    // Mirrors exception/IncompleteFileTransferException.java:9-15
    #[error("{message}")]
    Incomplete {
        /// Absolute path of the local target.
        path: PathBuf,
        /// Expected size.
        expected: u64,
        /// Received size.
        actual: u64,
        /// `"Incompete file size transfer: …"` (Java typo kept).
        message: String,
    },

    /// zsync patching failed for a reason that is not a transport error.
    // Mirrors dao/connection/http/HttpDAO.java:513-518
    #[error("{message}")]
    Zsync {
        /// Tree path of the file.
        relative_path: String,
        /// `"Failed to retrieve file: /{relative_path}.zsync\n{detail}"`.
        message: String,
        /// The zsync error.
        #[source]
        source: a3s_zsync::Error,
    },

    /// A task whose local path would leave its destination folder (decision D-15).
    #[error("{message}")]
    UnsafePath {
        /// The rejected path.
        path: PathBuf,
        /// The message.
        message: String,
    },

    /// The run was cancelled.
    #[error("Cancelled")]
    Cancelled,
}

impl DownloadError {
    /// Classification of the error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Remote(e) => e.kind(),
            Self::CreateDirectory { source, .. } | Self::LocalIo { source, .. } => {
                ErrorKind::from_io(source)
            }
            Self::Incomplete { .. } => ErrorKind::IncompleteTransfer,
            Self::Zsync { source, .. } => match source {
                a3s_zsync::Error::Io(e) => ErrorKind::from_io(e),
                a3s_zsync::Error::Cancelled => ErrorKind::Cancelled,
                _ => ErrorKind::InvalidData,
            },
            Self::UnsafePath { .. } => ErrorKind::InvalidData,
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    /// Whether this is a connection-class error (Java `SocketException`, "connection lost").
    pub fn is_connection_lost(&self) -> bool {
        match self {
            Self::Remote(e) => e.transport().is_some_and(is_connection_class),
            _ => false,
        }
    }

    /// A [`DownloadError::CreateDirectory`] for `path`.
    pub(crate) fn create_directory(path: &Path, source: io::Error) -> Self {
        Self::CreateDirectory {
            path: path.to_owned(),
            message: messages::cannot_create_directory(&absolute(path)),
            source,
        }
    }

    /// A [`DownloadError::LocalIo`] for `path`.
    pub(crate) fn local_io(path: &Path, source: io::Error) -> Self {
        Self::LocalIo {
            path: path.to_owned(),
            message: format!(
                "Failed to write file: {}\n{source}",
                absolute(path).display()
            ),
            source,
        }
    }

    /// A [`DownloadError::Incomplete`] for the local target `path`.
    pub(crate) fn incomplete(path: &Path, expected: u64, actual: u64) -> Self {
        let path = absolute(path);
        Self::Incomplete {
            message: messages::incomplete_transfer(&path.to_string_lossy(), actual, expected),
            path,
            expected,
            actual,
        }
    }

    /// A [`DownloadError::Zsync`] for the tree path `relative_path`.
    // Mirrors dao/connection/http/HttpDAO.java:513-518
    pub(crate) fn zsync(relative_path: &str, source: a3s_zsync::Error) -> Self {
        let core = format!("Failed to retrieve file: /{relative_path}.zsync");
        let kind = match &source {
            a3s_zsync::Error::Io(e) => ErrorKind::from_io(e),
            _ => ErrorKind::InvalidData,
        };
        Self::Zsync {
            relative_path: relative_path.to_owned(),
            message: messages::connection(&core, kind, Some(&source.to_string())),
            source,
        }
    }

    /// A [`DownloadError::UnsafePath`].
    pub(crate) fn unsafe_path(path: &Path, reason: &str) -> Self {
        Self::UnsafePath {
            path: path.to_owned(),
            message: format!(
                "Refusing to write outside the destination folder: {} ({reason})",
                path.display()
            ),
        }
    }
}

impl UserFacing for DownloadError {
    fn kind(&self) -> ErrorKind {
        Self::kind(self)
    }

    /// The Java message; it already contains the cause.
    fn user_message(&self) -> String {
        self.to_string()
    }
}

/// A failed task.
#[derive(Debug)]
pub struct FileError {
    /// Tree path of the task.
    pub relative_path: String,

    /// The error.
    pub error: DownloadError,
}

impl FileError {
    /// The cloneable projection carried by events and outcomes.
    pub fn to_event_error(&self) -> EventError {
        EventError::new(self.error.kind(), self.error.to_string()).with_path(&self.relative_path)
    }
}

/// `std::path::absolute`, or the path itself when it cannot be made absolute.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// Whether an I/O error is about the local file system rather than the network.
pub(crate) fn is_local_io(err: &io::Error) -> bool {
    use io::ErrorKind as K;
    matches!(
        err.kind(),
        K::StorageFull
            | K::ReadOnlyFilesystem
            | K::QuotaExceeded
            | K::FileTooLarge
            | K::IsADirectory
            | K::NotADirectory
            | K::InvalidFilename
            | K::PermissionDenied
            | K::NotFound
            | K::AlreadyExists
            | K::DirectoryNotEmpty
            | K::CrossesDevices
            | K::ResourceBusy
            | K::ExecutableFileBusy
    )
}

/// B15: a transport error that a new connection may fix, excluding incomplete transfers and
/// local file-system errors.
// Mirrors service/connection/ConnectionDownloadProcessor.java:116-117
pub(crate) fn is_connection_class(err: &a3s_transport::Error) -> bool {
    match err {
        a3s_transport::Error::Incomplete { .. } => false,
        a3s_transport::Error::Io(e) if is_local_io(e) => false,
        other => other.is_connection_lost(),
    }
}

/// What to do after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// Try again (resuming), after reconnecting the transport if `reconnect`.
    Retry {
        /// Replace the worker's transport first.
        reconnect: bool,
    },
    /// Stop the run: the connection is lost.
    ConnectionLost,
    /// Record a file error and continue with the next task.
    FileError,
    /// The run was cancelled.
    Cancelled,
}

/// Classifies a failed attempt that received `received` bytes. `reused` is true when the
/// transport had already completed a file (and the idle reconnect is still available).
// Mirrors service/connection/ConnectionDownloadProcessor.java:106-121,
// dao/connection/AbstractConnexionDAO.java:308-317 (with DV4)
pub(crate) fn classify(err: &a3s_transport::Error, received: u64, reused: bool) -> Attempt {
    match err {
        a3s_transport::Error::Cancelled => Attempt::Cancelled,
        a3s_transport::Error::Incomplete { .. } if received > 0 => {
            Attempt::Retry { reconnect: false }
        }
        e if is_connection_class(e) => {
            if received > 0 || reused {
                Attempt::Retry { reconnect: true }
            } else {
                Attempt::ConnectionLost
            }
        }
        _ => Attempt::FileError,
    }
}

/// Bounded resume retries of one task (B14, DV4).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RetryState {
    retries: u32,
    idle_reconnect_used: bool,
}

impl RetryState {
    /// [`classify`] with the per-task limits: at most [`MAX_RESUME_RETRIES`] retries, and one
    /// reconnect without received bytes on a reused transport.
    pub(crate) fn decide(
        &mut self,
        err: &a3s_transport::Error,
        received: u64,
        transport_reused: bool,
    ) -> Attempt {
        let reused = transport_reused && !self.idle_reconnect_used;
        match classify(err, received, reused) {
            Attempt::Retry { reconnect } => {
                if self.retries >= MAX_RESUME_RETRIES {
                    return if is_connection_class(err) {
                        Attempt::ConnectionLost
                    } else {
                        Attempt::FileError
                    };
                }
                self.retries += 1;
                if received == 0 {
                    self.idle_reconnect_used = true;
                }
                Attempt::Retry { reconnect }
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use a3s_transport::Error as E;

    use super::*;

    fn io(kind: io::ErrorKind) -> E {
        E::Io(io::Error::from(kind))
    }

    /// A real reqwest connect error, from a port nobody listens on.
    async fn http_connect_error() -> E {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let protocol = a3s_types::Protocol::new(
            a3s_types::ProtocolType::Http,
            "127.0.0.1",
            port.to_string(),
            "anonymous",
            "",
            false,
        );
        let options = a3s_transport::TransportOptions::from_protocol(&protocol);
        let mut transport = a3s_transport::Transport::connect(&protocol, &options)
            .await
            .expect("HTTP connect does no I/O");
        transport
            .fetch("x", &a3s_transport::TransferCtx::new())
            .await
            .expect_err("closed port")
    }

    #[tokio::test]
    async fn a1_classify_table() {
        let incomplete = || E::Incomplete {
            expected: 10,
            actual: 5,
        };
        assert_eq!(
            classify(&incomplete(), 5, false),
            Attempt::Retry { reconnect: false }
        );
        assert_eq!(classify(&incomplete(), 0, false), Attempt::FileError);
        assert_eq!(classify(&incomplete(), 0, true), Attempt::FileError);

        let connect = http_connect_error().await;
        assert!(
            matches!(&connect, E::Http(e) if e.is_connect()),
            "{connect:?}"
        );
        assert_eq!(classify(&connect, 0, false), Attempt::ConnectionLost);
        assert_eq!(
            classify(&connect, 0, true),
            Attempt::Retry { reconnect: true }
        );
        // Reused transport: one reconnect, then connection lost.
        let mut state = RetryState::default();
        assert_eq!(
            state.decide(&connect, 0, true),
            Attempt::Retry { reconnect: true }
        );
        assert_eq!(state.decide(&connect, 0, true), Attempt::ConnectionLost);

        assert_eq!(
            classify(&E::Timeout, 1, false),
            Attempt::Retry { reconnect: true }
        );
        assert_eq!(classify(&E::Timeout, 0, false), Attempt::ConnectionLost);

        for kind in [
            io::ErrorKind::StorageFull,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem,
        ] {
            assert_eq!(
                classify(&io(kind), 100, true),
                Attempt::FileError,
                "{kind:?}"
            );
        }
        assert!(is_connection_class(&io(io::ErrorKind::ConnectionReset)));
        assert_eq!(
            classify(&io(io::ErrorKind::ConnectionReset), 0, false),
            Attempt::ConnectionLost
        );

        assert_eq!(
            classify(&E::NotFound("x".to_owned()), 0, false),
            Attempt::FileError
        );
        assert_eq!(classify(&E::Unauthorized, 0, true), Attempt::FileError);
        assert_eq!(
            classify(
                &E::HttpStatus {
                    status: 416,
                    url: String::new()
                },
                0,
                false
            ),
            Attempt::FileError
        );
        assert_eq!(classify(&E::Cancelled, 10, true), Attempt::Cancelled);
    }

    #[test]
    fn a1_retry_counter_is_bounded() {
        let incomplete = E::Incomplete {
            expected: 10,
            actual: 5,
        };
        let mut state = RetryState::default();
        for _ in 0..MAX_RESUME_RETRIES {
            assert_eq!(
                state.decide(&incomplete, 1, false),
                Attempt::Retry { reconnect: false }
            );
        }
        assert_eq!(state.decide(&incomplete, 1, false), Attempt::FileError);

        let mut state = RetryState::default();
        for _ in 0..MAX_RESUME_RETRIES {
            assert_eq!(
                state.decide(&E::Timeout, 1, false),
                Attempt::Retry { reconnect: true }
            );
        }
        assert_eq!(state.decide(&E::Timeout, 1, false), Attempt::ConnectionLost);
    }

    #[test]
    fn a1_error_kinds_and_messages() {
        let err = DownloadError::incomplete(Path::new("/d/x.pbo"), 100, 40);
        assert_eq!(
            err.to_string(),
            "Incompete file size transfer: /d/x.pbo Remote size: 100 Bytes, Actual size: 40 Bytes"
        );
        assert_eq!(err.kind(), ErrorKind::IncompleteTransfer);
        assert!(!err.is_connection_lost());

        let err = DownloadError::create_directory(
            Path::new("/d/@a"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        );
        assert_eq!(
            err.to_string(),
            "Cannot create directory /d/@a\nPlease checkout file access permissions."
        );
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        let err = DownloadError::zsync(
            "@a/x.pbo",
            a3s_zsync::Error::InvalidControlFile("bad".to_owned()),
        );
        assert_eq!(
            err.to_string(),
            "Failed to retrieve file: /@a/x.pbo.zsync\ninvalid .zsync file: bad"
        );
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert_eq!(DownloadError::Cancelled.kind(), ErrorKind::Cancelled);

        let file = FileError {
            relative_path: "@a/x.pbo".to_owned(),
            error: DownloadError::incomplete(Path::new("/d/x.pbo"), 2, 1),
        };
        let event = file.to_event_error();
        assert_eq!(event.relative_path.as_deref(), Some("@a/x.pbo"));
        assert_eq!(event.kind, ErrorKind::IncompleteTransfer);
    }

    #[test]
    fn a16_download_error_bounds() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<DownloadError>();
        assert_bounds::<FileError>();
    }
}
