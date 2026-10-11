//! The commands shared by the command line and the interactive console.

pub(crate) mod build;
pub(crate) mod check;
pub(crate) mod extract;
pub(crate) mod sync;
pub(crate) mod update;

use a3s_foundation::error::{
    ErrorKind, UserFacing, repository_managed_error, repository_unexpected_error,
};
use a3s_types::local::Repository;

use crate::app::{App, Failed};

/// A copy of the stored repository `name`, or the error printed.
pub(crate) fn repository(app: &App, name: &str) -> Result<Repository, Failed> {
    app.data.lock().repository(name).cloned().map_err(|err| {
        say_error!("{}", err.user_message());
        Failed
    })
}

/// Prints a service error and fails.
pub(crate) fn report(err: &impl UserFacing) -> Failed {
    say_error!("{}", err.user_message());
    Failed
}

/// Prints the console lines of a repository error: the message for an expected error, a
/// generic line (and the chain in the log) for an unexpected one.
// Mirrors utils/RepositoryConsoleErrorPrinter.java:5-22
pub(crate) fn report_repository_error(repository: &str, kind: ErrorKind, message: &str) {
    if kind.is_managed() {
        say_error!("{}", repository_managed_error(repository, message));
    } else {
        tracing::error!("Repository {repository}: {message}");
        say_error!("{}", repository_unexpected_error(repository));
    }
}

/// Prints a percentage line when the value grew past the last one printed.
#[derive(Debug, Default)]
pub(crate) struct PercentLine {
    last: Option<u8>,
}

impl PercentLine {
    /// Prints `{label}... complete: {percent} %` when `percent` is above the last value.
    pub(crate) fn update(&mut self, label: &str, percent: u8) {
        if self.last.is_none_or(|last| percent > last) {
            self.last = Some(percent);
            say!("{label}... complete: {percent} %");
        }
    }

    /// Starts over, for a new stage.
    pub(crate) fn reset(&mut self) {
        self.last = None;
    }
}
