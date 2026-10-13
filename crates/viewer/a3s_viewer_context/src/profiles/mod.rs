//! Profiles and addons: the scan of the active profile's addon folders and event presets.

mod preset;
mod scan;

use std::sync::Arc;

pub use self::preset::use_event_preset;
pub(crate) use self::scan::scan_profile;
pub use self::scan::{AddonScan, ScanKey, ensure_scan};
use crate::error::ContextError;
use crate::tasks::Cx;

/// Profile and addon commands.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Command {
    /// Rescans the active profile's addon search folders and the Arma 3 folder as an
    /// [`crate::TaskKind::AddonScan`] task. While a scan runs the command is ignored; the UI
    /// asks again once it ended if the profile changed meanwhile ([`ensure_scan`]).
    ScanAddons,
}

/// Events of this domain (it has none).
#[allow(clippy::empty_enums, reason = "the domain has no events")]
#[derive(Clone, Debug)]
pub enum Event {}

/// Profiles and addons state.
#[derive(Debug, Default)]
pub struct State {
    /// The last finished addon scan.
    pub scan: Option<Arc<AddonScan>>,

    /// What the last requested scan was for; cleared when a scan ends.
    pub scan_requested: Option<ScanKey>,
}

pub(crate) fn handle(cx: &Cx, command: Command) -> Result<(), ContextError> {
    match command {
        Command::ScanAddons => scan::start(cx),
    }
}
