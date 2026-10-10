//! WebDAV transport (`dao/connection/webdav/WebdavDAO.java`). Downloads are plain GETs;
//! the WebDAV verbs are used for uploads.

use std::path::Path;
use std::sync::atomic::Ordering;

use a3s_types::Protocol;
use reqwest::header::{self, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use tokio::io::AsyncReadExt as _;

use crate::ctx::{COPY_BUFFER, TransferCtx};
use crate::http::{HttpTransport, check_status, url_for};
use crate::options::TransportOptions;
use crate::session::SessionState;
use crate::{Error, RemoteKind, Result, UploadSource, path};

pub struct WebdavTransport {
    http: HttpTransport,

    /// Client for the write verbs (`PUT`, `MKCOL`, `DELETE`, `PROPFIND`): never follows
    /// redirects.
    write_client: reqwest::Client,
    pub(crate) session: SessionState,
}

fn method(name: &'static str) -> Method {
    Method::from_bytes(name.as_bytes()).expect("WebDAV method names are valid tokens")
}

impl WebdavTransport {
    pub fn new(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        Ok(Self {
            http: HttpTransport::new(protocol, options)?,
            write_client: crate::http::build_client(protocol, options, false)?,
            session: SessionState::default(),
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

    fn file_url(&self, relative: &str) -> Url {
        self.http.url(relative)
    }

    /// Collection URL: always `/`-terminated, or servers answer 301.
    // Mirrors dao/connection/RemoteFile.java:58-62.
    fn dir_url(&self, relative: &str) -> Url {
        let mut url = url_for(
            &self.http.base,
            &path::join_remote(&self.http.remote_path, relative),
        );
        if let Ok(mut segments) = url.path_segments_mut() {
            segments.pop_if_empty().push("");
        }
        url
    }

    async fn send_write(
        &self,
        method: Method,
        url: Url,
        ctx: &TransferCtx,
        configure: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response> {
        let req = configure(self.http.request(&self.write_client, method, url));
        HttpTransport::send_cancellable(req, ctx).await
    }

    /// `HEAD` on the file: 2xx → `true`, 404/410 → `false`.
    // Mirrors dao/connection/webdav/WebdavDAO.java:249-277 (Sardine `exists` = HEAD).
    pub async fn exists(&self, relative: &str, ctx: &TransferCtx) -> Result<bool> {
        let url = self.file_url(relative);
        let head = self
            .http
            .request(&self.http.client, Method::HEAD, url.clone());
        let response = HttpTransport::send_cancellable(head, ctx).await?;
        let status = response.status();
        if status.is_success() {
            Ok(true)
        } else if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
            Ok(false)
        } else {
            check_status(status, &url, relative).map(|()| false)
        }
    }

    /// Creates every missing collection below the remote path, parents first.
    // Mirrors dao/connection/webdav/WebdavDAO.java:436-451 (PROPFIND instead of HEAD).
    pub async fn mkdirs(&mut self, relative_dir: &str, ctx: &TransferCtx) -> Result<()> {
        let mut current = String::new();
        for segment in path::segments(relative_dir) {
            let parent = std::mem::take(&mut current);
            current = path::join_relative(&parent, segment);
            if self.session.is_known_dir(&current) {
                continue;
            }
            ctx.check_cancelled()?;
            let url = self.dir_url(&current);
            let response = self
                .send_write(method("PROPFIND"), url.clone(), ctx, |req| {
                    req.header("Depth", "0")
                })
                .await?;
            match response.status() {
                StatusCode::MULTI_STATUS | StatusCode::OK => {
                    self.session.mark_known_dir(&current);
                    continue;
                }
                StatusCode::NOT_FOUND => {}
                other => check_status(other, &url, &current)?,
            }
            let response = self
                .send_write(method("MKCOL"), url.clone(), ctx, |req| req)
                .await?;
            match response.status() {
                // 405: created meanwhile by another connection.
                StatusCode::METHOD_NOT_ALLOWED => self.session.mark_known_dir(&current),
                status if status.is_success() => {
                    self.session.mark_created_dir(&current, &parent, segment);
                }
                other => check_write_status(other, &url, &current)?,
            }
        }
        Ok(())
    }

    /// `PUT` with an explicit `Content-Length`, streaming the body in chunks.
    // Mirrors dao/connection/webdav/WebdavDAO.java:279-345.
    pub(crate) async fn upload(
        &mut self,
        source: UploadSource,
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        let (parent, _) = path::split_parent(relative);
        self.mkdirs(parent, ctx).await?;
        ctx.check_cancelled()?;

        let len = source.len;
        let body = reqwest::Body::wrap_stream(upload_stream(source, ctx.clone()));
        let url = self.file_url(relative);
        let response = self
            .send_write(Method::PUT, url.clone(), ctx, |req| {
                req.header(header::CONTENT_LENGTH, HeaderValue::from(len))
                    .body(body)
            })
            .await
            .map_err(|err| {
                // The body stream reports cancellation as an I/O error inside reqwest's error.
                if ctx.cancel.is_cancelled() {
                    Error::Cancelled
                } else {
                    err
                }
            })?;
        check_write_status(response.status(), &url, relative)?;
        Ok(len)
    }

    /// `DELETE` (the server deletes collections recursively); missing → `Ok`.
    // Mirrors dao/connection/webdav/WebdavDAO.java:375-407.
    pub async fn delete(
        &mut self,
        relative: &str,
        kind: RemoteKind,
        ctx: &TransferCtx,
    ) -> Result<()> {
        let url = match kind {
            RemoteKind::File => self.file_url(relative),
            RemoteKind::Directory => self.dir_url(relative),
        };
        let response = self
            .send_write(Method::DELETE, url.clone(), ctx, |req| req)
            .await?;
        let status = response.status();
        if status == StatusCode::MULTI_STATUS {
            // RFC 4918 §9.6.1: some members of the collection could not be deleted.
            return Err(Error::Remote {
                operation: "delete",
                path: relative.to_owned(),
                reason: "207 Multi-Status: the server could not delete every member".into(),
            });
        }
        if !matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
            check_status(status, &url, relative)?;
        }
        let (parent, name) = path::split_parent(relative);
        self.session.remove_entry(parent, name, relative);
        Ok(())
    }
}

/// Status check for `PUT` and `MKCOL`: 409 Conflict means a missing parent collection.
// Mirrors dao/connection/webdav/WebdavDAO.java:231-242, 295, 342.
fn check_write_status(status: StatusCode, url: &Url, path: &str) -> Result<()> {
    if status == StatusCode::CONFLICT {
        return Err(Error::NotFound(path.to_owned()));
    }
    check_status(status, url, path)
}

/// Body stream of an upload: chunks of at most [`COPY_BUFFER`] bytes, counting progress and
/// honouring cancellation and throttling.
fn upload_stream(
    source: UploadSource,
    ctx: TransferCtx,
) -> impl futures::Stream<Item = std::io::Result<bytes::Bytes>> + Send + 'static {
    futures::stream::try_unfold((source.reader, ctx), |(mut reader, ctx)| async move {
        let mut buf = vec![0u8; COPY_BUFFER];
        let n = tokio::select! {
            () = ctx.cancel.cancelled() => {
                return Err(std::io::Error::other(Error::Cancelled));
            }
            res = reader.read(&mut buf) => res?,
        };
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n);
        ctx.progress.fetch_add(n as u64, Ordering::Relaxed);
        if let Some(throttle) = &ctx.throttle {
            tokio::select! {
                () = ctx.cancel.cancelled() => {
                    return Err(std::io::Error::other(Error::Cancelled));
                }
                () = throttle.consume(n) => {}
            }
        }
        Ok(Some((bytes::Bytes::from(buf), (reader, ctx))))
    })
}
