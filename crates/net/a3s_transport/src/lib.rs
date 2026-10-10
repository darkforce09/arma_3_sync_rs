//! Async transports (HTTP(S), FTP(S), SFTP, WebDAV) for Arma3Sync repositories.
//!
//! [`Transport`] is one connection to a repository. Paths are relative to the repository root
//! (e.g. `.a3s/sync`, `@ace/addons/ace_main.pbo`); the protocol's remote path is prepended.

mod ctx;
mod error;
mod ftp;
pub mod http;
mod options;
pub mod path;
mod sftp;
mod tls;
mod webdav;

use std::ops::Range;
use std::path::Path;

use a3s_types::{Protocol, ProtocolType};

pub use ctx::{Throttle, TransferCtx};
pub use error::{Error, Result};
pub use options::{ProxyConfig, TransportOptions, is_local_test_host};
pub use tokio_util::sync::CancellationToken;

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

    /// Closes the connection politely.
    pub async fn close(self) {
        match self {
            Self::Ftp(t) => t.quit().await,
            Self::Sftp(t) => t.close().await,
            Self::Http(_) | Self::Webdav(_) => {}
        }
    }
}

/// Opens `dest` for writing at `offset`: truncates it to `offset` bytes (0 = new file) and
/// positions at the end. Creates parent directories.
pub(crate) async fn open_destination(dest: &Path, offset: u64) -> Result<tokio::fs::File> {
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
    Ok(file)
}
