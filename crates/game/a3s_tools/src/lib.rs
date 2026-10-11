//! Game-side tools of the Java client's "Tools" menu, without any UI: the bikey extractor
//! ([`bikey`]), the RPT log viewer's tailer ([`rpt`]) and the TFAR / ACRE 2 TS3 plugin
//! installers ([`radio`]).

pub mod bikey;
pub mod radio;
pub mod rpt;

mod cancel;
mod fs_util;

pub use cancel::CancelToken;

/// How a front-end presents a message (Java's `JOptionPane` message types).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// `INFORMATION_MESSAGE`.
    Info,

    /// `WARNING_MESSAGE`.
    Warning,

    /// `ERROR_MESSAGE`.
    Error,
}
