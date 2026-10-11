//! HTTP(S) transport (`dao/connection/http/HttpDAO.java`).

use std::ops::Range;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use a3s_types::{Protocol, ProtocolType, ssl_policy};
use base64::Engine as _;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{StatusCode, Url};
use tokio::io::AsyncWriteExt as _;

use crate::ctx::{TransferCtx, maybe_timeout};
use crate::options::TransportOptions;
use crate::session::SessionState;
use crate::{Error, Result, path};

/// User agent sent by Arma3Sync; some repository hosts filter on it.
pub const USER_AGENT: &str = "ArmA3Sync";

pub struct HttpTransport {
    pub(crate) client: reqwest::Client,
    pub(crate) base: Url,
    pub(crate) remote_path: String,
    pub(crate) auth: Option<HeaderValue>,
    read_timeout: Option<Duration>,
    pub(crate) session: SessionState,
}

/// Builds the shared client used by the HTTP and WebDAV transports. Without
/// `follow_redirects` every 3xx is returned to the caller (WebDAV write verbs).
pub(crate) fn build_client(
    protocol: &Protocol,
    options: &TransportOptions,
    follow_redirects: bool,
) -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    // Repository files are already gzip-compressed; transparent compression would break
    // Content-Length / Range handling.
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, max-age=0"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));

    let redirect = if follow_redirects {
        reqwest::redirect::Policy::custom(|attempt| {
            let downgrade = attempt.url().scheme() == "http"
                && attempt
                    .previous()
                    .last()
                    .is_some_and(|u| u.scheme() == "https");
            if downgrade {
                attempt.error(Error::InsecureRedirect)
            } else if attempt.previous().len() >= 10 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        })
    } else {
        reqwest::redirect::Policy::none()
    };
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .redirect(redirect);
    if let Some(t) = options.connect_timeout {
        builder = builder.connect_timeout(t);
    }
    if let Some(t) = options.read_timeout {
        builder = builder.read_timeout(t);
    }
    if let Some(proxy) = &options.proxy {
        let mut p = reqwest::Proxy::all(proxy.url())?;
        if let (Some(login), Some(password)) = (&proxy.login, &proxy.password) {
            p = p.basic_auth(login, password);
        }
        builder = builder.proxy(p);
    }
    let tls = matches!(
        protocol.protocol_type,
        ProtocolType::Https | ProtocolType::HttpsWebdav
    );
    if tls && !protocol.validate_ssl_certificate {
        if !ssl_policy::is_allowed_for(protocol.hostname(), options.allow_insecure_tls) {
            return Err(Error::InsecureTlsRefused(protocol.hostname().to_owned()));
        }
        tracing::warn!(
            host = protocol.hostname(),
            "SSL certificate validation disabled"
        );
        builder = builder.tls_danger_accept_invalid_certs(true);
    }
    Ok(builder.build()?)
}

/// `Authorization` header for preemptive Basic auth, unless the login is anonymous.
pub(crate) fn basic_auth(protocol: &Protocol) -> Option<HeaderValue> {
    if protocol.is_anonymous() {
        return None;
    }
    let credentials = format!(
        "{}:{}",
        protocol.login.as_deref().unwrap_or_default(),
        protocol.password.as_deref().unwrap_or_default()
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
    let mut value = HeaderValue::from_str(&format!("Basic {encoded}")).ok()?;
    value.set_sensitive(true);
    Some(value)
}

/// Base URL `scheme://host:port/` of a protocol.
pub(crate) fn base_url(protocol: &Protocol) -> Result<Url> {
    let scheme = match protocol.protocol_type {
        ProtocolType::Http | ProtocolType::HttpWebdav => "http",
        ProtocolType::Https | ProtocolType::HttpsWebdav => "https",
        other => {
            return Err(Error::InvalidProtocol(format!(
                "{} is not an HTTP protocol",
                other.description()
            )));
        }
    };
    let port = protocol
        .port_number()
        .ok_or_else(|| Error::InvalidProtocol(format!("invalid port '{}'", protocol.port)))?;
    Url::parse(&format!("{scheme}://{}:{port}/", protocol.hostname()))
        .map_err(|e| Error::InvalidProtocol(format!("invalid URL: {e}")))
}

/// URL of `absolute_path` (already joined with the remote path), percent-encoding segments.
pub(crate) fn url_for(base: &Url, absolute_path: &str) -> Url {
    let mut url = base.clone();
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.clear();
        segments.extend(path::segments(absolute_path));
    }
    url
}

