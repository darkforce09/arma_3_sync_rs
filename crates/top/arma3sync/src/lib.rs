//! Arma3Sync: Arma 3 mod repository synchronization and launcher.
//!
//! This is the facade crate. It re-exports the public parts of the other crates
//! and hosts the shared entry point used by the `arma3sync` binary.

/// Entry point shared by all binaries.
pub fn run() -> std::process::ExitCode {
    std::process::ExitCode::SUCCESS
}
