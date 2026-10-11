//! HTTP(S) access for the updater: metadata reads, archive download with SHA-256 verification.

use std::path::{Path, PathBuf};

use a3s_foundation::error::format_chain;
use futures::StreamExt as _;
use reqwest::header::{CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Response};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::release::is_safe_zip_name;
use crate::{Release, UpdateConfig, UpdateError};

/// `User-Agent` of every updater request.
// Mirrors a3sUpdater/dao/HttpDAO.java:146
pub const USER_AGENT: &str = "ArmA3Sync-Updater";

/// Largest accepted metadata document.
// Mirrors a3sUpdater/dao/HttpDAO.java:87-90
pub const MAX_METADATA_BYTES: u64 = 1_048_576;

const MAX_REDIRECTS: usize = 10;

/// HTTP client configured from an [`UpdateConfig`]: HTTPS only unless `allow_http`, the
/// configured timeouts, at most 10 redirects, none from HTTPS to HTTP. No retries.
#[derive(Clone, Debug)]
pub struct HttpClient {
    client: reqwest::Client,
    allow_http: bool,
}

impl HttpClient {
    /// Builds the client.
    // Mirrors a3sUpdater/dao/HttpDAO.java:135-148
    pub fn new(config: &UpdateConfig) -> Result<Self, UpdateError> {
        let allow_http = config.allow_http();
        let redirect = reqwest::redirect::Policy::custom(move |attempt| {
            let previous_https = attempt
                .previous()
                .last()
                .is_some_and(|url| url.scheme() == "https");
            if let Err(err) = check_scheme(attempt.url(), allow_http) {
                attempt.error(err)
            } else if previous_https && attempt.url().scheme() == "http" {
                attempt.error(UpdateError::InsecureScheme(
                    "Redirect from HTTPS to HTTP refused.".to_owned(),
                ))
            } else if attempt.previous().len() > MAX_REDIRECTS {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(config.connect_timeout())
            .read_timeout(config.read_timeout())
            .redirect(redirect)
            .build()
            .map_err(|e| UpdateError::Transport(format_chain(&e)))?;
        Ok(Self { client, allow_http })
    }

    /// GETs a metadata document of at most 1 MiB, decoded as UTF-8 (invalid bytes replaced).
    // Mirrors a3sUpdater/dao/HttpDAO.java:81-96
    pub async fn read_text(
        &self,
        url: &Url,
        headers: &[(&str, &str)],
        cancel: &CancellationToken,
    ) -> Result<String, UpdateError> {
        let mut extra = HeaderMap::new();
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| UpdateError::Transport(format!("Invalid header name {name}: {e}")))?;
            let value = HeaderValue::from_str(value)
                .map_err(|e| UpdateError::Transport(format!("Invalid header value: {e}")))?;
            extra.insert(name, value);
        }
        let response = self.send(Method::GET, url, extra, cancel).await?;
        if header_content_length(&response).is_some_and(|len| len > MAX_METADATA_BYTES) {
            return Err(UpdateError::TooLarge);
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = cancellable(cancel, stream.next()).await? {
            let chunk = chunk.map_err(|e| transport_error(&e))?;
            if body.len() as u64 + chunk.len() as u64 > MAX_METADATA_BYTES {
                return Err(UpdateError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }

    /// HEADs `url` and returns its `Content-Length`, if the server sent one.
    // Mirrors a3sUpdater/dao/HttpDAO.java:27-36
    pub async fn content_length(
        &self,
        url: &Url,
        cancel: &CancellationToken,
    ) -> Result<Option<u64>, UpdateError> {
        let response = self
            .send(Method::HEAD, url, HeaderMap::new(), cancel)
            .await?;
        Ok(header_content_length(&response))
    }

    /// Sends one request and requires a 2xx status.
    async fn send(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        cancel: &CancellationToken,
    ) -> Result<Response, UpdateError> {
        check_scheme(url, self.allow_http)?;
        let request = self
            .client
            .request(method, url.clone())
            .headers(headers)
            .send();
        let response = cancellable(cancel, request)
            .await?
            .map_err(|e| transport_error(&e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpdateError::Http {
                status: status.as_u16(),
            });
        }
        Ok(response)
    }
}

/// Only `http`/`https`, and `http` only when allowed.
// Mirrors a3sUpdater/dao/HttpDAO.java:136-141
fn check_scheme(url: &Url, allow_http: bool) -> Result<(), UpdateError> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if allow_http => Ok(()),
        "http" => Err(UpdateError::InsecureScheme(
            "Updater requires HTTPS; set allow_http for local tests.".to_owned(),
        )),
        _ => Err(UpdateError::InsecureScheme(
            "Update URL must use HTTP or HTTPS.".to_owned(),
        )),
    }
}

/// The `Content-Length` header (reqwest reports 0 for every HEAD response body).
fn header_content_length(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn transport_error(err: &reqwest::Error) -> UpdateError {
    // A refused redirect carries the updater's own error as its source.
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(inner) = source {
        if let Some(UpdateError::InsecureScheme(message)) = inner.downcast_ref::<UpdateError>() {
            return UpdateError::InsecureScheme(message.clone());
        }
        source = inner.source();
    }
    if err.is_timeout() {
        UpdateError::Transport(format!("Update server timed out: {}", format_chain(err)))
    } else {
        UpdateError::Transport(format_chain(err))
    }
}

/// Awaits `future` unless `cancel` fires first.
async fn cancellable<T>(
    cancel: &CancellationToken,
    future: impl Future<Output = T>,
) -> Result<T, UpdateError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(UpdateError::Cancelled),
        value = future => Ok(value),
    }
}

/// Temporary directory (`arma3sync-update-*`) that holds a downloaded archive. It is deleted
/// when dropped unless [`Self::keep`] was called; cancelling a download does not delete it.
#[derive(Debug)]
pub struct StagingDir {
    dir: tempfile::TempDir,
}

impl StagingDir {
    /// Creates a new directory in the system temporary directory.
    // Mirrors a3sUpdater/dao/HttpDAO.java:40
    pub fn new() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("arma3sync-update-")
            .tempdir()?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Keeps the directory on disk and returns its path.
    pub fn keep(self) -> PathBuf {
        self.dir.keep()
    }
}

/// Options of [`download_release`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownloadOptions {
    /// Download releases without a SHA-256 checksum (legacy XML) instead of refusing them with
    /// [`UpdateError::Unverified`].
    pub allow_unverified: bool,
}

/// A downloaded archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadedArchive {
    /// `<staging>/<release.file_name>`.
    pub path: PathBuf,
    pub bytes: u64,

