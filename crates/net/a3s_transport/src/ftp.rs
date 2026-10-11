//! FTP / explicit FTPS transport (`dao/connection/ftp/FtpDAO.java`).
//!
//! All commands use absolute paths below the base directory resolved at connect time.

use std::collections::HashSet;
use std::future::Future;
use std::net::IpAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use a3s_types::{Protocol, ProtocolType};
use rustls::pki_types::ServerName;
use suppaftp::list::{File as ListEntry, ListParser};
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::{FileType, Response};
use suppaftp::{FtpError, FtpResult, Mode, Status};
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio_rustls::TlsConnector;

use crate::ctx::{TransferCtx, copy, copy_upload};
use crate::options::{TransportOptions, is_local_test_host};
use crate::session::SessionState;
use crate::{Error, RemoteKind, Result, UploadSource, path};

/// Upper bound for reading the reply of an interrupted transfer.
const RESYNC_TIMEOUT: Duration = Duration::from_secs(30);

pub struct FtpTransport {
    stream: AsyncRustlsFtpStream,

    /// Absolute base directory: the configured remote path, or the login directory.
    base: String,
    read_timeout: Option<Duration>,

    /// Address of the control connection's peer; passive data connections go there (NAT
    /// workaround).
    peer_ip: IpAddr,

    /// TLS settings for data connections opened by hand (FTPS).
    data_tls: Option<(TlsConnector, ServerName<'static>)>,
    pub(crate) session: SessionState,

    /// Set when the control connection could not be resynchronised after an interrupted
    /// transfer; every later call fails.
    poisoned: bool,
}

fn reply_code(err: &FtpError) -> Option<u32> {
    match err {
        FtpError::UnexpectedResponse(response) => Some(response.status.code()),
        _ => None,
    }
}

fn reply_text(response: &Response) -> String {
    response
        .as_string()
        .unwrap_or_else(|_| response.status.code().to_string())
        .trim()
        .to_owned()
}

fn map_ftp_error(err: FtpError, path: &str) -> Error {
    match &err {
        FtpError::UnexpectedResponse(response) if response.status == Status::NotLoggedIn => {
            Error::Unauthorized
        }
        FtpError::UnexpectedResponse(response) if response.status == Status::FileUnavailable => {
            Error::NotFound(path.to_owned())
        }
        _ => Error::Ftp(err),
    }
}

/// Maps a refused command on `path` to [`Error::Remote`]; connection errors stay
/// [`Error::Ftp`].
fn remote_error(err: FtpError, operation: &'static str, path: &str) -> Error {
    match err {
        FtpError::UnexpectedResponse(response) => Error::Remote {
            operation,
            path: path.to_owned(),
            reason: reply_text(&response),
        },
        other => Error::Ftp(other),
    }
}

async fn with_timeout<T>(
    timeout: Option<Duration>,
    fut: impl Future<Output = FtpResult<T>>,
) -> Result<T> {
    match timeout {
        Some(t) => tokio::time::timeout(t, fut)
            .await
            .map_err(|_elapsed| Error::Timeout)?
            .map_err(Error::Ftp),
        None => fut.await.map_err(Error::Ftp),
    }
}

/// What an `MLST` reply code says about a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MlstOutcome {
    Found,
    Missing,

    /// The server does not implement `MLST`: look the name up in a listing.
    Fallback,
    Failed,
}

// Mirrors dao/connection/ftp/FtpDAO.java:479-494.
fn mlst_outcome(code: u32) -> MlstOutcome {
    match code {
        200..=299 => MlstOutcome::Found,
        550 => MlstOutcome::Missing,
        500 | 501 | 502 | 504 => MlstOutcome::Fallback,
        _ => MlstOutcome::Failed,
    }
}

/// Parses `LIST` lines (POSIX, then DOS format). `None` when nothing parses although there
/// are lines other than `total …`, i.e. the format is unknown.
fn parse_listing(lines: &[String]) -> Option<Vec<ListEntry>> {
    let mut entries = Vec::new();
    let mut unparsed = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("total") {
            continue;
        }
        match ListParser::parse_posix(trimmed).or_else(|_| ListParser::parse_dos(trimmed)) {
            Ok(entry) if entry.name() == "." || entry.name() == ".." => {}
            Ok(entry) => entries.push(entry),
            Err(_) => unparsed = true,
        }
    }
    if entries.is_empty() && unparsed {
        None
    } else {
        Some(entries)
    }
}

