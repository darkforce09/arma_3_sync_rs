//! SFTP transport (`dao/connection/sftp/SftpDAO.java`), password authentication only.

use std::future::Future;
use std::io::SeekFrom;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use a3s_types::Protocol;
use russh::client::{AuthResult, Handle};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{OpenFlags, StatusCode};
use tokio::io::{AsyncSeekExt as _, AsyncWriteExt as _};

use crate::ctx::{TransferCtx, copy, copy_upload};
use crate::options::TransportOptions;
use crate::session::SessionState;
use crate::{Error, RemoteKind, Result, UploadSource, path};

/// Connection timeout used when the protocol's value is 0.
// Mirrors dao/connection/sftp/SftpDAO.java:531-538.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for closing a remote file handle (also after a cancelled upload).
const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Accepts any host key, like the Java client (Apache MINA's default verifier), but logs its
/// fingerprint.
struct HostKeyLogger {
    host: String,
}

impl russh::client::Handler for HostKeyLogger {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => tracing::info!(
                host = self.host,
                fingerprint = %key.fingerprint(russh::keys::HashAlg::Sha256),
                "SFTP host key"
            ),
            PublicKeyOrCertificate::Certificate(certificate) => tracing::info!(
                host = self.host,
                fingerprint = %certificate
                    .public_key()
                    .fingerprint(russh::keys::HashAlg::Sha256),
                key_id = certificate.key_id(),
                "SFTP host certificate"
            ),
        }
        Ok(true)
    }
}

/// Strips a leading `sftp://` (case-insensitive) and leading `/` from a stored URL.
// Mirrors dao/connection/sftp/SftpDAO.java:430-439.
pub(crate) fn normalize_url(value: &str) -> &str {
    let mut normalized = value.trim();
    const SCHEME: &str = "sftp://";
    if normalized
        .get(..SCHEME.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(SCHEME))
    {
        normalized = &normalized[SCHEME.len()..];
    }
    normalized.trim_start_matches('/')
}

/// `(host, configured path)` of a normalized URL; the path keeps its leading `/` or is empty.
// Mirrors dao/connection/sftp/SftpDAO.java:441-449.
pub(crate) fn split_url(normalized: &str) -> (&str, &str) {
    match normalized.find('/') {
        Some(i) => (&normalized[..i], &normalized[i..]),
        None => (normalized, ""),
    }
}

/// Base directory configured in the URL.
#[derive(Debug, PartialEq, Eq)]
enum ConfiguredBase {
    /// No path: the login (home) directory.
    Home,

    /// An absolute path (`/` for the SFTP root).
    Absolute(String),
}

/// Interprets the configured path: empty → home, only `/` → root, else the absolute path.
/// Only `..`, NUL, CR and LF are rejected (Java accepts `.`).
// Mirrors dao/connection/sftp/SftpDAO.java:414-423, 451-457.
fn configured_base(raw: &str) -> Result<ConfiguredBase> {
    let configured = raw.replace('\\', "/");
    if configured.contains(['\0', '\r', '\n']) || path::segments(&configured).any(|s| s == "..") {
        return Err(Error::InvalidPath(raw.escape_debug().to_string()));
    }
    if configured.is_empty() {
        return Ok(ConfiguredBase::Home);
    }
    let segments: Vec<&str> = path::segments(&configured).filter(|s| *s != ".").collect();
    Ok(ConfiguredBase::Absolute(path::join_remote(
        &segments.join("/"),
        "",
    )))
}

fn is_missing(err: &SftpError) -> bool {
    matches!(err, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile)
}

fn ignore_missing(result: Result<(), SftpError>) -> Result<()> {
    match result {
        Err(e) if !is_missing(&e) => Err(Error::Sftp(e)),
        _ => Ok(()),
    }
}

pub struct SftpTransport {
    _session: Handle<HostKeyLogger>,
    sftp: SftpSession,

    /// Absolute base directory of the repository.
    base: String,
    read_timeout: Option<Duration>,
    pub(crate) session: SessionState,
}

