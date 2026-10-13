//! Game tools: keys, RPT log, radio plugins.

use crate::error::ContextError;
use crate::tasks::Cx;

/// Commands of this domain (it has none).
#[allow(clippy::empty_enums, reason = "the domain has no commands")]
#[derive(Clone, Copy, Debug)]
pub enum Command {}

/// Events of this domain (it has none).
#[allow(clippy::empty_enums, reason = "the domain has no events")]
#[derive(Clone, Debug)]
pub enum Event {}

/// State of this domain.
#[derive(Debug, Default)]
pub struct State {}

pub(crate) fn handle(_cx: &Cx, command: Command) -> Result<(), ContextError> {
    match command {}
}
