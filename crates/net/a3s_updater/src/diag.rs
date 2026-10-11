//! Opt-in diagnostic file log of the updater. Writing it never fails an update.

use std::error::Error;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use a3s_foundation::paths::{AppDirs, PlatformEnv};
use parking_lot::Mutex;

/// File name of the default diagnostic log.
pub(crate) const LOG_FILE_NAME: &str = "arma3sync-updater-debug.log";

/// Serializes appends from every [`DiagnosticLog`] of the process.
static APPEND_LOCK: Mutex<()> = Mutex::new(());

/// Line separator of the log (Java `System.lineSeparator()`).
const LINE_SEPARATOR: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Diagnostic log: every message also goes to `tracing`; the file is written only when the log
/// is enabled and has a path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticLog {
    enabled: bool,
    path: Option<PathBuf>,
}

impl DiagnosticLog {
    /// A log that writes no file.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// A log writing to `path` when `enabled`; without a path, the default location
    /// (`<data dir>/logs/arma3sync-updater-debug.log`) is used.
    pub fn new(enabled: bool, path: Option<PathBuf>) -> Self {
        Self {
            enabled,
            path: path.or_else(default_path),
        }
    }

    /// `A3S_UPDATER_DEBUG=true` (ASCII case-insensitive) enables the log;
    /// `A3S_UPDATER_DEBUG_LOG_PATH` overrides its location.
    // Mirrors a3sUpdater/service/DiagnosticLog.java:21-34
    pub fn from_env() -> Self {
        let enabled =
            std::env::var("A3S_UPDATER_DEBUG").is_ok_and(|v| v.eq_ignore_ascii_case("true"));
        let path = std::env::var_os("A3S_UPDATER_DEBUG_LOG_PATH")
            .filter(|v| !v.to_string_lossy().trim().is_empty())
            .and_then(|v| std::path::absolute(PathBuf::from(v)).ok())
            .map(|p| normalize(&p));
        Self::new(enabled, path)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The log file, if its location is known.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn info(&self, message: &str) {
        tracing::info!(target: "a3s_updater", "{message}");
        self.write("INFO", message, None);
    }

    /// Logs `message` followed by `err` and each of its sources.
    pub fn error(&self, message: &str, err: Option<&dyn Error>) {
        if let Some(err) = err {
            tracing::error!(target: "a3s_updater", error = %err, "{message}");
        } else {
            tracing::error!(target: "a3s_updater", "{message}");
        }
        self.write("ERROR", message, err);
    }

    /// Writes the current timestamp to `marker` (parents created, file truncated), even when
    /// the log is disabled; a failure is logged, never returned.
    // Mirrors a3sUpdater/service/DiagnosticLog.java:44-56
    pub fn mark_elevated_startup(&self, marker: &Path) {
        let written = marker
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(marker, timestamp()));
        if let Err(err) = written {
            self.error(
                "Could not write the elevated updater startup marker.",
                Some(&err),
            );
        }
    }

    /// Appends `<timestamp> [<thread>] [<level>] <message>`, then for an error one line with the
    /// error and one `Caused by: ` line per distinct source.
    // Mirrors a3sUpdater/service/DiagnosticLog.java:58-80
    fn write(&self, level: &str, message: &str, err: Option<&dyn Error>) {
        let (true, Some(path)) = (self.enabled, &self.path) else {
            return;
        };
        let thread = std::thread::current();
        let mut line = format!(
            "{} [{}] [{level}] {message}",
            timestamp(),
            thread.name().unwrap_or("unnamed")
        );
        if let Some(err) = err {
            let mut previous = err.to_string();
            line.push_str(LINE_SEPARATOR);
            line.push_str(&previous);
            let mut source = err.source();
            while let Some(cause) = source {
                let text = cause.to_string();
                if !text.is_empty() && !previous.ends_with(&text) {
                    line.push_str(LINE_SEPARATOR);
                    line.push_str("Caused by: ");
                    line.push_str(&text);
                }
                previous = text;
                source = cause.source();
            }
        }
        line.push_str(LINE_SEPARATOR);

        let _guard = APPEND_LOCK.lock();
        // Diagnostics must never change the update result.
        let _ignored: std::io::Result<()> = (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .write_all(line.as_bytes())
        })();
    }
}

fn timestamp() -> String {
    chrono::Local::now().to_rfc3339()
}

/// `<data dir>/logs/arma3sync-updater-debug.log` of the Rust application's directories.
fn default_path() -> Option<PathBuf> {
    let env = PlatformEnv::current().ok()?;
    let dirs = AppDirs::resolve(&env).ok()?;
    Some(dirs.log_dir().join(LOG_FILE_NAME))
}

/// Lexically removes `.` and `..` components (Java `Path.normalize`).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