/// Maps an existence probe status: 200 → `true`, 404/410 → `false`, other 2xx → `false`.
// Mirrors dao/connection/http/HttpDAO.java:172-182, 395-399.
fn exists_from_status(status: StatusCode, url: &Url) -> Result<bool> {
    match status {
        StatusCode::OK => Ok(true),
        StatusCode::NOT_FOUND | StatusCode::GONE => Ok(false),
        StatusCode::UNAUTHORIZED => Err(Error::Unauthorized),
        s if s.is_success() => Ok(false),
        s => Err(Error::HttpStatus {
            status: s.as_u16(),
            url: url.to_string(),
        }),
    }
}

pub(crate) fn check_status(status: StatusCode, url: &Url, path: &str) -> Result<()> {
    if status.is_success() {
        return Ok(());
    }
    Err(match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Error::Unauthorized,
        StatusCode::NOT_FOUND | StatusCode::GONE => Error::NotFound(path.to_owned()),
        _ => Error::HttpStatus {
            status: status.as_u16(),
            url: url.to_string(),
        },
    })
}

impl HttpTransport {
    pub fn new(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        Ok(Self {
            client: build_client(protocol, options, true)?,
            base: base_url(protocol)?,
            remote_path: protocol.remote_path().to_owned(),
            auth: basic_auth(protocol),
            read_timeout: options.read_timeout,
            session: SessionState::default(),
        })
    }

    /// Request with the `Authorization` header (if any) on `client`.
    pub(crate) fn request(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        url: Url,
    ) -> reqwest::RequestBuilder {
        let mut req = client.request(method, url);
        if let Some(auth) = &self.auth {
            req = req.header(header::AUTHORIZATION, auth.clone());
        }
        req
    }

    /// Sends `req`, aborting on cancellation.
    pub(crate) async fn send_cancellable(
        req: reqwest::RequestBuilder,
        ctx: &TransferCtx,
    ) -> Result<reqwest::Response> {
        tokio::select! {
            () = ctx.cancel.cancelled() => Err(Error::Cancelled),
            res = req.send() => Ok(res?),
        }
    }

