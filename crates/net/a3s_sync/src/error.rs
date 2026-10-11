//! Errors of remote metadata access.
//!
//! Replaces `exception/remote/*` and the `IOException` messages the Java DAOs build with
//! `ConnectionExceptionFactory` (ported to [`a3s_foundation::error::messages::connection`]).

use a3s_foundation::error::messages::{self, RemoteA3sFile};
use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_repo_files::A3sFile;
use a3s_types::{Protocol, ProtocolType};

use crate::remote::RemoteFile;

/// A failed remote operation. `Display` is the Java message shown to the user.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RemoteError {
    /// Connecting (or logging in) failed.
    #[error("{message}")]
    Connect {
        /// Java-style message (see [`RemoteError::connect`]).
        message: String,
        /// The transport error.
        #[source]
        source: a3s_transport::Error,
    },

    /// Fetching a file failed.
    #[error("{message}")]
    Fetch {
        /// Path of the file below the repository root, without a leading `/`.
        relative_path: String,
        /// Java-style message (see [`RemoteError::fetch`]).
        message: String,
        /// The transport error.
        #[source]
        source: a3s_transport::Error,
    },

    /// A downloaded metadata file could not be decoded.
    #[error("{message}")]
    Decode {
        /// The metadata file.
        file: A3sFile,
        /// `"Failed to read file: {name}\n{detail}"`, see [`RemoteError::decode`].
        message: String,
        /// The decoding error.
        #[source]
        source: a3s_repo_files::Error,
    },

    /// A metadata file required for synchronization is missing after a check.
    #[error("{}", messages::remote_file_not_found(remote_a3s_file(*.0)))]
    MissingMetadata(A3sFile),

    /// The operation was cancelled.
    #[error("Cancelled")]
    Cancelled,
}

impl RemoteError {
    /// A failed connection to the repository of `protocol`.
    ///
    /// A cancelled transport error becomes [`RemoteError::Cancelled`].
    // Mirrors dao/connection/ftp/FtpDAO.java:200-208, sftp/SftpDAO.java:178-183,
    // http/HttpDAO.java:204-212, webdav/WebdavDAO.java:211-216
    pub fn connect(protocol: &Protocol, source: a3s_transport::Error) -> Self {
        if matches!(source, a3s_transport::Error::Cancelled) {
            return Self::Cancelled;
        }
        // `ConnectionService.init` rejects other protocol types before any DAO exists.
        // Mirrors service/ConnectionService.java:85-86
        if !is_repository_protocol(protocol.protocol_type)
            && matches!(source, a3s_transport::Error::InvalidProtocol(_))
        {
            return Self::Connect {
                message: UNSUPPORTED_PROTOCOL.to_owned(),
                source,
            };
        }
        let core = connect_core(protocol);
        let kind = transport_kind(&source);
        let detail = match &source {
            a3s_transport::Error::Unauthorized => messages::WRONG_LOGIN_PASSWORD.to_owned(),
            // FTP and SFTP change into the repository folder while connecting.
            // Mirrors dao/connection/ftp/FtpDAO.java:186-190
            a3s_transport::Error::NotFound(_) if is_ftp_like(protocol.protocol_type) => {
                format!("Remote directory not found: {}", protocol.remote_path())
            }
            other => other.to_string(),
        };
        Self::Connect {
            message: messages::connection(&core, kind, Some(&detail)),
            source,
        }
    }

    /// A failed fetch of `file` from the repository of `protocol`.
    ///
    /// A cancelled transport error becomes [`RemoteError::Cancelled`].
    pub fn fetch(protocol: &Protocol, file: &RemoteFile, source: a3s_transport::Error) -> Self {
        if matches!(source, a3s_transport::Error::Cancelled) {
            return Self::Cancelled;
        }
        Self::Fetch {
            relative_path: file.transport_path(),
            message: fetch_message(protocol, file, &source),
            source,
        }
    }

    /// A downloaded metadata file that could not be decoded.
    // Mirrors dao/A3SFilesAccessor.java:55-74
    pub fn decode(file: A3sFile, source: a3s_repo_files::Error) -> Self {
        let mut message = format!("Failed to read file: {}\n{source}", file.file_name());
        if matches!(
            source,
            a3s_repo_files::Error::Gzip(_) | a3s_repo_files::Error::TooLarge(_)
        ) {
            message.push('\n');
            message.push_str(FILE_CORRUPTED);
        }
        Self::Decode {
            file,
            message,
            source,
        }
    }

