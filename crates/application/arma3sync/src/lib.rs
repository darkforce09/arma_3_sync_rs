//! Arma3Sync: Arma 3 mod repository synchronization and launcher.
//!
//! The library half of the `arma3sync` binary: the shared entry point and the build information
//! baked into it.

pub mod build_info;

/// Entry point of the `arma3sync` binary.
pub fn run() -> std::process::ExitCode {
    std::process::ExitCode::SUCCESS
}
