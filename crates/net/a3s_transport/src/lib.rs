//! Async transports (HTTP(S), FTP(S), SFTP, WebDAV) for Arma3Sync repositories.
//!
//! [`Transport`] is one connection to a repository. Paths are relative to the repository root
//! (e.g. `.a3s/sync`, `@ace/addons/ace_main.pbo`); the protocol's remote path is prepended.
//!
//! Besides downloads, FTP(S), SFTP and WebDAV transports can write to a repository
//! ([`Transport::upload_file`], [`Transport::upload_bytes`], [`Transport::mkdirs`],
//! [`Transport::delete`]); [`Transport::exists`] works on every protocol. Every path is checked
//! with [`path::validate_relative`] before any network I/O. An [`UploadSession`] enables
//! directory and listing caches for a batch of operations.

mod ctx;
mod error;
mod ftp;
pub mod http;
mod options;
pub mod path;
pub mod proxy;
mod session;
mod sftp;
mod tls;
mod webdav;

use std::ops::Range;
use std::path::Path;

use a3s_types::{Protocol, ProtocolType};
use tokio::io::AsyncRead;

use crate::session::SessionState;

pub use ctx::{StartOffset, Throttle, TransferCtx};
pub use error::{Error, Result};
pub use options::{ProxyConfig, TransportOptions};
pub use session::{UploadSession, UploadSessionStats};
pub use tokio_util::sync::CancellationToken;

/// Kind of a remote entry for [`Transport::delete`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteKind {
    /// A single file.
    File,

    /// A directory, deleted recursively.
    Directory,
}

/// Bytes to upload: a local file or an in-memory buffer, with its length.
pub(crate) struct UploadSource {
    pub(crate) reader: Box<dyn AsyncRead + Send + Unpin>,
    pub(crate) len: u64,
}

impl UploadSource {
    async fn file(local: &Path) -> Result<Self> {
        let file = tokio::fs::File::open(local).await?;
        let len = file.metadata().await?.len();
        Ok(Self {
            reader: Box::new(file),
            len,
        })
    }

    fn bytes(bytes: &[u8]) -> Self {
        Self {
            reader: Box::new(std::io::Cursor::new(bytes.to_vec())),
            len: bytes.len() as u64,
        }
    }
}

/// One connection to a repository.
pub enum Transport {
    Http(http::HttpTransport),
    Ftp(ftp::FtpTransport),
    Sftp(sftp::SftpTransport),
    Webdav(webdav::WebdavTransport),
}

impl Transport {
    /// Connects (and logs in, for FTP and SFTP).
    pub async fn connect(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        Ok(match protocol.protocol_type {
            ProtocolType::Http | ProtocolType::Https => {
                Self::Http(http::HttpTransport::new(protocol, options)?)
            }
            ProtocolType::Ftp | ProtocolType::Ftps => {
                Self::Ftp(ftp::FtpTransport::connect(protocol, options).await?)
            }
            ProtocolType::Sftp => {
                Self::Sftp(sftp::SftpTransport::connect(protocol, options).await?)
            }
            ProtocolType::HttpWebdav | ProtocolType::HttpsWebdav => {
                Self::Webdav(webdav::WebdavTransport::new(protocol, options)?)
            }
            other => {
                return Err(Error::InvalidProtocol(format!(
                    "{} cannot be used for repositories",
                    other.description()
                )));
            }
        })
    }