    /// Classification of the error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Connect { source, .. } | Self::Fetch { source, .. } => transport_kind(source),
            Self::Decode { .. } => ErrorKind::InvalidData,
            Self::MissingMetadata(_) => ErrorKind::NotFound,
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    /// Whether a remote file is missing.
    pub fn is_not_found(&self) -> bool {
        self.kind() == ErrorKind::NotFound
    }

    /// Whether a new connection may help (Java: "connection lost").
    pub fn is_connection_lost(&self) -> bool {
        self.transport()
            .is_some_and(a3s_transport::Error::is_connection_lost)
    }

    /// The underlying transport error, for [`RemoteError::Connect`] and [`RemoteError::Fetch`].
    pub fn transport(&self) -> Option<&a3s_transport::Error> {
        match self {
            Self::Connect { source, .. } | Self::Fetch { source, .. } => Some(source),
            _ => None,
        }
    }

    /// Path of the affected file below the repository root, if known.
    pub fn relative_path(&self) -> Option<String> {
        match self {
            Self::Fetch { relative_path, .. } => Some(relative_path.clone()),
            Self::Decode { file, .. } | Self::MissingMetadata(file) => Some(file.relative_path()),
            Self::Connect { .. } | Self::Cancelled => None,
        }
    }
}

impl UserFacing for RemoteError {
    fn kind(&self) -> ErrorKind {
        Self::kind(self)
    }

    /// The Java message; it already contains the cause.
    fn user_message(&self) -> String {
        self.to_string()
    }
}

// Mirrors dao/A3SFilesAccessor.java:26
const FILE_CORRUPTED: &str = "The file seems to be corrupted.";

/// The `a3s_foundation::error` mirror of an [`A3sFile`].
pub(crate) fn remote_a3s_file(file: A3sFile) -> RemoteA3sFile {
    match file {
        A3sFile::Sync => RemoteA3sFile::Sync,
        A3sFile::ServerInfo => RemoteA3sFile::ServerInfo,
        A3sFile::Changelogs => RemoteA3sFile::Changelogs,
        A3sFile::AutoConfig => RemoteA3sFile::AutoConfig,
        A3sFile::Events => RemoteA3sFile::Events,
    }
}

// Mirrors service/ConnectionService.java:86
const UNSUPPORTED_PROTOCOL: &str = "Unknown or unsupported protocol.";

/// Whether a DAO exists for the protocol (`ConnectionService.init`).
fn is_repository_protocol(protocol_type: ProtocolType) -> bool {
    matches!(
        protocol_type,
        ProtocolType::Ftp
            | ProtocolType::Ftps
            | ProtocolType::Sftp
            | ProtocolType::Http
            | ProtocolType::Https
            | ProtocolType::HttpWebdav
            | ProtocolType::HttpsWebdav
    )
}

/// Whether the protocol is served by the FTP or SFTP DAO (as opposed to HTTP or WebDAV).
fn is_ftp_like(protocol_type: ProtocolType) -> bool {
    matches!(
        protocol_type,
        ProtocolType::Ftp | ProtocolType::Ftps | ProtocolType::Sftp
    )
}

/// Core message of a failed connection.
// Mirrors dao/connection/ftp/FtpDAO.java:204, sftp/SftpDAO.java:182-183, http/HttpDAO.java:208,
// webdav/WebdavDAO.java:213
fn connect_core(protocol: &Protocol) -> String {
    let host_url = a3s_types::local::protocol::host_url(protocol);
    match protocol.protocol_type {
        ProtocolType::Ftp | ProtocolType::Ftps => {
            format!("Failed to connect to the FTP server on url: {host_url}")
        }
        ProtocolType::Sftp => format!("Failed to connect to the SFTP server on url: {host_url}"),
        _ => format!("Failed to connect to the HTTP server on url: {host_url}"),
    }
}