    /// Whether the SHA-256 checksum was checked (and matched).
    pub verified: bool,
}

/// Downloads the release archive into `staging`.
///
/// `progress` receives the total number of bytes written after every chunk. When the release
/// announces a size, more bytes than announced abort the download and fewer fail it. A release
/// without SHA-256 is refused unless [`DownloadOptions::allow_unverified`] is set.
// Mirrors a3sUpdater/dao/HttpDAO.java:38-65,154-171
pub async fn download_release(
    http: &HttpClient,
    release: &Release,
    staging: &StagingDir,
    options: DownloadOptions,
    progress: &(dyn Fn(u64) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<DownloadedArchive, UpdateError> {
    if release.sha256.is_none() && !options.allow_unverified {
        return Err(UpdateError::Unverified);
    }
    if !is_safe_zip_name(&release.file_name) {
        return Err(UpdateError::Manifest(format!(
            "Invalid update archive name: {}",
            release.file_name
        )));
    }
    let path = staging.path().join(&release.file_name);
    let response = http
        .send(Method::GET, &release.download_url, HeaderMap::new(), cancel)
        .await?;
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| UpdateError::io(format!("Could not create {}", path.display()), e))?;
    let write_error = |e| UpdateError::io(format!("Could not write {}", path.display()), e);

    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = cancellable(cancel, stream.next()).await? {
        let chunk = chunk.map_err(|e| transport_error(&e))?;
        total += chunk.len() as u64;
        if release.size > 0 && total > release.size {
            return Err(UpdateError::Manifest(format!(
                "Update archive is larger than the announced {} bytes.",
                release.size
            )));
        }
        hasher.update(&chunk);
        cancellable(cancel, file.write_all(&chunk))
            .await?
            .map_err(write_error)?;
        progress(total);
    }
    file.flush().await.map_err(write_error)?;
    drop(file);

    if release.size > 0 && total < release.size {
        return Err(UpdateError::Manifest(format!(
            "Update archive has {total} of the announced {} bytes.",
            release.size
        )));
    }
    let verified = match &release.sha256 {
        Some(expected) => {
            let actual = to_hex(&hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected.trim()) {
                return Err(UpdateError::Integrity);
            }
            true
        }
        None => false,
    };
    Ok(DownloadedArchive {
        path,
        bytes: total,
        verified,
    })
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ignored = write!(out, "{b:02x}");
        out
    })
}
