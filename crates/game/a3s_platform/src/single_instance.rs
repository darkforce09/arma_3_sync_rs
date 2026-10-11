//! One running client per user and configuration folder: the first instance listens on a local
//! socket; a second one asks it to come to the front and then exits.
//!
//! The socket is a named pipe on Windows, an abstract socket on Linux and a socket file on other
//! Unixes. A process squatting on the name cannot stop the client from starting: if the owner
//! does not answer the handshake, the client starts without the lock.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use a3s_foundation::paths::{APP_DIR_NAME, Os};
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerOptions, Name};

/// Printed when another instance holds the lock.
// Mirrors main/ArmA3Sync.java:217-219
pub const MSG_ALREADY_LOCKED: &str = "Arma3Sync process is already locked.";

/// Printed before a second instance exits.
// Mirrors main/ArmA3Sync.java:249-253
pub const MSG_ALREADY_RUNNING: &str = "Can not start Arma3Sync. Process is already running.";

/// Exit code of a second instance.
pub const EXIT_CODE_ALREADY_RUNNING: i32 = 1;

/// The handshake a second instance sends.
const HELLO_LINE: &str = "a3s-instance/1 activate\n";

/// The owner's answer to [`HELLO_LINE`].
const REPLY_LINE: &str = "ok\n";

/// Longest request line the owner reads.
const MAX_REQUEST_BYTES: usize = 256;

/// The identity of the lock: one per configuration folder, so different users and portable
/// installs do not block each other.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceId(String);

impl InstanceId {
    /// `arma3sync-rs-<FNV-1a-64 of the folder path in hex>`; the path is lowercased first on
    /// Windows and macOS, whose file systems ignore case.
    pub fn for_config_dir(dir: &Path, os: Os) -> Self {
        let text = dir.to_string_lossy();
        let text = match os {
            Os::Windows | Os::MacOs => text.to_lowercase(),
            Os::Linux | Os::OtherUnix | Os::Other => text.into_owned(),
        };
        Self(format!("{APP_DIR_NAME}-{:016x}", fnv1a_64(text.as_bytes())))
    }

    /// An id from a raw string.
    pub fn from_raw(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// A message from another instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstanceMessage {
    /// Bring the window to the front.
    Activate,
}

/// How hard [`acquire`] tries.
#[derive(Clone, Debug)]
pub struct AcquireOptions {
    /// Listen-or-handshake attempts before giving up.
    pub attempts: u32,

    /// Pause between attempts.
    pub retry_delay: Duration,

    /// How long to wait for the owner's answer.
    pub reply_timeout: Duration,

    /// Folder of the socket file on Unixes other than Linux (the cache folder); the system
    /// temporary folder when `None` or when the path would be too long.
    pub socket_dir: Option<PathBuf>,
}

impl Default for AcquireOptions {
    fn default() -> Self {
        Self {
            attempts: 5,
            retry_delay: Duration::from_millis(100),
            reply_timeout: Duration::from_secs(2),
            socket_dir: None,
        }
    }
}

/// The outcome of [`acquire`].
#[derive(Debug)]
pub enum Acquire {
    /// This process holds the lock until the guard is dropped.
    Primary(InstanceGuard),

    /// Another instance holds the lock and was asked to come to the front: exit with
    /// [`EXIT_CODE_ALREADY_RUNNING`].
    Secondary,