impl SftpTransport {
    // Mirrors dao/connection/sftp/SftpDAO.java:96-186.
    pub async fn connect(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        let (host, configured) = split_url(normalize_url(&protocol.url));
        if host.is_empty() {
            return Err(Error::InvalidProtocol(
                "SFTP host is empty. Enter a host name before the optional remote path.".into(),
            ));
        }
        let configured = configured_base(configured)?;
        let host = host.to_owned();
        let port = protocol
            .port_number()
            .ok_or_else(|| Error::InvalidProtocol(format!("invalid port '{}'", protocol.port)))?;
        let login = protocol.login.clone().unwrap_or_default();
        let password = protocol.password.clone().unwrap_or_default();
        if password.is_empty() {
            return Err(Error::InvalidProtocol(
                "SFTP password is empty. Key-based authentication is not configured yet.".into(),
            ));
        }

        let config = Arc::new(russh::client::Config {
            inactivity_timeout: options.read_timeout,
            ..Default::default()
        });
        let handler = HostKeyLogger { host: host.clone() };
        // One timeout covers connect, authentication and the SFTP subsystem.
        let timeout = options.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let setup = async {
            let mut session =
                russh::client::connect(config, (host.as_str(), port), handler).await?;
            match session.authenticate_password(login, password).await? {
                AuthResult::Success => {}
                AuthResult::Failure { .. } => return Err(Error::Unauthorized),
            }
            let channel = session.channel_open_session().await?;
            channel.request_subsystem(true, "sftp").await?;
            let sftp = SftpSession::new(channel.into_stream()).await?;
            Ok::<_, Error>((session, sftp))
        };
        let (session, sftp) = tokio::time::timeout(timeout, setup)
            .await
            .map_err(|_elapsed| Error::Timeout)??;

        // Mirrors dao/connection/sftp/SftpDAO.java:414-423 (empty → home, `/` → root).
        let base = match configured {
            ConfiguredBase::Home => {
                let home = sftp.canonicalize(".").await?;
                if home.is_empty() {
                    return Err(Error::InvalidProtocol(
                        "SFTP server did not provide a working directory.".into(),
                    ));
                }
                home
            }
            ConfiguredBase::Absolute(base) => base,
        };
        tracing::info!(host, base, "SFTP connected");
        Ok(Self {
            _session: session,
            sftp,
            base,
            read_timeout: options.read_timeout,
            session: SessionState::default(),
        })
    }

    fn absolute(&self, relative: &str) -> String {
        path::join_remote(&self.base, relative)
    }

    async fn open(&self, relative: &str) -> Result<russh_sftp::client::fs::File> {
        self.sftp.open(self.absolute(relative)).await.map_err(|e| {
            if is_missing(&e) {
                Error::NotFound(relative.to_owned())
            } else {
                Error::Sftp(e)
            }
        })
    }

    pub async fn fetch(&self, relative: &str, ctx: &TransferCtx) -> Result<Vec<u8>> {
        let mut file = self.open(relative).await?;
        let mut out = Vec::new();
        copy(&mut file, &mut out, ctx, self.read_timeout).await?;
        Ok(out)
    }

    pub async fn download_file(
        &self,
        relative: &str,
        dest: &Path,
        resume_from: u64,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        let mut remote = self.open(relative).await?;
        if resume_from > 0 {
            remote.seek(SeekFrom::Start(resume_from)).await?;
        }
        let mut file = crate::open_destination(dest, resume_from).await?;
        let written = copy(&mut remote, &mut file, ctx, self.read_timeout).await?;
        file.sync_all().await?;
        Ok(resume_from + written)
    }

    /// `stat`: `NoSuchFile` → `false`.
    async fn stat_exists(&self, absolute: &str) -> Result<bool> {
        match self.sftp.metadata(absolute).await {
            Ok(_) => Ok(true),
            Err(e) if is_missing(&e) => Ok(false),
            Err(e) => Err(Error::Sftp(e)),
        }
    }

    // Mirrors dao/connection/sftp/SftpDAO.java:266-275, 390-400.
    pub async fn exists(&self, relative: &str, _ctx: &TransferCtx) -> Result<bool> {
        self.stat_exists(&self.absolute(relative)).await
    }

