//! Reading and writing `.a3s` repository metadata files and local client data.
//!
//! Every file in a repository's `.a3s/` folder is `GZIP(ObjectOutputStream(object))`
//! (`dao/A3SFilesAccessor.java`). This crate wraps [`a3s_types`]' Java codecs with the gzip
//! layer, size limits and atomic writes.

mod a3s_file;
mod error;
pub mod layout;

pub use a3s_file::{A3sFile, decode_a3s, encode_a3s, read_a3s, write_a3s};
pub use error::{Error, Result};