    /// The lock could not be taken nor its owner reached; start normally.
    Unlocked {
        /// Why, for the log.
        reason: String,
    },
}

/// Takes the single-instance lock `id`, or asks its owner to come to the front.
///
/// `on_message` runs on a background thread for every request of a later instance.
pub fn acquire(
    id: &InstanceId,
    options: &AcquireOptions,
    on_message: impl Fn(InstanceMessage) + Send + Sync + 'static,
) -> Acquire {
    let unlocked = |reason: String| {
        tracing::warn!("starting without the single-instance lock: {reason}");
        Acquire::Unlocked { reason }
    };
    let (name, socket_file) = match socket_name(id, options) {
        Ok(name) => name,
        Err(err) => return unlocked(format!("invalid socket name: {err}")),
    };
    let on_message: Arc<dyn Fn(InstanceMessage) + Send + Sync> = Arc::new(on_message);
    let attempts = options.attempts.max(1);
    let mut last_error = String::new();
    for attempt in 0..attempts {
        if attempt > 0 {
            thread::sleep(options.retry_delay);
        }
        let listener = ListenerOptions::new()
            .name(name.clone())
            .reclaim_name(true)
            .create_sync();
        match listener {
            Ok(listener) => {
                return match serve(listener, name, Arc::clone(&on_message)) {
                    Ok(guard) => Acquire::Primary(guard),
                    Err(err) => unlocked(format!("cannot start the listener thread: {err}")),
                };
            }
            // Windows reports an existing first pipe instance as access denied.
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
                ) =>
            {
                match ask_owner(&name, options.reply_timeout) {
                    Ok(()) => return Acquire::Secondary,
                    Err(err) => {
                        if err.kind() == io::ErrorKind::ConnectionRefused
                            && let Some(file) = &socket_file
                        {
                            remove_stale_socket(file);
                        }
                        last_error = err.to_string();
                    }
                }
            }
            Err(err) => return unlocked(format!("cannot listen: {err}")),
        }
    }
    unlocked(format!(
        "the lock is held but its owner did not answer ({last_error})"
    ))
}

/// The socket name, and the socket file when the name is a path.
#[cfg(any(windows, target_os = "linux", target_os = "android"))]
fn socket_name(
    id: &InstanceId,
    _options: &AcquireOptions,
) -> io::Result<(Name<'static>, Option<PathBuf>)> {
    use interprocess::local_socket::GenericNamespaced;
    let name = id.as_str().to_owned().to_ns_name::<GenericNamespaced>()?;
    Ok((name, None))
}

/// The socket name, and the socket file when the name is a path.
#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
fn socket_name(
    id: &InstanceId,
    options: &AcquireOptions,
) -> io::Result<(Name<'static>, Option<PathBuf>)> {
    use interprocess::local_socket::GenericFilePath;

    /// Longest socket file path accepted under the socket folder (`sun_path` is 104 bytes on
    /// macOS).
    const MAX_SOCKET_PATH_BYTES: usize = 100;

    /// A rendezvous point every instance of this user computes alike, not a temporary file.
    #[expect(
        clippy::disallowed_methods,
        reason = "the socket path must be predictable"
    )]
    fn system_temp_dir() -> PathBuf {
        std::env::temp_dir()
    }

    let file_name = format!("{}.sock", id.as_str());
    let in_socket_dir = options
        .socket_dir
        .as_ref()
        .map(|dir| dir.join(&file_name))
        .filter(|path| path.as_os_str().len() <= MAX_SOCKET_PATH_BYTES);
    let path = match in_socket_dir {
        Some(path) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            path
        }
        None => system_temp_dir().join(&file_name),
    };
    let name = path.clone().to_fs_name::<GenericFilePath>()?;
    Ok((name, Some(path)))
}

/// Removes a socket file left by a crashed owner (only if it is a socket).
#[cfg(unix)]
fn remove_stale_socket(path: &Path) {
    use std::os::unix::fs::FileTypeExt as _;
    let is_socket = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_socket());
    if is_socket && let Err(err) = std::fs::remove_file(path) {
        tracing::debug!("cannot remove the stale socket {}: {err}", path.display());
    }
}

#[cfg(not(unix))]
fn remove_stale_socket(_path: &Path) {}

/// Sends the handshake to the lock owner and waits up to `timeout` for its answer. The exchange
/// runs on a helper thread because blocking local sockets have no timeouts.
fn ask_owner(name: &Name<'static>, timeout: Duration) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let name = name.clone();
    thread::Builder::new()
        .name("a3s-single-instance-client".to_owned())
        .spawn(move || {
            // The receiver is gone after a timeout; nobody needs the late answer.
            let _ignored = sender.send(handshake(name));
        })?;
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the lock owner did not answer in time",
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other(
            "the handshake thread ended without an answer",
        )),
    }
}