/// Port of a `227` reply (`… (h1,h2,h3,h4,p1,p2)`).
fn parse_pasv_port(reply: &str) -> Option<u16> {
    let text = match reply.find('(') {
        Some(i) => &reply[i + 1..],
        None => reply.trim_start().get(3..)?,
    };
    let numbers: Vec<u16> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(6)
        .map(|s| s.parse::<u16>().ok().filter(|n| *n <= 255))
        .collect::<Option<_>>()?;
    (numbers.len() == 6).then(|| numbers[4] * 256 + numbers[5])
}

/// A directory listing.
enum Listing {
    Entries(Vec<ListEntry>),

    /// The directory cannot be entered (`CWD` refused).
    Missing,

    /// The listing was refused or its format is unknown.
    Unusable,
}

/// `size=` fact of an `MLST` fact line (`type=file;size=123;modify=…; name`).
fn mlst_size(facts: &str) -> Option<u64> {
    let facts = facts.split_once(' ').map_or(facts, |(facts, _name)| facts);
    facts.split(';').find_map(|fact| {
        let (key, value) = fact.split_once('=')?;
        if key.trim().eq_ignore_ascii_case("size") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

/// Result of a resumed download.
enum Resume {
    /// Final size of the destination.
    Done(u64),

    /// The server cannot resume (or ignored `REST`): download from the start.
    Restart,
}

impl FtpTransport {
    // Mirrors dao/connection/ftp/FtpDAO.java:112-193.
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

        let mut data_tls = None;
        if protocol.protocol_type == ProtocolType::Ftps {
            let validate = protocol.validate_ssl_certificate;
            if !validate && !is_local_test_host(&host) && !options.allow_insecure_tls {
                return Err(Error::InsecureTlsRefused(host));
            }
            let config = Arc::new(crate::tls::client_config(validate)?);
            let server_name = ServerName::try_from(host.clone())
                .map_err(|e| Error::Tls(format!("invalid server name '{host}': {e}")))?;
            let connector = TlsConnector::from(Arc::clone(&config));
            // AUTH TLS, then PBSZ 0 + PROT P (FtpDAO.java:148-154).
            stream = stream
                .into_secure(AsyncRustlsConnector::from(connector.clone()), &host)
                .await?;
            data_tls = Some((connector, server_name));
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

        // Base directory (FtpDAO.java:70-73, 172-193): `CWD <remote path>`, or the login
        // directory.
        let remote_path = protocol.remote_path();
        let base = if remote_path.is_empty() {
            stream.pwd().await?
        } else {
            match stream.cwd(remote_path).await {
                Ok(()) => path::join_remote(remote_path, ""),
                Err(FtpError::UnexpectedResponse(_)) => {
                    return Err(Error::NotFound(remote_path.to_owned()));
                }
                Err(e) => return Err(e.into()),
            }
        };

        Ok(Self {
            stream,
            base,
            read_timeout: options.read_timeout,
            peer_ip: addr.ip(),
            data_tls,
            session: SessionState::default(),
            poisoned: false,
        })
    }

    fn absolute(&self, relative: &str) -> String {
        path::join_remote(&self.base, relative)
    }

    fn ensure_usable(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::ConnectionPoisoned)
        } else {
            Ok(())
        }
    }

    fn resync_timeout(&self) -> Duration {
        self.read_timeout
            .unwrap_or(RESYNC_TIMEOUT)
            .min(RESYNC_TIMEOUT)
    }

    /// Resynchronises the control connection after a transfer stream was dropped mid-way:
    /// the next command reads the transfer's reply first, then `NOOP` must answer. Poisons
    /// the transport when that fails.
    ///
    /// `ABOR` is not used: some servers (libunftp) answer it and then still send the reply
    /// of the interrupted transfer, which desynchronises the control connection.
    async fn resync(&mut self) {
        let timeout = self.resync_timeout();
        match tokio::time::timeout(timeout, self.stream.noop()).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::warn!(%err, "FTP control connection lost after an interrupted transfer");
                self.poisoned = true;
            }
            Err(_elapsed) => {
                tracing::warn!(
                    "FTP control connection did not answer after an interrupted transfer"
                );
                self.poisoned = true;
            }
        }
    }

    /// Reads the final reply of a data transfer opened by hand; poisons the transport when
    /// no reply arrives.
    async fn drain_manual_reply(&mut self) {
        let timeout = self.resync_timeout();
        let reply = tokio::time::timeout(
            timeout,
            self.stream
                .read_response_in(&[Status::ClosingDataConnection, Status::RequestedFileActionOk]),
        )
        .await;
        match reply {
            Ok(Ok(_) | Err(FtpError::UnexpectedResponse(_))) => {}
            _ => self.poisoned = true,
        }
    }

    /// Marks the transport poisoned when a completion reply timed out: the reply may still
    /// arrive later and desynchronise the control connection.
    fn poison_on_timeout<T>(&mut self, result: Result<T>) -> Result<T> {
        if matches!(result, Err(Error::Timeout)) {
            self.poisoned = true;
        }
        result
    }

    async fn retrieve<W>(
        &mut self,
        relative: &str,
        writer: &mut W,
        ctx: &TransferCtx,
    ) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin + ?Sized,
    {
        self.ensure_usable()?;
        let absolute = self.absolute(relative);
        let mut transfer = self
            .stream
            .retr_as_stream(&absolute)
            .await
            .map_err(|e| map_ftp_error(e, relative))?;
        let copied = copy(&mut transfer, writer, ctx, self.read_timeout).await;
        match copied {
            Ok(n) => {
                let finished = with_timeout(self.read_timeout, transfer.finish()).await;
                self.poison_on_timeout(finished)?;
                Ok(n)
            }
            Err(err) => {
                // Dropping the transfer stream closes the data connection; its reply is read
                // before the next command.
                drop(transfer);
                self.resync().await;
                Err(err)
            }
        }
    }

    pub async fn fetch(&mut self, relative: &str, ctx: &TransferCtx) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.retrieve(relative, &mut out, ctx).await?;
        Ok(out)
    }

    pub async fn download_file(
        &mut self,
        relative: &str,
        dest: &Path,
        resume_from: u64,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        self.ensure_usable()?;
        if resume_from > 0 {
            match self
                .download_resumed(relative, dest, resume_from, ctx)
                .await?
            {
                Resume::Done(len) => return Ok(len),
                Resume::Restart => {
                    tracing::warn!(
                        relative,
                        resume_from,
                        "FTP download cannot be resumed safely; downloading from the start"
                    );
                }
            }
        }
        let mut file = crate::open_destination(dest, 0).await?;
        let written = self.retrieve(relative, &mut file, ctx).await?;
        file.sync_all().await?;
        Ok(written)
    }

    /// Size of a remote file from `SIZE`, else from the `size=` fact of `MLST`; `None` when
    /// the server answers neither (or the file is missing).
    async fn remote_size(&mut self, absolute: &str) -> Result<Option<u64>> {
        match self
            .stream
            .custom_command(format!("SIZE {absolute}"), &[Status::File])
            .await
        {
            Ok(response) => {
                if let Some(size) = reply_text(&response)
                    .get(3..)
                    .and_then(|s| s.trim().parse().ok())
                {
                    return Ok(Some(size));
                }
            }
            Err(FtpError::UnexpectedResponse(_)) => {}
            Err(e) => return Err(e.into()),
        }
        match self.stream.mlst(Some(absolute)).await {
            Ok(facts) => Ok(mlst_size(&facts)),
            Err(FtpError::UnexpectedResponse(_) | FtpError::BadResponse) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Resumes a download at `offset`, sending the commands in the order commons-net uses
    /// (`FTPClient._openDataConnection_`): `PASV`, connect, `REST`, `RETR`. Servers clear the
    /// restart marker on any command between `REST` and the transfer (RFC 3659 §5.3), so
    /// suppaftp's `REST`-then-`PASV` order made them send the whole file.
    ///
    /// Only resumes when the remote size is known (`SIZE` or `MLST`), so that a server that
    /// still sends more than the remaining bytes is detected; otherwise, or when `PASV` or
    /// `REST` is refused, the download restarts from 0.
    // Mirrors dao/connection/ftp/FtpDAO.java:161 (setRestartOffset), 330-424.
    async fn download_resumed(
        &mut self,
        relative: &str,
        dest: &Path,
        offset: u64,
        ctx: &TransferCtx,
    ) -> Result<Resume> {
        let absolute = self.absolute(relative);
        let Some(size) = self.remote_size(&absolute).await? else {
            return Ok(Resume::Restart);
        };
        if offset > size {
            return Ok(Resume::Restart);
        }
        if offset == size {
            let file = crate::open_destination(dest, size).await?;
            file.sync_all().await?;
            return Ok(Resume::Done(size));
        }

        // PASV, then connect the data socket.
        let reply = match self
            .stream
            .custom_command("PASV", &[Status::PassiveMode])
            .await
        {
            Ok(reply) => reply,
            Err(FtpError::UnexpectedResponse(_)) => return Ok(Resume::Restart),
            Err(e) => return Err(e.into()),
        };
        let Some(port) = parse_pasv_port(&reply_text(&reply)) else {
            return Ok(Resume::Restart);
        };
        let connect = tokio::net::TcpStream::connect((self.peer_ip, port));
        let data = crate::ctx::maybe_timeout(Some(self.resync_timeout()), connect).await?;

        // REST immediately before RETR.
        match self
            .stream
            .custom_command(format!("REST {offset}"), &[Status::RequestFilePending])
            .await
        {
            Ok(_) => {}
            Err(FtpError::UnexpectedResponse(_)) => return Ok(Resume::Restart),
            Err(e) => return Err(e.into()),
        }
        self.stream
            .custom_command(
                format!("RETR {absolute}"),
                &[Status::AboutToSend, Status::AlreadyOpen],
            )
            .await
            .map_err(|e| map_ftp_error(e, relative))?;

        let mut reader: Box<dyn AsyncRead + Send + Unpin> = match &self.data_tls {
            Some((connector, name)) => match connector.connect(name.clone(), data).await {
                Ok(tls) => Box::new(tls),
                Err(e) => {
                    self.drain_manual_reply().await;
                    return Err(Error::Tls(e.to_string()));
                }
            },
            None => Box::new(data),
        };

        let remaining = size - offset;
        let copied = async {
            let mut file = crate::open_destination(dest, offset).await?;
            // One byte more than expected reveals a server that ignored REST.
            let mut limited = (&mut reader).take(remaining + 1);
            let n = copy(&mut limited, &mut file, ctx, self.read_timeout).await?;
            file.sync_all().await?;
            Ok::<_, Error>(n)
        }
        .await;
        drop(reader);

        match copied {
            Ok(n) if n > remaining => {
                self.drain_manual_reply().await;
                self.ensure_usable()?;
                ctx.progress
                    .fetch_sub(n, std::sync::atomic::Ordering::Relaxed);
                Ok(Resume::Restart)
            }
            Ok(n) => {
                let reply = with_timeout(
                    self.read_timeout,
                    self.stream.read_response_in(&[
                        Status::ClosingDataConnection,
                        Status::RequestedFileActionOk,
                    ]),
                )
                .await;
                self.poison_on_timeout(reply)?;
                let total = offset + n;
                // Mirrors dao/connection/ftp/FtpDAO.java:387-393 (size check after the
                // transfer).
                if total != size {
                    return Err(Error::Incomplete {
                        expected: size,
                        actual: total,
                    });
                }
                Ok(Resume::Done(total))
            }
            Err(err) => {
                self.drain_manual_reply().await;
                Err(err)
            }
        }
    }

    /// Lists the directory `absolute_dir` like Java (`FtpDAO.java:460-462`): `CWD` into it,
    /// `LIST` without argument (servers split `LIST` arguments on spaces), `CWD` back to the
    /// base. With `all`, `LIST -a` is tried first so dot-files are included.
    async fn list_dir(&mut self, absolute_dir: &str, all: bool) -> Result<Listing> {
        if !self.try_cwd(absolute_dir).await? {
            return Ok(Listing::Missing);
        }
        let mut lines = if all {
            self.stream.list(Some("-a")).await
        } else {
            self.stream.list(None).await
        };
        if all && matches!(lines, Err(FtpError::UnexpectedResponse(_))) {
            lines = self.stream.list(None).await;
        }
        let base = self.base.clone();
        match self.stream.cwd(&base).await {
            // Every command uses absolute paths: a refused CWD back does not invalidate the
            // listing.
            Ok(()) | Err(FtpError::UnexpectedResponse(_)) => {}
            Err(e) => return Err(e.into()),
        }
        match lines {
            Ok(lines) => Ok(parse_listing(&lines).map_or(Listing::Unusable, Listing::Entries)),
            // The directory exists (CWD succeeded): some servers answer an empty one so.
            Err(e) if matches!(reply_code(&e), Some(450 | 550)) => Ok(Listing::Entries(Vec::new())),
            Err(FtpError::UnexpectedResponse(response)) => {
                tracing::warn!(
                    directory = absolute_dir,
                    reply = reply_text(&response),
                    "FTP directory listing unavailable"
                );
                Ok(Listing::Unusable)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Names in the directory `absolute_dir`: empty when it is missing, `None` when the
    /// listing is unusable.
    // Mirrors dao/connection/ftp/FtpDAO.java:460-477.
    async fn list_names(&mut self, absolute_dir: &str) -> Result<Option<HashSet<String>>> {
        Ok(match self.list_dir(absolute_dir, false).await? {
            Listing::Entries(entries) => {
                Some(entries.iter().map(|e| e.name().to_owned()).collect())
            }
            Listing::Missing => Some(HashSet::new()),
            Listing::Unusable => None,
        })
    }

    // Mirrors dao/connection/ftp/FtpDAO.java:430-458.
    pub async fn exists(&mut self, relative: &str, _ctx: &TransferCtx) -> Result<bool> {
        self.ensure_usable()?;
        let (parent, name) = path::split_parent(relative);
        if self.session.is_active() {
            let mut found = self
                .session
                .cached_listing(parent)
                .map(|names| names.contains(name));
            if found.is_none()
                && let Some(names) = self.list_names(&self.absolute(parent)).await?
            {
                self.session.store_listing(parent, names);
                found = self
                    .session
                    .cached_listing(parent)
                    .map(|names| names.contains(name));
            }
            match found {
                Some(true) => return Ok(true),
                // Servers hide dot-files from LIST: confirm those with MLST.
                Some(false) if !name.starts_with('.') => return Ok(false),
                _ => {}
            }
        }
        self.exists_absolute(&self.absolute(relative)).await
    }

    /// `MLST`, or an uncached listing of the parent when the server has no `MLST`.
    // Mirrors dao/connection/ftp/FtpDAO.java:479-494.
    async fn exists_absolute(&mut self, absolute: &str) -> Result<bool> {
        let code = match self.stream.mlst(Some(absolute)).await {
            // BadResponse: a 250 reply without a fact line.
            Ok(_) | Err(FtpError::BadResponse) => return Ok(true),
            Err(FtpError::UnexpectedResponse(response)) => response,
            Err(e) => return Err(e.into()),
        };
        match mlst_outcome(code.status.code()) {
            MlstOutcome::Found => Ok(true),
            MlstOutcome::Missing => Ok(false),
            MlstOutcome::Fallback => {
                let trimmed = absolute.trim_end_matches('/');
                let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
                let parent = if parent.is_empty() { "/" } else { parent };
                Ok(match self.list_dir(parent, true).await? {
                    Listing::Entries(entries) => entries.iter().any(|e| e.name() == name),
                    Listing::Missing | Listing::Unusable => false,
                })
            }
            MlstOutcome::Failed => Err(Error::Ftp(FtpError::UnexpectedResponse(code))),
        }
    }

    /// `CWD`: `true` if the directory exists.
    async fn try_cwd(&mut self, absolute: &str) -> Result<bool> {
        match self.stream.cwd(absolute).await {
            Ok(()) => Ok(true),
            Err(FtpError::UnexpectedResponse(_)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Creates the missing directories of `relative_dir` below the base, parents first.
    ///
    /// In a session the deepest known directory of the chain is re-checked with `CWD`: if it
    /// vanished, the whole known chain is forgotten and every segment is checked again
    /// (`FtpDAO.java:631-639`), so missing directories are recreated.
    // Mirrors dao/connection/ftp/FtpDAO.java:621-660.
    pub async fn mkdirs(&mut self, relative_dir: &str, ctx: &TransferCtx) -> Result<()> {
        self.ensure_usable()?;
        let mut prefixes: Vec<String> = Vec::new();
        for segment in path::segments(relative_dir) {
            let parent = prefixes.last().map_or("", String::as_str);
            prefixes.push(path::join_relative(parent, segment));
        }
        let mut start = 0;
        if let Some(known) = prefixes
            .iter()
            .rposition(|dir| self.session.is_known_dir(dir))
        {
            ctx.check_cancelled()?;
            if self.try_cwd(&self.absolute(&prefixes[known])).await? {
                // Its ancestors exist too.
                start = known + 1;
            } else {
                self.session.forget_chain(&prefixes[known]);
            }
        }
        for index in start..prefixes.len() {
            let current = &prefixes[index];
            let parent = if index == 0 { "" } else { &prefixes[index - 1] };
            let segment = path::split_parent(current).1;
            ctx.check_cancelled()?;
            let absolute = self.absolute(current);
            if self.try_cwd(&absolute).await? {
                self.session.mark_known_dir(current);
                continue;
            }
            match self.stream.mkdir(&absolute).await {
                Ok(()) => self.session.mark_created_dir(current, parent, segment),
                Err(err @ FtpError::UnexpectedResponse(_)) => {
                    // Another connection may have created it meanwhile.
                    if self.try_cwd(&absolute).await? {
                        self.session.mark_known_dir(current);
                    } else {
                        return Err(remote_error(err, "create directory", current));
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    // Mirrors dao/connection/ftp/FtpDAO.java:496-619.
    pub(crate) async fn upload(
        &mut self,
        mut source: UploadSource,
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        self.ensure_usable()?;
        if !relative.is_ascii() {
            // Java (commons-net) sends ISO-8859-1 names; suppaftp sends UTF-8.
            tracing::warn!(
                relative,
                "non-ASCII FTP path: names are sent as UTF-8, Java clients use ISO-8859-1"
            );
        }
        let (parent, name) = path::split_parent(relative);
        // `mkdirs` enters a known parent with CWD before the transfer and recreates it if it
        // vanished meanwhile.
        self.mkdirs(parent, ctx).await?;
        let written = self.store(&mut source, relative, ctx).await?;
        self.session.add_entry(parent, name);
        Ok(written)
    }

    /// `STOR <absolute>` and copies `source`.
    async fn store(
        &mut self,
        source: &mut UploadSource,
        relative: &str,
        ctx: &TransferCtx,
    ) -> Result<u64> {
        ctx.check_cancelled()?;
        let absolute = self.absolute(relative);
        let mut transfer = self
            .stream
            .put_with_stream(&absolute)
            .await
            .map_err(|e| map_ftp_error(e, relative))?;
        match copy_upload(&mut source.reader, &mut transfer, ctx, self.read_timeout).await {
            Ok(n) => {
                let finished = with_timeout(self.read_timeout, transfer.finish()).await;
                self.poison_on_timeout(finished)?;
                Ok(n)
            }
            Err(err) => {
                drop(transfer);
                self.resync().await;
                Err(err)
            }
        }
    }

    // Mirrors dao/connection/ftp/FtpDAO.java:683-725.
    pub async fn delete(
        &mut self,
        relative: &str,
        kind: RemoteKind,
        ctx: &TransferCtx,
    ) -> Result<()> {
        self.ensure_usable()?;
        let absolute = self.absolute(relative);
        match kind {
            RemoteKind::File => self.delete_file(&absolute).await?,
            RemoteKind::Directory => self.delete_dir(absolute, ctx).await?,
        }
        let (parent, name) = path::split_parent(relative);
        self.session.remove_entry(parent, name, relative);
        Ok(())
    }

    /// `DELE`; a 550 is only accepted when the file is confirmed missing.
    async fn delete_file(&mut self, absolute: &str) -> Result<()> {
        match self.stream.rm(absolute).await {
            Ok(()) => Ok(()),
            Err(e) if reply_code(&e) == Some(550) && !self.exists_absolute(absolute).await? => {
                Ok(())
            }
            Err(e) => Err(remote_error(e, "delete file", absolute)),
        }
    }

    /// Deletes the contents of `absolute` recursively (`CWD` + `LIST -a`), then `RMD`. A
    /// directory that cannot be entered is only treated as missing when `MLST` (or the
    /// parent's listing) confirms it.
    fn delete_dir<'a>(
        &'a mut self,
        absolute: String,
        ctx: &'a TransferCtx,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            ctx.check_cancelled()?;
            let entries = match self.list_dir(&absolute, true).await? {
                Listing::Entries(entries) => entries,
                Listing::Missing => {
                    if self.exists_absolute(&absolute).await? {
                        return Err(Error::Remote {
                            operation: "list directory",
                            path: absolute,
                            reason: "cannot enter the directory".into(),
                        });
                    }
                    return Ok(());
                }
                Listing::Unusable => {
                    return Err(Error::Remote {
                        operation: "list directory",
                        path: absolute,
                        reason: "unrecognised listing format".into(),
                    });
                }
            };
            for entry in entries {
                let child = format!("{}/{}", absolute.trim_end_matches('/'), entry.name());
                if entry.is_directory() {
                    self.delete_dir(child, ctx).await?;
                } else {
                    self.delete_file(&child).await?;
                }
            }
            match self.stream.rmdir(&absolute).await {
                Ok(()) => Ok(()),
                // 550 is fine only if the directory is really gone (not, e.g., still holding
                // entries hidden from the listing).
                Err(e) if reply_code(&e) == Some(550) && !self.try_cwd(&absolute).await? => Ok(()),
                Err(e) => Err(remote_error(e, "remove directory", &absolute)),
            }
        })
    }

    pub async fn quit(mut self) {
        let _ignored = self.stream.quit().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{MlstOutcome, mlst_outcome, mlst_size, parse_listing, parse_pasv_port};

    #[test]
    fn parses_mlst_size() {
        assert_eq!(
            mlst_size("type=file;size=1033;modify=20201019151930; a b.pbo"),
            Some(1033)
        );
        assert_eq!(mlst_size("Type=file;Size=7; x"), Some(7));
        assert_eq!(mlst_size("type=dir;modify=1; d"), None);
    }

    #[test]
    fn mlst_reply_decision() {
        assert_eq!(mlst_outcome(250), MlstOutcome::Found);
        assert_eq!(mlst_outcome(550), MlstOutcome::Missing);
        for code in [500, 501, 502, 504] {
            assert_eq!(mlst_outcome(code), MlstOutcome::Fallback);
        }
        assert_eq!(mlst_outcome(530), MlstOutcome::Failed);
        assert_eq!(mlst_outcome(421), MlstOutcome::Failed);
    }

    #[test]
    fn parses_pasv_replies() {
        assert_eq!(
            parse_pasv_port("227 Entering Passive Mode (127,0,0,1,195,80)."),
            Some(195 * 256 + 80)
        );
        assert_eq!(
            parse_pasv_port("227 Entering Passive Mode 10,0,0,1,4,1"),
            Some(1025)
        );
        assert_eq!(parse_pasv_port("227 nothing"), None);
        assert_eq!(parse_pasv_port("227 (1,2,3,4,999,1)"), None);
    }

    #[test]
    fn parses_listings() {
        let lines = vec![
            "total 2".to_owned(),
            "drwxr-xr-x 1 owner group 0 Nov 5 2018 sub dir".to_owned(),
            "-rw-r--r-- 1 owner group 8192 Nov 5 2018 a b.pbo".to_owned(),
            "drwxr-xr-x 1 owner group 0 Nov 5 2018 .".to_owned(),
        ];
        let entries = parse_listing(&lines).unwrap();
        let names: Vec<_> = entries
            .iter()
            .map(|e| (e.name(), e.is_directory()))
            .collect();
        assert_eq!(names, [("sub dir", true), ("a b.pbo", false)]);
        assert!(parse_listing(&["garbage line".to_owned()]).is_none());
        assert!(parse_listing(&["total 0".to_owned()]).unwrap().is_empty());
    }
}
