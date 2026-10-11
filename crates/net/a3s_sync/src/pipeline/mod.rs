//! The end-to-end client synchronization.
//!
//! A [`SyncSession`] checks a repository (metadata, plan, completion), lets the caller change
//! the selection, then runs downloads: archives left by a previous run are uncompressed, extra
//! local files deleted, files downloaded and uncompressed, and a report stored. A paused or
//! interrupted run resumes on the next [`SyncSession::download`]. [`sync_repository`] chains
//! everything for automatic updates and the command line. Client data is read and written only
//! through the caller's [`SyncStore`].

mod auto;
mod session;
mod store;
mod userconfig;

pub use auto::{
    CLI_RECONNECT_DELAY, ReconnectPolicy, SyncEnd, SyncJob, SyncRunReport, sync_repository,
};
pub use session::{CheckError, CheckRequest, RunEnd, RunReport, SyncSession};
pub use store::{MemoryStore, StoreChanges, StoreError, SyncStore};
pub use userconfig::{
    USERCONFIG_CONFIRM_MESSAGE, UserconfigAction, UserconfigCopy, UserconfigOutcome,
    UserconfigPolicy, copy_userconfig, plan_userconfig_copy,
};