fn handshake(name: Name<'static>) -> io::Result<()> {
    let mut stream = LocalSocketStream::connect(name)?;
    stream.write_all(HELLO_LINE.as_bytes())?;
    stream.flush()?;
    let reply = read_line(&mut stream, REPLY_LINE.len())?;
    if reply.as_deref() == Some(REPLY_LINE.trim_end_matches('\n').as_bytes()) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected answer from the lock owner",
        ))
    }
}

/// Reads up to `max` bytes or the first `\n`; the line without its `\n`, or `None` when the
/// stream ended or the limit was reached first.
fn read_line(stream: &mut impl io::Read, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    while line.len() < max {
        if stream.read(&mut byte)? == 0 {
            return Ok(None);
        }
        if byte[0] == b'\n' {
            return Ok(Some(line));
        }
        line.push(byte[0]);
    }
    Ok(None)
}

/// Holds the single-instance lock; dropping it (or [`Self::shutdown`]) releases the name.
#[derive(Debug)]
pub struct InstanceGuard {
    stop: Arc<AtomicBool>,
    name: Name<'static>,
    thread: Option<thread::JoinHandle<()>>,
}

impl InstanceGuard {
    /// Stops listening and releases the name.
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let Some(thread) = self.thread.take() else {
            return;
        };
        // Wake the blocking `accept` with a connection of our own; the thread then sees the flag.
        match LocalSocketStream::connect(self.name.clone()) {
            Ok(_wake) => {
                if thread.join().is_err() {
                    tracing::warn!("the single-instance listener thread panicked");
                }
            }
            Err(err) => tracing::warn!(
                "cannot wake the single-instance listener, leaving it running: {err}"
            ),
        }
    }
}

/// Starts the accept loop on its own thread.
fn serve(
    listener: LocalSocketListener,
    name: Name<'static>,
    on_message: Arc<dyn Fn(InstanceMessage) + Send + Sync>,
) -> io::Result<InstanceGuard> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = thread::Builder::new()
        .name("a3s-single-instance".to_owned())
        .spawn(move || accept_loop(&listener, &thread_stop, &on_message))?;
    Ok(InstanceGuard {
        stop,
        name,
        thread: Some(thread),
    })
}

fn accept_loop(
    listener: &LocalSocketListener,
    stop: &AtomicBool,
    on_message: &Arc<dyn Fn(InstanceMessage) + Send + Sync>,
) {
    loop {
        let accepted = listener.accept();
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match accepted {
            Ok(stream) => {
                // One short-lived thread per client, so a silent client never blocks the loop.
                let on_message = Arc::clone(on_message);
                let spawned = thread::Builder::new()
                    .name("a3s-single-instance-conn".to_owned())
                    .spawn(move || handle_client(stream, on_message.as_ref()));
                if let Err(err) = spawned {
                    tracing::warn!("cannot handle a single-instance request: {err}");
                }
            }
            Err(err) => {
                tracing::debug!("single-instance accept failed: {err}");
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Answers a valid handshake with `ok` and reports it; anything else is closed silently.
fn handle_client(
    mut stream: LocalSocketStream,
    on_message: &(dyn Fn(InstanceMessage) + Send + Sync),
) {
    let request = match read_line(&mut stream, MAX_REQUEST_BYTES) {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(err) => {
            tracing::debug!("cannot read a single-instance request: {err}");
            return;
        }
    };
    if request != HELLO_LINE.trim_end_matches('\n').as_bytes() {
        tracing::debug!("ignoring an unknown single-instance request");
        return;
    }
    if let Err(err) = stream
        .write_all(REPLY_LINE.as_bytes())
        .and_then(|()| stream.flush())
    {
        tracing::debug!("cannot answer a single-instance request: {err}");
    }
    on_message(InstanceMessage::Activate);
}
