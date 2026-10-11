//! Reading and writing `.a3s` repository metadata files and local client data.
//!
//! Every file in a repository's `.a3s/` folder is `GZIP(ObjectOutputStream(object))`
//! (`dao/A3SFilesAccessor.java`). This crate wraps [`a3s_types`]' Java codecs with the gzip
//! layer, size limits and atomic writes.
//!
//! [`local`] is the Rust client's own TOML store (configuration, preferences, profiles,
//! repositories, SHA-1 caches, Workshop metadata). [`legacy`] reads the Java client's files
//! (including the AES-sealed `*.a3s.repository`) and imports them once into that store.
//! [`services`] keeps that state in memory for the CLI and the viewer and persists every change.

mod a3s_file;
pub mod autoconfig_export;
mod error;
pub mod hash;
pub mod layout;
pub mod legacy;
pub mod local;
pub mod services;
mod temp_file;

pub use a3s_file::{A3sFile, decode_a3s, encode_a3s, read_a3s, write_a3s, write_a3s_stream};
pub use error::{Error, Result};
pub use temp_file::{TempFileMode, temp_file_next_to};
