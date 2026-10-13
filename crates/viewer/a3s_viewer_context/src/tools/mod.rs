//! Game tools: server keys, the radio plugin installer and the game log.
//!
//! The tool windows call the functions of this module with the context's [`Cx`]; the results
//! land in [`State`], which the windows read every frame.

mod keys;
mod log;
mod platform;
mod radio;

pub use self::keys::{
    KeyCopy, KeyScan, KeysState, ModKeys, copy_keys, find_keys, keys_target, set_keys_target,
};
pub use self::log::{
    LineKind, LogChoice, LogLine, LogState, MAX_LOG_LINES, follow_log, line_kind, rpt_files,
    set_rpt_folder, stop_log,
};
pub use self::platform::Platform;
pub use self::radio::{
    Installed, RadioInspection, RadioInstall, RadioPlan, RadioState, inspect_radio, install_radio,
    set_ts3_dir,
};

use a3s_tools::CancelToken;

use crate::error::ContextError;
use crate::tasks::{Cx, TaskCtx, TaskError};

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
pub struct State {
    /// How the tools reach the operating system.
    pub platform: Platform,

    /// Server keys of the enabled mods.
    pub keys: KeysState,

    /// The radio plugin installer.
    pub radio: RadioState,

    /// The followed game log.
    pub log: LogState,
}

pub(crate) fn handle(_cx: &Cx, command: Command) -> Result<(), ContextError> {
    match command {}
}

/// Runs `f` on the blocking pool with a [`CancelToken`] that fires when the task is cancelled.
/// A panic in `f` is resumed in the task, so the task reports it.
async fn blocking<T, F>(task: &TaskCtx, f: F) -> Result<T, TaskError>
where
    T: Send + 'static,
    F: FnOnce(&CancelToken) -> T + Send + 'static,
{
    let cancel = CancelToken::new();
    let watcher = {
        let cancel = cancel.clone();
        let token = task.cancel.clone();
        tokio::spawn(async move {
            token.cancelled().await;
            cancel.cancel();
        })
    };
    let joined = tokio::task::spawn_blocking(move || f(&cancel)).await;
    watcher.abort();
    match joined {
        Ok(value) => Ok(value),
        Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
        Err(_) => Err(TaskError::Cancelled),
    }
}
