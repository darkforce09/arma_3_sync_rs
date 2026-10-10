//! WebDAV transport (`dao/connection/webdav/WebdavDAO.java`). Downloads are plain GETs;
//! the WebDAV verbs are used for uploads.

use std::path::Path;

use a3s_types::Protocol;

use crate::Result;
use crate::ctx::TransferCtx;
use crate::http::HttpTransport;
use crate::options::TransportOptions;

pub struct WebdavTransport {
    http: HttpTransport,
}

impl WebdavTransport {
    pub fn new(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        Ok(Self {
            http: HttpTransport::new(protocol, options)?,
        })
    }

    pub async fn fetch(&mut self, relative: &str, ctx: &TransferCtx) -> Result<Vec<u8>> {
        self.http.fetch(relative, ctx).await
    }

    /// WebDAV downloads never resume (Java always overwrites).
    pub async fn download_file(
        &mut self,
        relative: &str,
        dest: &Path,
        _resume_from: u64,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        self.http.download_file(relative, dest, 0, ctx).await
    }
}
