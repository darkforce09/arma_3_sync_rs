//! Game-side tools of the Java client, without any UI: the "Tools" menu's bikey extractor
//! ([`bikey`]), the RPT log viewer's tailer ([`rpt`]) and the TFAR / ACRE 2 TS3 plugin
//! installers ([`radio`]), and the addon groups' Arma 3 Launcher preset import and export
//! ([`preset`]).

pub mod bikey;
pub mod preset;
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
