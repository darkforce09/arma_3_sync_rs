//! The RPT viewer's engine: preloads an Arma 3 `.rpt` log and then follows it like `tail -f`,
//! surviving appends, truncation and rotation.
//!
//! [`LogTailer`] is the pollable core; [`RptTail`] runs it on a named background thread that
//! stops (and is joined) on [`RptTail::stop`] or drop.

use std::fs::{File, Metadata};
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Title of the viewer window before a file is chosen.
pub const VIEWER_TITLE: &str = "RPT Viewer";

/// Name of the tailing thread.
pub const THREAD_NAME: &str = "rpt-tail";

/// How often the viewer polls the log file.
// Mirrors ui/tools/rpt/Tail.java:25
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// Title of the viewer window while it shows `file_name`.
// Mirrors ui/tools/rpt/RptViewerPanel.java:116
pub fn viewer_title_reading(file_name: &str) -> String {
    format!("{VIEWER_TITLE} - Reading {file_name}")
}

/// The text the viewer starts with: the whole file decoded as lossy UTF-8, ending in `\n`.
/// `None` for an empty file.
// Mirrors ui/tools/rpt/Tail.java:44-76
pub fn preload_text(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Some(text)
}

/// What one [`LogTailer::poll`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TailUpdate {
    /// The file was truncated or replaced, and is read again from its start. `lines` may begin
    /// with the unterminated last line of the previous file.
    pub reset: bool,

    /// New complete lines, without their terminators.
    pub lines: Vec<String>,
}

/// Follows a log file by polling.
///
/// Each [`poll`](Self::poll) reopens the path, so a file renamed away and recreated is picked up.
/// Lines end at `\n`, `\r` or `\r\n` and are decoded as lossy UTF-8. An unterminated last line is
/// held back until its terminator arrives, or emitted on the first poll that sees no growth (so
/// the last words of a crashed game still show). A file that shrinks or whose identity changes is
/// read again from the start. A missing file is not an error: it is read from its start once it
/// appears.
#[derive(Debug)]
pub struct LogTailer {
    path: PathBuf,
    /// Bytes of the current file already consumed (complete lines plus `pending`).
    offset: u64,
    /// Identity of the file `offset` refers to, when known.
    identity: Option<FileIdentity>,
    /// The file was missing on the last look.
    absent: bool,
    /// Bytes of an unterminated last line.
    pending: Vec<u8>,
    /// The last byte consumed was a `\r`, so a leading `\n` belongs to it.
    after_cr: bool,
    /// The preload ended inside a line and already showed it with a `\n`, so that line's own
    /// terminator is skipped.
    after_preloaded_fragment: bool,
}

impl LogTailer {
    /// Starts at the current end of the file, or at its start once it appears when it is missing.
    // Mirrors ui/tools/rpt/LogFileTailer.java:96-100
    pub fn at_end(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (offset, identity, absent) = match std::fs::metadata(&path) {
            Ok(meta) => (meta.len(), file_identity(&meta), false),
            Err(_) => (0, None, true),
        };
        Self::new(path, offset, identity, absent)
    }

    /// Starts at byte `offset` of the file.
    pub fn at_offset(path: impl Into<PathBuf>, offset: u64) -> Self {
        let path = path.into();
        let (identity, absent) = match std::fs::metadata(&path) {
            Ok(meta) => (file_identity(&meta), false),
            Err(_) => (None, true),
        };
        Self::new(path, offset, identity, absent)
    }

