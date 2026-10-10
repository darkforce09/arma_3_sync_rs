/// Transport errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("remote file not found: {0}")]
    NotFound(String),

    #[error("wrong login or password")]
    Unauthorized,

    #[error("HTTP status {status} for {url}")]
    HttpStatus { status: u16, url: String },

    #[error("connection timed out")]
    Timeout,

    #[error("cancelled")]
    Cancelled,

    #[error("incomplete transfer: expected {expected} bytes, got {actual}")]
    Incomplete { expected: u64, actual: u64 },

    #[error("{0}")]
    InvalidProtocol(String),

    #[error(
        "SSL certificate validation is disabled for the non-local host '{0}'. Enable certificate \
         validation, or set A3S_ALLOW_INSECURE_SSL=1 for a controlled development test."
    )]
    InsecureTlsRefused(String),

    #[error("operation not supported by {protocol}: {operation}")]
    Unsupported {
        protocol: &'static str,
        operation: &'static str,
    },

    #[error("redirect from HTTPS to HTTP refused")]
    InsecureRedirect,

    #[error("HTTP: {0}")]
    Http(#[source] reqwest::Error),

    #[error("FTP: {0}")]
    Ftp(#[source] suppaftp::FtpError),

    #[error("SSH: {0}")]
    Ssh(#[source] russh::Error),

    #[error("SFTP: {0}")]
    Sftp(#[source] russh_sftp::client::error::Error),

    #[error("TLS: {0}")]
    Tls(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether retrying with a new connection may help (Java: "connection lost").
    pub fn is_connection_lost(&self) -> bool {
        match self {
            Self::Timeout | Self::Incomplete { .. } | Self::Ssh(_) => true,
            Self::Io(e) => !matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ),
            Self::Http(e) => e.is_timeout() || e.is_connect() || e.is_request() || e.is_body(),
            Self::Ftp(e) => matches!(
                e,
                suppaftp::FtpError::ConnectionError(_)
                    | suppaftp::FtpError::DataConnectionAlreadyOpen
            ),
            _ => false,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_redirect() {
            Self::InsecureRedirect
        } else {
            Self::Http(err)
        }
    }
}

impl From<suppaftp::FtpError> for Error {
    fn from(err: suppaftp::FtpError) -> Self {
        Self::Ftp(err)
    }
}

impl From<russh::Error> for Error {
    fn from(err: russh::Error) -> Self {
        Self::Ssh(err)
    }
}

impl From<russh_sftp::client::error::Error> for Error {
    fn from(err: russh_sftp::client::error::Error) -> Self {
        Self::Sftp(err)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