    pub fn protocol_name(&self) -> &'static str {
        match self {
            Self::Http(_) => "HTTP",
            Self::Ftp(_) => "FTP",
            Self::Sftp(_) => "SFTP",
            Self::Webdav(_) => "WebDAV",
        }
    }

    /// Downloads a whole (small) file into memory, e.g. `.a3s/sync`.
    pub async fn fetch(&mut self, relative: &str, ctx: &TransferCtx) -> Result<Vec<u8>> {
        path::validate_relative(relative)?;
        match self {
            Self::Http(t) => t.fetch(relative, ctx).await,
            Self::Ftp(t) => t.fetch(relative, ctx).await,
            Self::Sftp(t) => t.fetch(relative, ctx).await,
            Self::Webdav(t) => t.fetch(relative, ctx).await,
        }
    }

    /// Downloads to `dest`, appending from `resume_from` when the protocol supports it.
    ///
    /// Returns the final size of `dest`.
    pub async fn download_file(
        &mut self,
        relative: &str,
        dest: &Path,
        resume_from: u64,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        ctx.check_cancelled()?;
        path::validate_relative(relative)?;
        match self {
            Self::Http(t) => t.download_file(relative, dest, resume_from, ctx).await,
            Self::Ftp(t) => t.download_file(relative, dest, resume_from, ctx).await,
            Self::Sftp(t) => t.download_file(relative, dest, resume_from, ctx).await,
            Self::Webdav(t) => t.download_file(relative, dest, resume_from, ctx).await,
        }
    }

    /// Downloads a byte range (HTTP only; used for zsync patching).
    pub async fn fetch_range(
        &mut self,
        relative: &str,
        range: Range<u64>,
        ctx: &TransferCtx,
    ) -> Result<Vec<u8>> {
        path::validate_relative(relative)?;
        match self {
            Self::Http(t) => t.fetch_range(relative, range, ctx).await,
            _ => Err(Error::Unsupported {
                protocol: self.protocol_name(),
                operation: "range requests",
            }),
        }
    }

    /// Whether zsync partial transfer can be used (HTTP with working range requests).
    pub async fn supports_partial_transfer(&mut self, ctx: &TransferCtx) -> bool {
        match self {
            Self::Http(t) => t.supports_ranges(ctx).await,
            _ => false,
        }
    }

    /// Whether the protocol supports write operations (`false` for HTTP).
    pub fn supports_upload(&self) -> bool {
        !matches!(self, Self::Http(_))
    }

    /// Whether a remote **file** exists. A missing parent directory yields `Ok(false)`.
    // Mirrors dao/connection/AbstractConnexionDAO.java:362-391.
    pub async fn exists(&mut self, relative: &str, ctx: &TransferCtx) -> Result<bool> {
        ctx.check_cancelled()?;
        let relative = non_root(relative)?;
        match self {
            Self::Http(t) => t.exists(&relative, ctx).await,
            Self::Ftp(t) => t.exists(&relative, ctx).await,
            Self::Sftp(t) => t.exists(&relative, ctx).await,
            Self::Webdav(t) => t.exists(&relative, ctx).await,
        }
    }

    /// Uploads `local` to `relative`, creating missing parent directories and overwriting an
    /// existing remote file. Returns the number of bytes sent.
    // Mirrors dao/connection/AbstractConnexionDAO.java:393-425.
    pub async fn upload_file(
        &mut self,
        local: &Path,
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        ctx.check_cancelled()?;
        let relative = non_root(relative)?;
        self.require_upload("upload")?;
        let source = UploadSource::file(local).await?;
        self.upload(source, &relative, ctx).await
    }

    /// Uploads an in-memory buffer verbatim (e.g. `encode_a3s` output to `.a3s/sync`),
    /// creating missing parent directories.
    // Mirrors dao/connection/AbstractConnexionDAO.java:427-449.
    pub async fn upload_bytes(
        &mut self,
        bytes: &[u8],
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<()> {
        ctx.check_cancelled()?;
        let relative = non_root(relative)?;
        self.require_upload("upload")?;
        self.upload(UploadSource::bytes(bytes), &relative, ctx)
            .await
            .map(|_| ())
    }

    async fn upload(
        &mut self,
        source: UploadSource,
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        match self {
            Self::Http(_) => Err(self.unsupported("upload")),
            Self::Ftp(t) => t.upload(source, relative, ctx).await,
            Self::Sftp(t) => t.upload(source, relative, ctx).await,
            Self::Webdav(t) => t.upload(source, relative, ctx).await,
        }
    }

    /// Creates `relative_dir` and its missing parents below the base directory. Existing
    /// directories are not an error; `""` is a no-op.
    pub async fn mkdirs(&mut self, relative_dir: &str, ctx: &TransferCtx) -> Result<()> {
        ctx.check_cancelled()?;
        let relative = path::normalize_relative(relative_dir)?;
        self.require_upload("mkdir")?;
        if relative.is_empty() {
            return Ok(());
        }
        match self {
            Self::Http(_) => Err(self.unsupported("mkdir")),
            Self::Ftp(t) => t.mkdirs(&relative, ctx).await,
            Self::Sftp(t) => t.mkdirs(&relative, ctx).await,
            Self::Webdav(t) => t.mkdirs(&relative, ctx).await,
        }
    }

    /// Deletes a file, or a directory recursively. A missing target is `Ok(())`.
    // Mirrors dao/connection/AbstractConnexionDAO.java:451-469.
    pub async fn delete(
        &mut self,
        relative: &str,
        kind: RemoteKind,
        ctx: &TransferCtx,
    ) -> Result<()> {
        ctx.check_cancelled()?;
        let relative = non_root(relative)?;
        self.require_upload("delete")?;
        match self {
            Self::Http(_) => Err(self.unsupported("delete")),
            Self::Ftp(t) => t.delete(&relative, kind, ctx).await,
            Self::Sftp(t) => t.delete(&relative, kind, ctx).await,
            Self::Webdav(t) => t.delete(&relative, kind, ctx).await,
        }
    }

    /// Starts an upload session: directory (and, for FTP, listing) caches stay enabled until
    /// the returned guard is dropped.
    pub fn upload_session(&mut self) -> UploadSession<'_> {
        UploadSession::new(self)
    }

    /// Counters of the current or last upload session.
    pub fn upload_session_stats(&self) -> UploadSessionStats {
        self.session_state().stats()
    }

    pub(crate) fn session_state(&self) -> &SessionState {
        match self {
            Self::Http(t) => &t.session,
            Self::Ftp(t) => &t.session,
            Self::Sftp(t) => &t.session,
            Self::Webdav(t) => &t.session,
        }
    }

    pub(crate) fn session_state_mut(&mut self) -> &mut SessionState {
        match self {
            Self::Http(t) => &mut t.session,
            Self::Ftp(t) => &mut t.session,
            Self::Sftp(t) => &mut t.session,
            Self::Webdav(t) => &mut t.session,
        }
    }

    fn unsupported(&self, operation: &'static str) -> Error {
        Error::Unsupported {
            protocol: self.protocol_name(),
            operation,
        }
    }

    // Mirrors dao/connection/http/HttpDAO.java:415-428.
    fn require_upload(&self, operation: &'static str) -> Result<()> {
        if self.supports_upload() {
            Ok(())
        } else {
            Err(self.unsupported(operation))
        }
    }

    /// Closes the connection politely.
    pub async fn close(self) {
        match self {
            Self::Ftp(t) => t.quit().await,
            Self::Sftp(t) => t.close().await,
            Self::Http(_) | Self::Webdav(_) => {}
        }
    }
}

/// Validates `relative` and rejects the repository root: deleting, uploading to or checking
/// the root itself is never a valid request.
fn non_root(relative: &str) -> Result<String> {
    let normalized = path::normalize_relative(relative)?;
    if normalized.is_empty() {
        return Err(Error::InvalidPath(relative.to_owned()));
    }
    Ok(normalized)
}

/// Opens `dest` for writing at `offset`: truncates it to `offset` bytes (0 = new file) and
/// positions at the end. Creates parent directories. Records `offset` in `ctx.start_offset`.
pub(crate) async fn open_destination(
    dest: &Path,
    offset: u64,
    ctx: &TransferCtx,
) -> Result<tokio::fs::File> {
    use tokio::io::AsyncSeekExt as _;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(offset == 0)
        .open(dest)
        .await?;
    if offset > 0 {
        file.set_len(offset).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
    }
    ctx.start_offset.set(offset);
    Ok(file)
}
