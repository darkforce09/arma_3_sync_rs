//! Client synchronization engine: check, plan and download repository content.
//!
//! * [`model`]: the client-side plan tree and the download/delete work items.
//! * [`events`]: progress and outcome events ([`SyncEvent`]) and their [`EventSender`].
//! * [`options`]: [`SyncOptions`] derived from a client repository.
//! * [`error`]: [`RemoteError`], with the Java user-facing messages.
//! * [`remote`]: repository metadata access ([`RemoteConnection`], [`RemoteMetadata`]).
//! * [`status`]: repository status, revision handling and update notifications.
//! * [`event_filter`]: restricts a plan to the addons of one event.
//! * [`plan`]: compares the remote tree with the local disk ([`plan::plan_repository`]) and
//!   summarizes a selection ([`PlanSummary`]).
//! * [`completion`]: zsync completion of outdated local files.

pub mod completion;
pub mod error;
pub mod event_filter;
pub mod events;
pub mod model;
pub mod options;
pub mod plan;
pub mod remote;
pub mod status;

pub use completion::{CompletionError, CompletionStats};
pub use error::RemoteError;
pub use events::{
    EventError, EventSender, Outcome, PercentCounter, Phase, RateLimiter, SyncEvent,
    TransferProgress, WAITING_FOR_SERVER_THRESHOLD,
};
pub use model::{
    DeleteTask, DownloadStatus, DownloadTask, DownloadTaskKind, PlanDirectory, PlanLeaf, PlanNode,
    PlanTree, ROOT_NAME, file_name, parent_relative_path,
};
pub use options::{MAX_DOWNLOAD_ERRORS, SyncOptions, SyncOptionsError};
pub use plan::{AddonLocator, NoAddons, PlanError, PlanSummary, ReportTotals};
pub use remote::{PartialTransferSupport, RemoteConnection, RemoteFile, RemoteMetadata};
