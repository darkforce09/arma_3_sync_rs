//! SFTP transport (`dao/connection/sftp/SftpDAO.java`), password authentication only.

use std::io::SeekFrom;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s_types::Protocol;
use russh::client::{AuthResult, Handle};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use tokio::io::AsyncSeekExt as _;

use crate::ctx::{TransferCtx, copy};
use crate::options::TransportOptions;
use crate::{Error, Result, path};

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
        if let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key {
            tracing::info!(
                host = self.host,
                fingerprint = %key.fingerprint(russh::keys::HashAlg::Sha256),
                "SFTP host key"
            );
        }
        Ok(true)
    }
}

pub struct SftpTransport {
    _session: Handle<HostKeyLogger>,
    sftp: SftpSession,
    remote_path: String,
    read_timeout: Option<Duration>,
}

impl SftpTransport {
    pub async fn connect(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        let host = protocol.hostname().to_owned();
        let port = protocol
            .port_number()
            .ok_or_else(|| Error::InvalidProtocol(format!("invalid port '{}'", protocol.port)))?;
        let config = Arc::new(russh::client::Config {
            inactivity_timeout: options.read_timeout,
            ..Default::default()
        });
        let handler = HostKeyLogger { host: host.clone() };
        let connect = russh::client::connect(config, (host.as_str(), port), handler);
        let mut session = match options.connect_timeout {
            Some(t) => tokio::time::timeout(t, connect)
                .await
                .map_err(|_elapsed| Error::Timeout)??,
            None => connect.await?,
        };

        let login = protocol.login.clone().unwrap_or_default();
        let password = protocol.password.clone().unwrap_or_default();
        match session.authenticate_password(login, password).await? {
            AuthResult::Success => {}
            AuthResult::Failure { .. } => return Err(Error::Unauthorized),
        }
        let channel = session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream()).await?;
        Ok(Self {
            _session: session,
            sftp,
            remote_path: protocol.remote_path().to_owned(),
            read_timeout: options.read_timeout,
        })
    }

    fn absolute(&self, relative: &str) -> String {
        path::join_remote(&self.remote_path, relative)
    }

    async fn open(&self, relative: &str) -> Result<russh_sftp::client::fs::File> {
        self.sftp
            .open(self.absolute(relative))
            .await
            .map_err(|e| match e {
                russh_sftp::client::error::Error::Status(ref s)
                    if s.status_code == russh_sftp::protocol::StatusCode::NoSuchFile =>
                {
                    Error::NotFound(relative.to_owned())
                }
                other => Error::Sftp(other),
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

    pub async fn close(self) {
        let _ignored = self.sftp.close().await;
    }
}
