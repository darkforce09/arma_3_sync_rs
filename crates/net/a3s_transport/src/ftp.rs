//! FTP / explicit FTPS transport (`dao/connection/ftp/FtpDAO.java`).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s_types::{Protocol, ProtocolType};
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode, Status};

use crate::ctx::{TransferCtx, copy};
use crate::options::{TransportOptions, is_local_test_host};
use crate::{Error, Result, path};

pub struct FtpTransport {
    stream: AsyncRustlsFtpStream,
    remote_path: String,
    read_timeout: Option<Duration>,
}

fn map_ftp_error(err: FtpError, path: &str) -> Error {
    match &err {
        FtpError::UnexpectedResponse(response)
            if response.status == Status::FileUnavailable
                || response.status == Status::NotLoggedIn =>
        {
            if response.status == Status::NotLoggedIn {
                Error::Unauthorized
            } else {
                Error::NotFound(path.to_owned())
            }
        }
        _ => Error::Ftp(err),
    }
}

impl FtpTransport {
    pub async fn connect(protocol: &Protocol, options: &TransportOptions) -> Result<Self> {
        let host = protocol.hostname().to_owned();
        let port = protocol
            .port_number()
            .ok_or_else(|| Error::InvalidProtocol(format!("invalid port '{}'", protocol.port)))?;
        let addr = tokio::net::lookup_host((host.as_str(), port))
            .await?
            .next()
            .ok_or_else(|| Error::InvalidProtocol(format!("unknown host '{host}'")))?;

        let connect = AsyncRustlsFtpStream::connect_timeout(
            addr,
            options.connect_timeout.unwrap_or(Duration::from_secs(300)),
        );
        let mut stream = connect.await?;

        if protocol.protocol_type == ProtocolType::Ftps {
            let validate = protocol.validate_ssl_certificate;
            if !validate && !is_local_test_host(&host) && !options.allow_insecure_tls {
                return Err(Error::InsecureTlsRefused(host));
            }
            let config = crate::tls::client_config(validate)?;
            let connector =
                AsyncRustlsConnector::from(tokio_rustls::TlsConnector::from(Arc::new(config)));
            stream = stream.into_secure(connector, &host).await?;
        }

        let login = protocol.login.as_deref().unwrap_or("anonymous");
        let password = protocol.password.as_deref().unwrap_or("");
        stream.login(login, password).await.map_err(|e| match e {
            FtpError::UnexpectedResponse(_) => Error::Unauthorized,
            other => Error::Ftp(other),
        })?;
        stream.transfer_type(FileType::Binary).await?;
        stream.set_mode(Mode::Passive);
        // Servers behind NAT often announce a private address in their PASV reply.
        stream.set_passive_nat_workaround(true);

        Ok(Self {
            stream,
            remote_path: protocol.remote_path().to_owned(),
            read_timeout: options.read_timeout,
        })
    }

    fn absolute(&self, relative: &str) -> String {
        path::join_remote(&self.remote_path, relative)
    }

    async fn retrieve<W>(
        &mut self,
        relative: &str,
        offset: u64,
        writer: &mut W,
        ctx: &TransferCtx,
    ) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin + ?Sized,
    {
        let absolute = self.absolute(relative);
        if offset > 0 {
            self.stream.resume_transfer(offset as usize).await?;
        }
        let mut transfer = self
            .stream
            .retr_as_stream(&absolute)
            .await
            .map_err(|e| map_ftp_error(e, relative))?;
        let copied = copy(&mut transfer, writer, ctx, self.read_timeout).await;
        match copied {
            Ok(n) => {
                transfer.finish().await?;
                Ok(n)
            }
            Err(err) => {
                // Dropping the transfer stream closes the data connection.
                drop(transfer);
                Err(err)
            }
        }
    }

    pub async fn fetch(&mut self, relative: &str, ctx: &TransferCtx) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.retrieve(relative, 0, &mut out, ctx).await?;
        Ok(out)
    }

    pub async fn download_file(
        &mut self,
        relative: &str,
        dest: &Path,
        resume_from: u64,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        let mut file = crate::open_destination(dest, resume_from).await?;
        let written = self.retrieve(relative, resume_from, &mut file, ctx).await?;
        file.sync_all().await?;
        Ok(resume_from + written)
    }

    pub async fn quit(mut self) {
        let _ignored = self.stream.quit().await;
    }
}