    /// Creates every missing component of `join(base, relative_dir)`, including missing
    /// components of the configured base path (Java SFTP behaviour).
    // Mirrors dao/connection/sftp/SftpDAO.java:372-388.
    async fn mkdirs_absolute(&mut self, relative_dir: &str, ctx: &TransferCtx) -> Result<()> {
        let target = self.absolute(relative_dir);
        let mut current = String::new();
        for segment in path::segments(&target) {
            let parent = std::mem::take(&mut current);
            current = format!("{parent}/{segment}");
            if self.session.is_known_dir(&current) {
                continue;
            }
            ctx.check_cancelled()?;
            if self.stat_exists(&current).await? {
                self.session.mark_known_dir(&current);
                continue;
            }
            match self.sftp.create_dir(current.as_str()).await {
                Ok(()) => self.session.mark_known_dir(&current),
                // Another connection may have created it meanwhile.
                Err(e) => {
                    if self.stat_exists(&current).await? {
                        self.session.mark_known_dir(&current);
                    } else {
                        return Err(Error::Sftp(e));
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn mkdirs(&mut self, relative_dir: &str, ctx: &TransferCtx) -> Result<()> {
        self.mkdirs_absolute(relative_dir, ctx).await
    }

    /// Opens with `Create | Write | Truncate`, writes in 64 KiB chunks and closes.
    // Mirrors dao/connection/sftp/SftpDAO.java:277-327.
    pub(crate) async fn upload(
        &mut self,
        mut source: UploadSource,
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        let (parent, _) = path::split_parent(relative);
        self.mkdirs_absolute(parent, ctx).await?;
        ctx.check_cancelled()?;
        let target = self.absolute(relative);
        let mut file = self
            .sftp
            .open_with_flags(
                target,
                OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
            )
            .await?;
        let written = copy_upload(&mut source.reader, &mut file, ctx, self.read_timeout).await;
        // Close the handle even after an error (bounded, the server may be stuck); the partial
        // file stays (like Java).
        let close_timeout = self
            .read_timeout
            .unwrap_or(CLOSE_TIMEOUT)
            .min(CLOSE_TIMEOUT);
        let closed = tokio::time::timeout(close_timeout, file.shutdown()).await;
        let written = written?;
        closed.map_err(|_elapsed| Error::Timeout)??;
        Ok(written)
    }

    // Mirrors dao/connection/sftp/SftpDAO.java:339-370.
    pub async fn delete(
        &mut self,
        relative: &str,
        kind: RemoteKind,
        ctx: &TransferCtx,
    ) -> Result<()> {
        let target = self.absolute(relative);
        if !self.stat_exists(&target).await? {
            return Ok(());
        }
        self.delete_path(target.clone(), kind == RemoteKind::Directory, ctx)
            .await?;
        let (parent, name) = target.rsplit_once('/').unwrap_or(("", &target));
        self.session.remove_entry(parent, name, &target);
        Ok(())
    }

    fn delete_path<'a>(
        &'a self,
        target: String,
        directory: bool,
        ctx: &'a TransferCtx,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            ctx.check_cancelled()?;
            if directory {
                // Entries vanishing meanwhile (another client) are not an error.
                let entries = match self.sftp.read_dir(target.as_str()).await {
                    Ok(entries) => entries,
                    Err(e) if is_missing(&e) => return Ok(()),
                    Err(e) => return Err(e.into()),
                };
                for entry in entries {
                    let name = entry.file_name();
                    if name == "." || name == ".." {
                        continue;
                    }
                    let child = format!("{}/{name}", target.trim_end_matches('/'));
                    self.delete_path(child, entry.metadata().is_dir(), ctx)
                        .await?;
                }
                ignore_missing(self.sftp.remove_dir(target).await)
            } else {
                ignore_missing(self.sftp.remove_file(target).await)
            }
        })
    }

    pub async fn close(self) {
        let _ignored = self.sftp.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfiguredBase, configured_base, normalize_url, split_url};
    use crate::Error;

    #[test]
    fn configured_base_cases() {
        assert_eq!(configured_base("").unwrap(), ConfiguredBase::Home);
        for root in ["/", "//", "/./"] {
            assert_eq!(
                configured_base(root).unwrap(),
                ConfiguredBase::Absolute("/".into())
            );
        }
        assert_eq!(
            configured_base("/a/./b\\c").unwrap(),
            ConfiguredBase::Absolute("/a/b/c".into())
        );
        for bad in ["/a/../b", "/..", "/a\nb"] {
            assert!(
                matches!(configured_base(bad), Err(Error::InvalidPath(_))),
                "{bad:?}"
            );
        }
    }

    // Ported from src/test/java/fr/soe/a3s/dao/connection/sftp/SftpDAOTest.java.
    #[test]
    fn sftp_url_normalisation() {
        let url = normalize_url("sftp:///enterprise.vpzbrig21.de/arma3sync/workshop");
        assert_eq!(url, "enterprise.vpzbrig21.de/arma3sync/workshop");
        assert_eq!(
            split_url(url),
            ("enterprise.vpzbrig21.de", "/arma3sync/workshop")
        );
        assert_eq!(
            split_url(normalize_url("enterprise.vpzbrig21.de")),
            ("enterprise.vpzbrig21.de", "")
        );
        assert_eq!(split_url(normalize_url("SFTP://h/p")), ("h", "/p"));
        assert_eq!(split_url(normalize_url("///h/p/q")), ("h", "/p/q"));
        assert_eq!(split_url(normalize_url("sftp://")), ("", ""));
    }
}