/// The message of a failed fetch of `file`.
///
/// HTTP and WebDAV report the full URL. FTP and SFTP detect a missing file while connecting and
/// report the connection URL; any other FTP/SFTP failure is `"Failed to retrieve file: …"`.
// Mirrors dao/connection/http/HttpDAO.java:170-212, ftp/FtpDAO.java:186-208, ftp/FtpDAO.java:405-410,
// sftp/SftpDAO.java:172-183, sftp/SftpDAO.java:242-246
pub(crate) fn fetch_message(
    protocol: &Protocol,
    file: &RemoteFile,
    source: &a3s_transport::Error,
) -> String {
    let relative = file.relative_file_path();
    let core = if !is_ftp_like(protocol.protocol_type) {
        format!(
            "Failed to connect to the HTTP server on url: {}{relative}",
            a3s_types::local::protocol::host_url(protocol)
        )
    } else if matches!(source, a3s_transport::Error::NotFound(_)) {
        connect_core(protocol)
    } else {
        format!("Failed to retrieve file: {relative}")
    };
    let detail = match source {
        // Mirrors dao/connection/http/HttpDAO.java:176, ftp/FtpDAO.java:196
        a3s_transport::Error::NotFound(_) => format!("Remote file not found: {relative}"),
        // Mirrors dao/connection/http/HttpDAO.java:174, ftp/FtpDAO.java:164
        a3s_transport::Error::Unauthorized => messages::WRONG_LOGIN_PASSWORD.to_owned(),
        a3s_transport::Error::Incomplete { expected, actual } => {
            messages::incomplete_transfer(&relative, *actual, *expected)
        }
        other => other.to_string(),
    };
    messages::connection(&core, transport_kind(source), Some(&detail))
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
        E::Http(e) if e.is_connect() => ErrorKind::ConnectionFailed,
        E::Ftp(suppaftp::FtpError::ConnectionError(_)) | E::Ssh(_) | E::ConnectionPoisoned => {
            ErrorKind::ConnectionFailed
        }
        E::Io(e) => ErrorKind::from_io(e),
        E::Remote { .. }
        | E::HttpStatus { .. }
        | E::InsecureRedirect
        | E::Sftp(_)
        | E::Http(_)
        | E::Ftp(_) => ErrorKind::Io,
    }
}

#[cfg(test)]
mod tests {
    use a3s_foundation::error::ErrorKind;
    use a3s_types::{Protocol, ProtocolType};

    use super::*;

    fn http() -> Protocol {
        Protocol::new(
            ProtocolType::Http,
            "127.0.0.1/repo",
            "8080",
            "anonymous",
            "",
            false,
        )
    }

    fn ftp() -> Protocol {
        Protocol::new(
            ProtocolType::Ftp,
            "88.99.69.243/Repo/WOG",
            "21",
            "anonymous",
            "",
            false,
        )
    }

    fn sync_file() -> RemoteFile {
        RemoteFile::new("sync", ".a3s", false)
    }

    #[test]
    fn fetch_messages_http_core() {
        let err = RemoteError::fetch(
            &http(),
            &sync_file(),
            a3s_transport::Error::NotFound(".a3s/sync".to_owned()),
        );
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(err.is_not_found());
        assert_eq!(err.relative_path().as_deref(), Some(".a3s/sync"));

        let err = RemoteError::fetch(&http(), &sync_file(), a3s_transport::Error::Timeout);
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert!(err.is_connection_lost());

        let err = RemoteError::fetch(
            &http(),
            &sync_file(),
            a3s_transport::Error::Incomplete {
                expected: 20,
                actual: 10,
            },
        );
        assert_eq!(err.kind(), ErrorKind::IncompleteTransfer);

        let err = RemoteError::fetch(&http(), &sync_file(), a3s_transport::Error::Unauthorized);
        assert!(
            err.to_string()
                .ends_with("/repo/.a3s/sync\nlogin or password wrong or missing")
        );
        assert_eq!(err.kind(), ErrorKind::Unauthorized);
    }

