//! Platform-specific application directories (XDG, APPDATA, portable mode).
//!
//! The Rust client keeps its own configuration, data and cache directories under
//! [`APP_DIR_NAME`] (`arma3sync-rs`), a name that never collides with the Java client's
//! `Arma3Sync` folders, even on case-insensitive file systems. The [`legacy`] module discovers,
//! read-only, where a Java installation keeps its data so that it can be imported.
//!
//! Resolution is a pure function of a [`PlatformEnv`], so every platform's rules are testable on
//! any host.

mod dirs;
mod env;
mod error;
pub mod legacy;

pub use dirs::AppDirs;
pub use env::{Os, PlatformEnv};
pub use error::Error;

/// Name of the Rust client's application directories.
pub const APP_DIR_NAME: &str = "arma3sync-rs";

/// Name of the Java client's application directories.
// Mirrors dao/ApplicationPaths.java:17
pub const JAVA_APP_NAME: &str = "Arma3Sync";

/// Marker file next to the executable that enables portable mode.
pub const PORTABLE_MARKER: &str = "arma3sync-rs.portable";

/// Folder next to the executable that holds all data in portable mode.
pub const PORTABLE_DATA_DIR: &str = "arma3sync-rs-data";

/// File name of the exported error report.
// Mirrors dao/DataAccessConstants.java:49
pub const EXPORT_LOG_FILE_NAME: &str = "ArmA3Sync-log.txt";

/// Environment variable that overrides the configuration directory (final directory).
pub const CONFIG_DIR_VAR: &str = "A3S_CONFIG_DIR";

/// Environment variable that overrides the data directory (final directory).
pub const DATA_DIR_VAR: &str = "A3S_DATA_DIR";

/// Environment variable that overrides the cache directory (final directory).
pub const CACHE_DIR_VAR: &str = "A3S_CACHE_DIR";

/// Environment variable that enables portable mode when it equals `true` (ignoring case).
pub const PORTABLE_VAR: &str = "A3S_PORTABLE";
