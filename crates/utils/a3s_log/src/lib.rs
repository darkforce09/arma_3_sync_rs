//! Logging setup built on `tracing`, shared by all binaries.
//!
//! Port of the Java client's `utils/DebugLogger.java`:
//! * [`init`] installs a global subscriber with a stderr layer (`A3S_LOG`, default `warn`) and,
//!   opt-in like Java's `-debug`, a size-rotated debug file `arma3sync-rs.log`.
//! * Both layers use the Java-like line format of [`JavaLikeFormat`].
//! * [`redact`] and [`describe_endpoint`] keep credentials out of log lines.

mod format;
mod redact;
mod rotating;

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::Metadata;
use tracing_subscriber::filter::{EnvFilter, FilterFn, LevelFilter};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{Layer as _, fmt};

pub use format::JavaLikeFormat;
pub use redact::{describe_endpoint, redact};
pub use rotating::{LOG_FILE_COUNT, MAX_LOG_BYTES, RotatingFile, RotatingWriter};

/// File name of the debug log inside the log directory.
pub const LOG_FILE_NAME: &str = "arma3sync-rs.log";

/// Environment variable with the stderr filter (`EnvFilter` syntax).
pub const LOG_FILTER_VAR: &str = "A3S_LOG";

/// Errors of [`init`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A global subscriber is already installed.
    #[error("logging is already initialized")]
    AlreadyInitialized,

    /// The `A3S_LOG` filter is invalid. Reported as a warning and replaced by `warn`; never
    /// returned by [`init`].
    #[error("invalid {LOG_FILTER_VAR} filter: {0}")]
    Filter(String),
}

/// Logging configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogConfig {
    /// Enables the debug file (CLI `--debug`/`-debug`, env `A3S_DEBUG=true`).
    pub debug: bool,

    /// Directory of the debug file (`AppDirs::log_dir`). Required when `debug` is set.
    pub log_dir: Option<PathBuf>,

    /// Enables the stderr layer.
    pub stderr: bool,
}

/// Keeps the debug file writer alive; returned by [`init`].
#[derive(Debug)]
pub struct LogHandle {
    file: Option<RotatingFile>,
    log_file: Option<PathBuf>,
}

impl LogHandle {
    /// Absolute path of the debug file, `None` when file logging is disabled or failed.
    pub fn log_file(&self) -> Option<&Path> {
        self.log_file.as_deref()
    }

    /// The debug file writer, if any.
    pub fn writer(&self) -> Option<&RotatingFile> {
        self.file.as_ref()
    }
}

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Installs the global `tracing` subscriber. A second call returns
/// [`Error::AlreadyInitialized`].
///
/// If the debug file cannot be opened, logging still starts without it, and one line
/// `Arma3Sync diagnostic logging could not be initialized: {error}` is printed to stderr.
pub fn init(config: &LogConfig) -> Result<LogHandle, Error> {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return Err(Error::AlreadyInitialized);
    }

    let (stderr_filter, filter_error) = stderr_filter();
    let stderr_layer = config.stderr.then(|| {
        fmt::layer()
            .event_format(JavaLikeFormat)
            .with_ansi(false)
            .with_writer(io::stderr)
            .with_filter(stderr_filter)
    });

    let file = if config.debug {
        match open_debug_file(config.log_dir.as_deref()) {
            Ok(file) => Some(file),
            Err(err) => {
                // Mirrors utils/DebugLogger.java:57-64
                writeln!(
                    io::stderr(),
                    "Arma3Sync diagnostic logging could not be initialized: {err}"
                )
                .ok();
                None
            }
        }
    } else {
        None
    };
    let file_layer = file.clone().map(|file| {
        fmt::layer()
            .event_format(JavaLikeFormat)
            .with_ansi(false)
            .with_writer(file)
            .with_filter(FilterFn::new(file_filter))
    });

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init()
        .map_err(|_already_set| Error::AlreadyInitialized)?;

    if let Some(err) = filter_error {
        tracing::warn!("{err}");
    }
    let log_file = file.as_ref().map(|file| {
        let path = std::path::absolute(file.path()).unwrap_or_else(|_| file.path().to_path_buf());
        // Mirrors utils/DebugLogger.java:55
        tracing::info!("Debug logging enabled. Log file: {}", path.display());
        path
    });
    Ok(LogHandle { file, log_file })
}

/// The stderr filter from `A3S_LOG`, else `warn`; an invalid value is returned as an error to
/// report once the subscriber is installed.
fn stderr_filter() -> (EnvFilter, Option<Error>) {
    let default = || EnvFilter::default().add_directive(LevelFilter::WARN.into());
    match std::env::var(LOG_FILTER_VAR) {
        Ok(value) if !value.trim().is_empty() => match EnvFilter::try_new(&value) {
            Ok(filter) => (filter, None),
            Err(err) => (default(), Some(Error::Filter(err.to_string()))),
        },
        _ => (default(), None),
    }
}

/// Debug file filter: `debug` for the application's own targets, `info` for everything else.
fn file_filter(metadata: &Metadata<'_>) -> bool {
    let target = metadata.target();
    let max = if target.starts_with("a3s_") || target.starts_with("arma3sync") {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };
    *metadata.level() <= max
}

fn open_debug_file(log_dir: Option<&Path>) -> io::Result<RotatingFile> {
    let log_dir = log_dir
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no log directory configured"))?;
    RotatingFile::open(log_dir.join(LOG_FILE_NAME), MAX_LOG_BYTES, LOG_FILE_COUNT)
}

#[cfg(test)]
mod tests {
    use tracing::Level;
    use tracing::metadata::Kind;

    use super::*;

    macro_rules! metadata {
        ($target:expr, $level:expr) => {{
            struct Callsite;
            impl tracing::callsite::Callsite for Callsite {
                fn set_interest(&self, _: tracing::subscriber::Interest) {}
                fn metadata(&self) -> &Metadata<'_> {
                    unreachable!()
                }
            }
            static CALLSITE: Callsite = Callsite;
            Metadata::new(
                "test",
                $target,
                $level,
                None,
                None,
                None,
                tracing::field::FieldSet::new(&[], tracing::callsite::Identifier(&CALLSITE)),
                Kind::EVENT,
            )
        }};
    }

    #[test]
    fn file_filter_levels() {
        assert!(file_filter(&metadata!("a3s_sync::x", Level::DEBUG)));
        assert!(file_filter(&metadata!("arma3sync_cli", Level::DEBUG)));
        assert!(!file_filter(&metadata!("a3s_sync", Level::TRACE)));
        assert!(file_filter(&metadata!("reqwest", Level::INFO)));
        assert!(!file_filter(&metadata!("reqwest", Level::DEBUG)));
    }
}
