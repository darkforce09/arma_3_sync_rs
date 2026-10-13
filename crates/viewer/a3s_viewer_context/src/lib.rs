//! Application state, command bus and background tasks of the Arma3Sync viewer.
//!
//! [`ViewerContext`] is the one object the UI holds (Java's `ui/Facade`): the shared
//! [`AppState`], a tokio runtime, the command bus (UI to background, [`Command`]), the event
//! channel (background to UI, [`AppEvent`]) with a [`RepaintHook`], the task registry, and the
//! periodic repository check.
//!
//! Each domain has its own module with a `Command`, an `Event` and a `State`: [`repositories`],
//! [`sync`], [`admin`], [`profiles`], [`launch`] and [`tools`].

pub mod admin;
pub mod launch;
pub mod profiles;
pub mod repositories;
pub mod sync;
pub mod tools;

mod background;
mod checker;
mod clock;
mod command;
mod context;
mod error;
mod event;
mod schedule;
mod state;
mod store;
mod tasks;

pub use checker::{
    CheckOutcome, CheckTrigger, NetworkChecker, RemoteCheck, RemoteChecker, RepositoryCheckResult,
};
pub use clock::{Clock, ManualClock, SystemClock};
pub use command::Command;
pub use context::{ContextOptions, ViewerContext};
pub use error::ContextError;
pub use event::{AppEvent, Change, RepaintHook, TaskEvent};
pub use schedule::CheckSchedule;
pub use state::{AppFlags, AppState, RepositoryRuntime};
pub use store::StateStore;
pub use sync::{NetworkSynchronizer, Synchronizer};
pub use tasks::{
    Cx, TaskCtx, TaskError, TaskId, TaskInfo, TaskKey, TaskKind, TaskOutcome, TaskSnapshot,
};