    /// Reads the whole file for the viewer ([`preload_text`]) and returns a tailer that continues
    /// exactly where that read ended, so no line is lost or shown twice. When the file ended
    /// inside a line, that line's terminator, once written, adds no empty line. A missing file
    /// gives no text and a tailer that waits for it.
    pub fn preload(path: impl Into<PathBuf>) -> io::Result<(Option<String>, Self)> {
        let path = path.into();
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok((None, Self::new(path, 0, None, true)));
            }
            Err(err) => return Err(err),
        };
        let identity = file_identity(&file.metadata()?);
        let mut bytes = Vec::new();
        #[expect(
            clippy::verbose_file_reads,
            reason = "the identity must come from the handle that was read"
        )]
        file.read_to_end(&mut bytes)?;
        let mut tailer = Self::new(path, bytes.len() as u64, identity, false);
        match bytes.last() {
            Some(b'\r') => tailer.after_cr = true,
            Some(b'\n') | None => {}
            Some(_) => tailer.after_preloaded_fragment = true,
        }
        Ok((preload_text(&bytes), tailer))
    }

    fn new(path: PathBuf, offset: u64, identity: Option<FileIdentity>, absent: bool) -> Self {
        Self {
            path,
            offset,
            identity,
            absent,
            pending: Vec::new(),
            after_cr: false,
            after_preloaded_fragment: false,
        }
    }

    /// Reads what was appended since the last poll.
    ///
    /// Errors (other than a missing file) leave the tailer unchanged, so the next poll retries.
    // Mirrors ui/tools/rpt/LogFileTailer.java:106-127
    pub fn poll(&mut self) -> io::Result<TailUpdate> {
        let mut update = TailUpdate::default();
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                self.absent = true;
                self.flush_pending(&mut update);
                return Ok(update);
            }
            Err(err) => return Err(err),
        };
        let meta = file.metadata()?;
        let identity = file_identity(&meta);
        let replaced = match (self.identity, identity) {
            (Some(old), Some(new)) => old != new,
            _ => false,
        };
        // A file that reappears after vanishing is a new file; one that was missing from the
        // start is simply read from its start.
        let reappeared = self.absent && self.offset > 0;
        let reset = replaced || reappeared || meta.len() < self.offset;
        let start = if reset { 0 } else { self.offset };

        let mut appended = Vec::new();
        if meta.len() > start {
            file.seek(SeekFrom::Start(start))?;
            #[expect(clippy::verbose_file_reads, reason = "reads from an offset")]
            file.read_to_end(&mut appended)?;
        }

        if reset {
            self.flush_pending(&mut update);
            update.reset = true;
            self.after_cr = false;
            self.after_preloaded_fragment = false;
        }
        self.identity = identity;
        self.absent = false;
        self.offset = start + appended.len() as u64;

        if appended.is_empty() {
            self.flush_pending(&mut update);
        } else {
            self.split_lines(&appended, &mut update.lines);
        }
        Ok(update)
    }

    /// Appends the complete lines of `pending + bytes` to `lines` and keeps the rest pending.
    fn split_lines(&mut self, bytes: &[u8], lines: &mut Vec<String>) {
        let mut bytes = bytes;
        if self.after_preloaded_fragment && !bytes.is_empty() {
            self.after_preloaded_fragment = false;
            match bytes[0] {
                b'\n' => bytes = &bytes[1..],
                b'\r' => {
                    bytes = &bytes[1..];
                    self.after_cr = true;
                }
                _ => {}
            }
        }
        if self.after_cr && !bytes.is_empty() {
            self.after_cr = false;
            if bytes[0] == b'\n' {
                bytes = &bytes[1..];
            }
        }
        let mut start = 0;
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' => {
                    self.push_line(&bytes[start..index], lines);
                    index += 1;
                    start = index;
                }
                b'\r' => {
                    self.push_line(&bytes[start..index], lines);
                    index += 1;
                    if index == bytes.len() {
                        self.after_cr = true;
                    } else if bytes[index] == b'\n' {
                        index += 1;
                    }
                    start = index;
                }
                _ => index += 1,
            }
        }
        self.pending.extend_from_slice(&bytes[start..]);
    }

    /// Emits `pending + tail` as one line.
    fn push_line(&mut self, tail: &[u8], lines: &mut Vec<String>) {
        if self.pending.is_empty() {
            lines.push(String::from_utf8_lossy(tail).into_owned());
        } else {
            self.pending.extend_from_slice(tail);
            lines.push(String::from_utf8_lossy(&self.pending).into_owned());
            self.pending.clear();
        }
    }

    /// Emits the held-back unterminated line, if any.
    fn flush_pending(&mut self, update: &mut TailUpdate) {
        if !self.pending.is_empty() {
            update
                .lines
                .push(String::from_utf8_lossy(&self.pending).into_owned());
            self.pending.clear();
        }
    }
}

/// What makes a file the same file across polls.
#[cfg(unix)]
type FileIdentity = (u64, u64);

/// What makes a file the same file across polls.
#[cfg(not(unix))]
type FileIdentity = std::time::SystemTime;

#[cfg(unix)]
#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as platforms without an identity"
)]
fn file_identity(meta: &Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn file_identity(meta: &Metadata) -> Option<FileIdentity> {
    meta.created().ok()
}

/// What the tailing thread reports to its sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RptEvent {
    /// The file's content when tailing started, ending in `\n` (see [`preload_text`]).
    Preloaded(String),

    /// The file was truncated or replaced; the following lines come from its start.
    Reset,

    /// New complete lines; the viewer appends each followed by `\n`.
    Lines(Vec<String>),

    /// Reading failed. Reported once per distinct message in a row; polling continues.
    Error(String),
}

/// A log file followed on the [`THREAD_NAME`] thread.
///
/// The thread preloads the file, then polls it every `interval` and sends every event to the
/// sink. [`stop`](Self::stop) and drop both stop and join it.
#[derive(Debug)]
pub struct RptTail {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RptTail {
    /// Starts following `path`. Fails only when the thread cannot be spawned.
    pub fn spawn(
        path: PathBuf,
        interval: Duration,
        mut sink: impl FnMut(RptEvent) + Send + 'static,
    ) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || run(&path, interval, &thread_stop, &mut sink))?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    /// Stops the thread and waits for it to end.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            // A panicking sink already ended the thread; there is nothing left to stop.
            let _ignored: thread::Result<()> = handle.join();
        }
    }
}

impl Drop for RptTail {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The tailing thread's body.
// Mirrors ui/tools/rpt/Tail.java:24-42 and ui/tools/rpt/LogFileTailer.java:91-139
fn run(path: &Path, interval: Duration, stop: &AtomicBool, sink: &mut dyn FnMut(RptEvent)) {
    let mut last_error: Option<String> = None;
    let mut tailer = match LogTailer::preload(path) {
        Ok((text, tailer)) => {
            if let Some(text) = text {
                sink(RptEvent::Preloaded(text));
            }
            tailer
        }
        Err(err) => {
            // The content shows through the tailer once the file becomes readable.
            report_error(&mut last_error, err.to_string(), sink);
            LogTailer::at_offset(path, 0)
        }
    };

    while !stop.load(Ordering::Relaxed) {
        match tailer.poll() {
            Ok(update) => {
                last_error = None;
                if update.reset {
                    sink(RptEvent::Reset);
                }
                if !update.lines.is_empty() {
                    sink(RptEvent::Lines(update.lines));
                }
            }
            Err(err) => report_error(&mut last_error, err.to_string(), sink),
        }
        let deadline = Instant::now() + interval;
        while !stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            thread::park_timeout(deadline - now);
        }
    }
}

/// Sends `message` unless it repeats the previous error.
fn report_error(last: &mut Option<String>, message: String, sink: &mut dyn FnMut(RptEvent)) {
    if last.as_deref() != Some(message.as_str()) {
        sink(RptEvent::Error(message.clone()));
        *last = Some(message);
    }
}
