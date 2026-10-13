use a3s_foundation::error::{ErrorKind, UserFacing};

use crate::tasks::{TaskId, TaskKey};

/// Errors of the context itself: shutting down, a busy task, the runtime.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ContextError {
    /// Shutdown started: no command is accepted and no task starts.
    #[error("The application is shutting down.")]
    Closed,

    /// An exclusive task with the same key is running.
    #[error("{} is already running.", key.kind.label())]
    TaskBusy {
        /// The running task.
        id: TaskId,
        /// Its key.
        key: TaskKey,
    },

    /// The tokio runtime could not be built.
    #[error("Failed to start the background runtime: {0}")]
    Runtime(std::io::Error),
}

impl UserFacing for ContextError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Closed => ErrorKind::Cancelled,
            Self::TaskBusy { .. } => ErrorKind::InvalidInput,
            Self::Runtime(_) => ErrorKind::Internal,
        }
    }
}