    /// Whether a file exists: `HEAD`, or `GET` when the server refuses `HEAD`.
    // Mirrors dao/connection/http/HttpDAO.java:172-182, 390-407 (`fileExists` = response 200).
    pub async fn exists(&mut self, relative: &str, ctx: &TransferCtx) -> Result<bool> {
        let url = self.url(relative);
        let head = self.request(&self.client, reqwest::Method::HEAD, url.clone());
        let mut response = Self::send_cancellable(head, ctx).await?;
        if matches!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
        ) {
            let get = self.request(&self.client, reqwest::Method::GET, url.clone());
            // The body is dropped unread with the response.
            response = Self::send_cancellable(get, ctx).await?;
        }
        exists_from_status(response.status(), &url)
    }

    pub fn url(&self, relative: &str) -> Url {
        url_for(&self.base, &path::join_remote(&self.remote_path, relative))
    }

    fn get(&self, relative: &str) -> (Url, reqwest::RequestBuilder) {
        let url = self.url(relative);
        let mut req = self.client.get(url.clone());
        if let Some(auth) = &self.auth {
            req = req.header(header::AUTHORIZATION, auth.clone());
        }
        (url, req)
    }

    async fn send(
        &self,
        relative: &str,
        range: Option<String>,
        ctx: &TransferCtx,
    ) -> Result<reqwest::Response> {
        let (url, mut req) = self.get(relative);
        if let Some(range) = range {
            req = req.header(header::RANGE, range);
        }
        let response = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            res = req.send() => res?,
        };
        check_status(response.status(), &url, relative)?;
        Ok(response)
    }

    async fn next_chunk(
        &self,
        response: &mut reqwest::Response,
        ctx: &TransferCtx,
    ) -> Result<Option<bytes::Bytes>> {
        let chunk = response.chunk();
        let chunk = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            res = maybe_timeout(self.read_timeout, async { chunk.await.map_err(std::io::Error::other) }) => res?,
        };
        Ok(chunk)
    }

    /// Downloads a whole file into memory.
    pub async fn fetch(&mut self, relative: &str, ctx: &TransferCtx) -> Result<Vec<u8>> {
        let mut response = self.send(relative, None, ctx).await?;
        let expected = response.content_length();
        let mut out = Vec::with_capacity(expected.unwrap_or(0).min(64 << 20) as usize);
        while let Some(chunk) = self.next_chunk(&mut response, ctx).await? {
            out.extend_from_slice(&chunk);
            ctx.progress
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        if let Some(expected) = expected
            && out.len() as u64 != expected
        {
            return Err(Error::Incomplete {
                expected,
                actual: out.len() as u64,
            });
        }
        Ok(out)
    }

    /// Downloads `range` (end exclusive) of a file into memory.
    pub async fn fetch_range(
        &mut self,
        relative: &str,
        range: Range<u64>,
        ctx: &TransferCtx,
    ) -> Result<Vec<u8>> {
        if range.is_empty() {
            return Ok(Vec::new());
        }
        let header = format!("bytes={}-{}", range.start, range.end - 1);
        let mut response = self.send(relative, Some(header), ctx).await?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(Error::Unsupported {
                protocol: "HTTP server",
                operation: "range requests",
            });
        }
        let expected = range.end - range.start;
        let mut out = Vec::with_capacity(expected as usize);
        while let Some(chunk) = self.next_chunk(&mut response, ctx).await? {
            out.extend_from_slice(&chunk);
            ctx.account(chunk.len()).await?;
        }
        if out.len() as u64 != expected {
            return Err(Error::Incomplete {
                expected,
                actual: out.len() as u64,
            });
        }
        Ok(out)
    }

    /// Downloads to `dest`, resuming from `resume_from` bytes if the server supports ranges.
    ///
    /// Returns the final size of `dest`.
    pub async fn download_file(
        &mut self,
        relative: &str,
        dest: &Path,
        resume_from: u64,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        let range = (resume_from > 0).then(|| format!("bytes={resume_from}-"));
        let mut response = self.send(relative, range, ctx).await?;
        let resumed = resume_from > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        let offset = if resumed { resume_from } else { 0 };
        let expected = response.content_length().map(|len| len + offset);

        let mut file = crate::open_destination(dest, offset, ctx).await?;
        let mut written = offset;
        while let Some(chunk) = self.next_chunk(&mut response, ctx).await? {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            ctx.account(chunk.len()).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        if let Some(expected) = expected
            && written != expected
        {
            return Err(Error::Incomplete {
                expected,
                actual: written,
            });
        }
        Ok(written)
    }

    /// Whether the server answers range requests (`HttpDAO.checkPartialFileTransfer`):
    /// `Range: bytes=0-1` on `.a3s/sync` must return 206.
    pub async fn supports_ranges(&mut self, ctx: &TransferCtx) -> bool {
        self.probe_range(".a3s/sync", ctx)
            .await
            .is_ok_and(|probe| probe.supports_ranges())
    }

    /// Requests `Range: bytes=0-1` of `relative` and returns the status and the response
    /// headers, which the partial-transfer check shows when the server ignored the range. The
    /// body is not read. Error statuses fail like a download.
    // Mirrors dao/connection/http/HttpDAO.java:431-446, 540-560.
    pub async fn probe_range(&mut self, relative: &str, ctx: &TransferCtx) -> Result<RangeProbe> {
        path::validate_relative(relative)?;
        let response = self
            .send(relative, Some("bytes=0-1".to_owned()), ctx)
            .await?;
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        Ok(RangeProbe {
            status: response.status().as_u16(),
            headers,
        })
    }
}

/// Answer of [`HttpTransport::probe_range`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeProbe {
    /// HTTP status: 206 when the server honoured the range.
    pub status: u16,

    /// Response headers in the order received (lower-case names, values decoded lossily).
    pub headers: Vec<(String, String)>,
}

impl RangeProbe {
    /// Whether the server answered with 206 Partial Content.
    pub fn supports_ranges(&self) -> bool {
        self.status == StatusCode::PARTIAL_CONTENT.as_u16()
    }
}
