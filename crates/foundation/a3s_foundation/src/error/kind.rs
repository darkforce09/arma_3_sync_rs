use std::io;

/// Classification of an error, mapped from the Java exception hierarchy.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// A local or remote file, or a repository, does not exist (`FileNotFoundException`,
    /// `Remote*FileNotFoundException`, `*FileNotFound*`, `RepositoryNotFoundException`).
    NotFound,

    /// The connection could not be established (`UnknownHostException`, `SocketException`).
    ConnectionFailed,

    /// The server did not accept the connection in time.
    ConnectTimeout,

    /// The server stopped sending data.
    ReadTimeout,

    /// A timeout that is neither clearly a connect nor a read timeout.
    Timeout,

    /// Wrong or missing login or password.
    Unauthorized,

    /// The TLS certificate could not be validated.
    TlsValidation,

    /// A transfer ended before the expected size was reached
    /// (`IncompleteFileTransferException`).
    IncompleteTransfer,

    /// A directory could not be created or deleted (`CreateDirectoryException`,
    /// `DeleteDirectoryException`), or access was denied.
    PermissionDenied,

    /// A file could not be decoded (`LoadingException`, `RepositoryServerInfoReadException`).
    InvalidData,

    /// A file could not be written (`WritingException`).
    WriteFailed,

    /// Invalid user input or settings (`CheckException`, `ProfileException`, empty main or
    /// default folder).
    InvalidInput,

    /// The game could not be launched (`LaunchException`).
    Launch,

    /// The operation was cancelled by the user.
    Cancelled,

    /// The protocol does not support the operation.
    Unsupported,

    /// Any other I/O error.
    Io,

    /// Anything else: an unexpected error.
    Internal,
}

impl ErrorKind {
    /// Maps an [`io::Error`] to a kind by its [`io::ErrorKind`].
    pub fn from_io(err: &io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::TimedOut => Self::Timeout,
            io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
            | io::ErrorKind::AddrNotAvailable => Self::ConnectionFailed,
            io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => Self::InvalidData,
            io::ErrorKind::WriteZero | io::ErrorKind::StorageFull => Self::WriteFailed,
            io::ErrorKind::Unsupported => Self::Unsupported,
            _ => Self::Io,
        }
    }

    /// Whether the error is expected and shown as a plain message (everything but
    /// [`Self::Internal`]).
    // Mirrors console/CommandGeneral.java:115-119
    pub fn is_managed(self) -> bool {
        self != Self::Internal
    }

    /// Whether the kind is one of the three timeout kinds.
    pub fn is_timeout(self) -> bool {
        matches!(
            self,
            Self::ConnectTimeout | Self::ReadTimeout | Self::Timeout
        )
    }
}

/// Classifies an arbitrary error by walking its source chain: the first [`io::Error`] decides
/// (see [`ErrorKind::from_io`]); without one the error is [`ErrorKind::Internal`].
///
/// Crates implementing [`crate::error::UserFacing`] call [`crate::error::UserFacing::kind`] directly instead.
pub fn kind_of(err: &(dyn std::error::Error + 'static)) -> ErrorKind {
    let mut current = Some(err);
    while let Some(e) = current {
        if let Some(io) = e.downcast_ref::<io::Error>() {
            return ErrorKind::from_io(io);
        }
        current = e.source();
    }
    ErrorKind::Internal
}

/// Classifies a timeout by the message of the underlying error.
///
/// `read` is tested before `connect`, as in Java.
// Mirrors exception/ConnectionExceptionFactory.java:36-43
pub fn classify_timeout(message: Option<&str>) -> ErrorKind {
    let Some(message) = message else {
        return ErrorKind::Timeout;
    };
    let lower = message.to_lowercase();
    if lower.contains("read") {
        ErrorKind::ReadTimeout
    } else if lower.contains("connect") {
        ErrorKind::ConnectTimeout
    } else {
        ErrorKind::Timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_e3_classify_timeout() {
        assert_eq!(
            classify_timeout(Some("Read timed out")),
            ErrorKind::ReadTimeout
        );
        assert_eq!(
            classify_timeout(Some("connect timed out")),
            ErrorKind::ConnectTimeout
        );
        assert_eq!(
            classify_timeout(Some("Read ... connect")),
            ErrorKind::ReadTimeout
        );
        assert_eq!(classify_timeout(Some("CONNECT")), ErrorKind::ConnectTimeout);
        assert_eq!(classify_timeout(Some("timed out")), ErrorKind::Timeout);
        assert_eq!(classify_timeout(None), ErrorKind::Timeout);
    }

    #[test]
    fn a_e4_from_io_table() {
        use io::ErrorKind as K;
        let table = [
            (K::NotFound, ErrorKind::NotFound),
            (K::PermissionDenied, ErrorKind::PermissionDenied),
            (K::TimedOut, ErrorKind::Timeout),
            (K::ConnectionRefused, ErrorKind::ConnectionFailed),
            (K::ConnectionReset, ErrorKind::ConnectionFailed),
            (K::ConnectionAborted, ErrorKind::ConnectionFailed),
            (K::NotConnected, ErrorKind::ConnectionFailed),
            (K::HostUnreachable, ErrorKind::ConnectionFailed),
            (K::NetworkUnreachable, ErrorKind::ConnectionFailed),
            (K::AddrNotAvailable, ErrorKind::ConnectionFailed),
            (K::InvalidData, ErrorKind::InvalidData),
            (K::UnexpectedEof, ErrorKind::InvalidData),
            (K::WriteZero, ErrorKind::WriteFailed),
            (K::StorageFull, ErrorKind::WriteFailed),
            (K::Unsupported, ErrorKind::Unsupported),
            (K::AlreadyExists, ErrorKind::Io),
            (K::Other, ErrorKind::Io),
        ];
        for (io_kind, expected) in table {
            assert_eq!(
                ErrorKind::from_io(&io::Error::from(io_kind)),
                expected,
                "{io_kind:?}"
            );
        }
    }

    #[derive(Debug, thiserror::Error)]
    enum Wrapper {
        #[error("loading failed")]
        Load(#[source] io::Error),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("custom")]
    struct Custom;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Wrapper);

    #[test]
    fn a_e4_kind_of_walks_the_chain() {
        let err = Wrapper::Load(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(kind_of(&err), ErrorKind::NotFound);
        let nested = Outer(Wrapper::Load(io::Error::from(
            io::ErrorKind::PermissionDenied,
        )));
        assert_eq!(kind_of(&nested), ErrorKind::PermissionDenied);
        assert_eq!(kind_of(&Custom), ErrorKind::Internal);
    }

    #[test]
    fn managed_and_timeout_predicates() {
        assert!(ErrorKind::NotFound.is_managed());
        assert!(ErrorKind::Io.is_managed());
        assert!(!ErrorKind::Internal.is_managed());
        assert!(ErrorKind::ReadTimeout.is_timeout());
        assert!(ErrorKind::ConnectTimeout.is_timeout());
        assert!(ErrorKind::Timeout.is_timeout());
        assert!(!ErrorKind::ConnectionFailed.is_timeout());
    }
}
