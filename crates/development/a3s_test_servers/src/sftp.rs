//! russh + russh-sftp SFTP server jailed to a tempdir, password authentication.

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_types::{Protocol, ProtocolType};
use russh::keys::PrivateKey;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

use crate::{AbortOnDrop, Fixture, outer_with_root};

pub const USER: &str = "a3s";
pub const PASSWORD: &str = "secret";

struct SshHandler {
    root: Arc<PathBuf>,
    home: Arc<String>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for SshHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == USER && password == PASSWORD {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match (name, self.channels.remove(&channel_id)) {
            ("sftp", Some(channel)) => {
                session.channel_success(channel_id)?;
                let handler = SftpHandler {
                    root: Arc::clone(&self.root),
                    home: Arc::clone(&self.home),
                    handles: HashMap::new(),
                    next_handle: 0,
                };
                russh_sftp::server::run(channel.into_stream(), handler).await;
            }
            _ => session.channel_failure(channel_id)?,
        }
        Ok(())
    }
}

enum OpenHandle {
    File(tokio::fs::File),
    Dir(Option<Vec<File>>),
}

struct SftpHandler {
    root: Arc<PathBuf>,
    home: Arc<String>,
    handles: HashMap<String, OpenHandle>,
    next_handle: u64,
}

fn io_status(err: &std::io::Error) -> StatusCode {
    match err.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en-US".to_owned(),
    }
}

impl SftpHandler {
    /// Absolute virtual path (relative paths are below the home directory).
    fn virtual_path(&self, path: &str) -> Result<String, StatusCode> {
        let full = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("{}/{path}", self.home)
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in full.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(StatusCode::PermissionDenied),
                other => parts.push(other),
            }
        }
        Ok(format!("/{}", parts.join("/")))
    }

    fn local(&self, path: &str) -> Result<PathBuf, StatusCode> {
        let virtual_path = self.virtual_path(path)?;
        Ok(self.root.join(virtual_path.trim_start_matches('/')))
    }

    fn add_handle(&mut self, handle: OpenHandle) -> String {
        self.next_handle += 1;
        let key = self.next_handle.to_string();
        self.handles.insert(key.clone(), handle);
        key
    }

    async fn attrs(path: &Path) -> Result<FileAttributes, StatusCode> {
        let metadata = tokio::fs::metadata(path).await.map_err(|e| io_status(&e))?;
        Ok(FileAttributes::from(&metadata))
    }
}

impl russh_sftp::server::Handler for SftpHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let path = self.local(&filename)?;
        let options = tokio::fs::OpenOptions::from(std::fs::OpenOptions::from(pflags));
        let file = options.open(&path).await.map_err(|e| io_status(&e))?;
        let handle = self.add_handle(OpenHandle::File(file));
        Ok(Handle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        match self.handles.remove(&handle) {
            Some(OpenHandle::File(mut file)) => {
                file.flush().await.map_err(|e| io_status(&e))?;
                Ok(ok(id))
            }
            Some(OpenHandle::Dir(_)) => Ok(ok(id)),
            None => Err(StatusCode::Failure),
        }
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let Some(OpenHandle::File(file)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| io_status(&e))?;
        let mut data = vec![0u8; len.min(256 * 1024) as usize];
        let n = file.read(&mut data).await.map_err(|e| io_status(&e))?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(n);
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let Some(OpenHandle::File(file)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| io_status(&e))?;
        file.write_all(&data).await.map_err(|e| io_status(&e))?;
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let local = self.local(&path)?;
        Ok(Attrs {
            id,
            attrs: Self::attrs(&local).await?,
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let Some(OpenHandle::File(file)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure);
        };
        let metadata = file.metadata().await.map_err(|e| io_status(&e))?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        _path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        _handle: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let local = self.local(&path)?;
        let mut reader = tokio::fs::read_dir(&local)
            .await
            .map_err(|e| io_status(&e))?;
        let mut files = Vec::new();
        while let Some(entry) = reader.next_entry().await.map_err(|e| io_status(&e))? {
            let metadata = entry.metadata().await.map_err(|e| io_status(&e))?;
            files.push(File::new(
                entry.file_name().to_string_lossy(),
                FileAttributes::from(&metadata),
            ));
        }
        let handle = self.add_handle(OpenHandle::Dir(Some(files)));
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Dir(files)) => match files.take() {
                Some(files) => Ok(Name { id, files }),
                None => Err(StatusCode::Eof),
            },
            _ => Err(StatusCode::Failure),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let local = self.local(&filename)?;
        tokio::fs::remove_file(&local)
            .await
            .map_err(|e| io_status(&e))?;
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let local = self.local(&path)?;
        tokio::fs::create_dir(&local)
            .await
            .map_err(|e| io_status(&e))?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let local = self.local(&path)?;
        tokio::fs::remove_dir(&local)
            .await
            .map_err(|e| io_status(&e))?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let virtual_path = self.virtual_path(&path)?;
        Ok(Name {
            id,
            files: vec![File::dummy(virtual_path)],
        })
    }
}

/// Serves `root` with the given home directory (virtual path, e.g. `/` or `/home/a3s`).
pub async fn serve(root: PathBuf, home: &str) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind SFTP");
    let port = listener.local_addr().expect("local addr").port();
    let key = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
    let config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![key],
        ..Default::default()
    });
    let root = Arc::new(root);
    let home = Arc::new(home.to_owned());
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = SshHandler {
                root: Arc::clone(&root),
                home: Arc::clone(&home),
                channels: HashMap::new(),
            };
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, stream, handler).await {
                    let _ignored = session.await;
                }
            });
        }
    });
    (port, task)
}

/// SFTP fixture. `home` is the server's home directory, `url_path` is appended to the
/// protocol URL and `base` is the server-side base directory relative to the root.
pub async fn fixture_with(home: &str, url_path: &str, password: &str, base: &str) -> Fixture {
    let (outer, root) = outer_with_root();
    std::fs::create_dir_all(root.join(home.trim_start_matches('/'))).expect("create home");
    let (port, task) = serve(root.clone(), home).await;
    let protocol = Protocol::new(
        ProtocolType::Sftp,
        format!("127.0.0.1{url_path}"),
        port.to_string(),
        USER,
        password,
        true,
    );
    Fixture {
        protocol,
        base: root.join(base.trim_start_matches('/')),
        outer,
        request_count: None,
        server: Box::new(AbortOnDrop(task)),
    }
}

pub async fn fixture() -> Fixture {
    fixture_with("/", "", PASSWORD, "").await
}
