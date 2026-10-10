use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tracing_subscriber::fmt::MakeWriter;

/// Size limit of one debug log file (Java `FileHandler` limit).
// Mirrors utils/DebugLogger.java:48
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Number of debug log files kept, the current one included (Java `FileHandler` count).
// Mirrors utils/DebugLogger.java:48
pub const LOG_FILE_COUNT: usize = 3;

/// An append-only log file rotated by size: `x.log`, `x.log.1` … `x.log.{keep-1}`.
///
/// When a write would make `x.log` exceed `max_bytes`, `x.log.1` becomes `x.log.2` (the oldest
/// file is dropped), `x.log` becomes `x.log.1` and a new `x.log` is started. A single record is
/// never split across files. I/O errors after opening are swallowed: diagnostics must never
/// affect the application. Cheap to clone; clones share the file.
#[derive(Clone, Debug)]
pub struct RotatingFile {
    path: Arc<Path>,
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    max_bytes: u64,
    keep: usize,
    file: Option<File>,
    len: u64,
}

impl RotatingFile {
    /// Opens (appending) or creates `path`, creating its directory if missing.
    pub fn open(path: PathBuf, max_bytes: u64, keep: usize) -> io::Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let file = open_append(&path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path: path.into(),
            state: Arc::new(Mutex::new(State {
                max_bytes,
                keep: keep.max(1),
                file: Some(file),
                len,
            })),
        })
    }

    /// Path of the current log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes one record, rotating first if needed. Never fails.
    fn write_record(&self, buf: &[u8]) {
        let mut state = self.state.lock();
        let incoming = buf.len() as u64;
        if state.len > 0 && state.len.saturating_add(incoming) > state.max_bytes {
            self.rotate(&mut state);
        } else if state.file.is_none() {
            // A previous reopen failed; try again.
            self.reopen(&mut state);
        }
        if let Some(file) = state.file.as_mut()
            && file.write_all(buf).is_ok()
        {
            state.len = state.len.saturating_add(incoming);
        }
    }

    /// Shifts the numbered files and starts a new current file. Errors are ignored.
    fn rotate(&self, state: &mut State) {
        // Close the current file first (required on Windows to rename it).
        state.file = None;
        let numbered = |n: usize| -> PathBuf {
            let mut name = self.path.as_os_str().to_owned();
            name.push(format!(".{n}"));
            PathBuf::from(name)
        };
        if state.keep > 1 {
            fs::remove_file(numbered(state.keep - 1)).ok();
            for n in (1..state.keep - 1).rev() {
                fs::rename(numbered(n), numbered(n + 1)).ok();
            }
            fs::rename(&self.path, numbered(1)).ok();
        } else {
            fs::remove_file(&self.path).ok();
        }
        self.reopen(state);
    }

    /// (Re)opens the current file; on failure the file stays closed and the next write retries.
    fn reopen(&self, state: &mut State) {
        state.file = open_append(&self.path).ok();
        state.len = state
            .file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |meta| meta.len());
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl io::Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_record(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Writer handed out by [`RotatingFile`] as a [`MakeWriter`].
#[derive(Debug)]
pub struct RotatingWriter<'a>(&'a RotatingFile);

impl io::Write for RotatingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_record(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RotatingFile {
    type Writer = RotatingWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        RotatingWriter(self)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn a_l3_rotation_keeps_three_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("logs").join("x.log");
        let mut file = RotatingFile::open(path.clone(), 100, 3).expect("open");
        for chunk in 0..7u8 {
            file.write_all(&[b'0' + chunk; 50]).expect("write");
        }

        let mut names: Vec<String> = fs::read_dir(path.parent().expect("parent"))
            .expect("read_dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        assert_eq!(names, ["x.log", "x.log.1", "x.log.2"]);
        for name in &names {
            let len = fs::metadata(tmp.path().join("logs").join(name))
                .expect("metadata")
                .len();
            assert!(len <= 100, "{name} has {len} bytes");
        }
        assert_eq!(fs::read(&path).expect("read"), [b'6'; 50]);
        let mut previous = vec![b'4'; 50];
        previous.extend([b'5'; 50]);
        assert_eq!(
            fs::read(tmp.path().join("logs/x.log.1")).expect("read"),
            previous
        );

        // Reopening appends.
        drop(file);
        let mut file = RotatingFile::open(path.clone(), 100, 3).expect("reopen");
        file.write_all(b"tail").expect("write");
        let mut expected = vec![b'6'; 50];
        expected.extend(b"tail");
        assert_eq!(fs::read(&path).expect("read"), expected);
    }

    #[test]
    fn failed_reopen_is_retried_on_next_write() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("logs");
        let path = dir.join("x.log");
        let mut file = RotatingFile::open(path.clone(), 100, 3).expect("open");
        file.write_all(&[b'a'; 60]).expect("write");

        // The directory disappears: the rotation cannot reopen the file; the record is dropped
        // without an error.
        fs::remove_dir_all(&dir).expect("remove dir");
        file.write_all(&[b'b'; 60]).expect("write is swallowed");
        assert!(!path.exists());

        // Once the directory is back, the next write reopens the file.
        fs::create_dir(&dir).expect("recreate dir");
        file.write_all(b"after").expect("write");
        assert_eq!(fs::read(&path).expect("read"), b"after");
    }

    #[test]
    fn rotation_with_keep_one_truncates() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("x.log");
        let mut file = RotatingFile::open(path.clone(), 10, 1).expect("open");
        file.write_all(b"12345678").expect("write");
        file.write_all(b"abcdef").expect("write");
        assert_eq!(fs::read(&path).expect("read"), b"abcdef");
        assert_eq!(fs::read_dir(tmp.path()).expect("read_dir").count(), 1);
    }
}
