//! Foundation shared by every crate: the error taxonomy and chain formatting ([`error`]),
//! human-readable sizes, speeds and dates ([`format`](mod@format)), logging setup
//! ([`log`](mod@log)), the platform application directories ([`paths`]) and Windows
//! command-line and PowerShell quoting ([`command_line`]).

pub mod command_line;
pub mod error;
pub mod format;
pub mod log;
pub mod paths;