    #[test]
    fn connect_messages_and_kinds() {
        let err = RemoteError::connect(
            &ftp(),
            a3s_transport::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
        );
        assert_eq!(err.kind(), ErrorKind::ConnectionFailed);
        assert!(err.transport().is_some());
        assert_eq!(err.relative_path(), None);

        let err = RemoteError::connect(
            &http(),
            a3s_transport::Error::InvalidProtocol("SOCKS5 cannot be used".to_owned()),
        );
        assert_eq!(err.kind(), ErrorKind::InvalidInput);

        let mut socks = http();
        socks.protocol_type = ProtocolType::Socks5;
        let err = RemoteError::connect(
            &socks,
            a3s_transport::Error::InvalidProtocol("SOCKS5 cannot be used".to_owned()),
        );
        assert_eq!(err.kind(), ErrorKind::InvalidInput);

        let err = RemoteError::connect(
            &ftp(),
            a3s_transport::Error::NotFound("/Repo/WOG".to_owned()),
        );
        assert_eq!(err.kind(), ErrorKind::NotFound);

        let err = RemoteError::connect(&ftp(), a3s_transport::Error::Unauthorized);
        assert!(
            err.to_string()
                .ends_with("\nlogin or password wrong or missing")
        );

        assert!(matches!(
            RemoteError::connect(&ftp(), a3s_transport::Error::Cancelled),
            RemoteError::Cancelled
        ));
        assert!(matches!(
            RemoteError::fetch(&ftp(), &sync_file(), a3s_transport::Error::Cancelled),
            RemoteError::Cancelled
        ));
        assert_eq!(RemoteError::Cancelled.kind(), ErrorKind::Cancelled);
    }

    #[test]
    fn transport_kind_table() {
        use a3s_transport::Error as E;
        let table = [
            (E::NotFound(String::new()), ErrorKind::NotFound),
            (E::Unauthorized, ErrorKind::Unauthorized),
            (E::Timeout, ErrorKind::Timeout),
            (E::Cancelled, ErrorKind::Cancelled),
            (
                E::Incomplete {
                    expected: 1,
                    actual: 0,
                },
                ErrorKind::IncompleteTransfer,
            ),
            (E::InvalidProtocol(String::new()), ErrorKind::InvalidInput),
            (
                E::InsecureTlsRefused(String::new()),
                ErrorKind::TlsValidation,
            ),
            (E::Tls(String::new()), ErrorKind::TlsValidation),
            (
                E::Unsupported {
                    protocol: "FTP",
                    operation: "x",
                },
                ErrorKind::Unsupported,
            ),
            (
                E::HttpStatus {
                    status: 500,
                    url: String::new(),
                },
                ErrorKind::Io,
            ),
            (E::InsecureRedirect, ErrorKind::Io),
            (
                E::Ftp(suppaftp::FtpError::ConnectionError(std::io::Error::other(
                    "x",
                ))),
                ErrorKind::ConnectionFailed,
            ),
            (E::Ftp(suppaftp::FtpError::BadResponse), ErrorKind::Io),
            (E::ConnectionPoisoned, ErrorKind::ConnectionFailed),
            (E::InvalidPath("..".to_owned()), ErrorKind::InvalidInput),
            (
                E::Remote {
                    operation: "MKD",
                    path: "a".to_owned(),
                    reason: "550".to_owned(),
                },
                ErrorKind::Io,
            ),
            (
                E::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
                ErrorKind::ConnectionFailed,
            ),
        ];
        for (err, kind) in table {
            assert_eq!(transport_kind(&err), kind, "{err:?}");
        }
    }

    #[test]
    fn decode_messages() {
        let gzip =
            a3s_repo_files::decode_a3s::<a3s_types::ServerInfo>(b"not gzip").expect_err("corrupt");
        let err = RemoteError::decode(A3sFile::Sync, gzip);
        let text = err.to_string();
        assert!(
            text.ends_with("\nThe file seems to be corrupted."),
            "{text}"
        );
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(!err.is_connection_lost());
        assert_eq!(err.relative_path().as_deref(), Some(".a3s/sync"));

        let other = a3s_repo_files::Error::Types(a3s_types::Error::NullValue("x"));
        let text = RemoteError::decode(A3sFile::Events, other).to_string();
        assert!(!text.ends_with("corrupted."), "{text}");
    }

    #[test]
    fn missing_metadata_texts() {
        assert_eq!(
            RemoteError::MissingMetadata(A3sFile::Events).kind(),
            ErrorKind::NotFound
        );
        assert!(!RemoteError::MissingMetadata(A3sFile::Events).is_connection_lost());
    }
}
