//! zsync implementation compatible with the jazsync files produced by Arma3Sync.
//!
//! Arma3Sync (HTTP repositories only) publishes a `<file>.zsync` next to every file. A client
//! holding an older local copy uses it to find which 8 KiB blocks it already has, downloads only
//! the missing byte ranges and reassembles the new file.
//!
//! * [`ControlFile::generate`] writes `.zsync` files byte-identical to `jazsyncmake.MetaFileMaker`.
//! * [`ControlFile::parse`] reads them.
//! * [`match_blocks`] scans a local file for reusable blocks.
//! * [`Plan`] lists the byte ranges to download; [`assemble`] rebuilds and verifies the file.

mod assemble;
mod control;
mod error;
mod matcher;
mod rsum;

pub use assemble::{Plan, assemble};
pub use control::{BLOCK_SIZE, ControlFile, HashLengths, Header, format_mtime};
pub use error::{Error, Result};
pub use matcher::{BlockMatches, match_blocks};
pub use rsum::Rsum;
