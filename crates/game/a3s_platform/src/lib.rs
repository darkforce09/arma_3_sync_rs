//! Operating-system integration of the Arma3Sync client: Windows registry access, Arma 3, Steam
//! and `TeamSpeak` 3 detection (including Steam libraries and Proton prefixes on Linux), special
//! folders, "is this game running" probes, start-with-the-OS entries, desktop shortcuts and the
//! single-instance lock.
//!
//! Everything that touches the OS goes through an injectable seam ([`registry::RegistryRead`],
//! [`folders::SpecialFolders`], [`process::ProcessProbe`], [`a3s_foundation::paths::PlatformEnv`]),
//! so every OS's behaviour is testable on any host.

mod error;
mod fs_write;
mod vdf;

pub mod autostart;
pub mod detect;
pub mod folders;
pub mod process;
pub mod registry;
pub mod reveal;
pub mod shortcut;
pub mod single_instance;
pub mod steam;

pub use error::Error;

/// Result type of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
