//! Foundation shared by every crate: the error taxonomy and chain formatting ([`error`]),
//! human-readable sizes, speeds and dates ([`format`](mod@format)), logging setup
//! ([`log`](mod@log)) and the platform application directories ([`paths`]).

pub mod error;
pub mod format;
pub mod log;
pub mod paths;
