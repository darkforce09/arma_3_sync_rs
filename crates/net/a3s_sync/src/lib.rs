//! Client synchronization engine: check, plan and download repository content.
//!
//! * [`model`]: the client-side plan tree and the download/delete work items.
//! * [`events`]: progress and outcome events ([`SyncEvent`]) and their [`EventSender`].
//! * [`options`]: [`SyncOptions`] derived from a client repository.
//! * [`error`]: [`RemoteError`], with the Java user-facing messages.
//! * [`remote`]: repository metadata access ([`RemoteConnection`], [`RemoteMetadata`]).
//! * [`status`]: repository status, revision handling and update notifications.

pub mod error;
pub mod events;
pub mod model;
pub mod options;
pub mod remote;
pub mod status;

pub use error::RemoteError;
pub use events::{EventError, EventSender, Outcome, Phase, SyncEvent, TransferProgress};
pub use model::{
    DeleteTask, DownloadTask, DownloadTaskKind, PlanDirectory, PlanLeaf, PlanNode, PlanTree,
};
pub use options::{SyncOptions, SyncOptionsError};
pub use remote::{PartialTransferSupport, RemoteConnection, RemoteFile, RemoteMetadata};
