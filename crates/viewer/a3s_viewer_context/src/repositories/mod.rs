//! Repository checks and the per-repository runtime state.

mod check;

use std::collections::BTreeMap;

use a3s_sync::status::NotificationTracker;

use crate::checker::{CheckTrigger, RepositoryCheckResult};
use crate::error::ContextError;
use crate::state::RepositoryRuntime;
use crate::tasks::Cx;

/// The text of the "updates available" toast and tray message.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:163
pub const UPDATES_AVAILABLE_MESSAGE: &str = "Repositories updates!";

/// Repository commands.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Command {
    /// Checks the named repositories (the UI's "check" shortcut). Unknown names are skipped;
    /// no names completes at once with no results. Fails with
    /// [`ContextError::TaskBusy`] while a check asked for by the user runs.
    Check {
        /// The repositories to check.
        names: Vec<String>,
    },

    /// Cancels the running check asked for by the user, every in-flight connection included.
    CancelCheck,
}

/// Repository events.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// A check run ended; its results were applied to the state.
    CheckCompleted {
        /// What started the run.
        trigger: CheckTrigger,
        /// One result per checked repository, in name order.
        results: Vec<RepositoryCheckResult>,
    },

    /// A periodic check found updates to notify the user about (once per server revision).
    /// The UI shows a toast ([`UPDATES_AVAILABLE_MESSAGE`]; a tray message when the window is
    /// hidden) and badges the repositories, whose runtime status is `Updated`.
    UpdatesAvailable {
        /// The updated repositories, in name order.
        repositories: Vec<String>,
    },
}

/// The runtime state of the repositories.
#[derive(Debug, Default)]
pub struct State {
    /// Runtime state by repository name; an entry is created on its first write.
    pub runtime: BTreeMap<String, RepositoryRuntime>,

    /// The server revisions already notified, kept for the whole session.
    pub tracker: NotificationTracker,
}

impl State {
    /// The runtime state of `name`, or the defaults if there is none.
    pub fn runtime(&self, name: &str) -> RepositoryRuntime {
        self.runtime.get(name).cloned().unwrap_or_default()
    }

    /// The runtime state of `name`, created with the defaults if there is none.
    pub fn runtime_mut(&mut self, name: &str) -> &mut RepositoryRuntime {
        self.runtime.entry(name.to_owned()).or_default()
    }
}

pub(crate) fn handle(cx: &Cx, command: Command) -> Result<(), ContextError> {
    match command {
        Command::Check { names } => check::start(cx, names),
        Command::CancelCheck => {
            check::cancel(cx);
            Ok(())
        }
    }
}
