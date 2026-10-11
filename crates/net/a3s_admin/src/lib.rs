//! Repository administration: build, check and upload Arma3Sync repositories, and edit their
//! events.
//!
//! * [`build`]: build a repository from a local folder ([`build_repository`]), writing `.a3s/`
//!   metadata, `.zsync` and `.zip` files exactly like the Java implementation.
//! * [`check`]: check that the server holds every file of the remote sync tree
//!   ([`check_repository_content`]).
//! * [`upload`]: upload a built repository ([`upload_repository`]): changed files first, then
//!   the metadata, then deletions.
//! * [`events`]: edit the events (modsets) of a repository ([`EventsDoc`]), save them locally
//!   or upload them.

pub mod build;
pub mod check;
pub mod events;
pub mod upload;

pub use build::{BuildError, BuildEvent, BuildOptions, BuildOutcome, build_repository};
pub use check::{CheckError, ContentCheckReport, check_repository_content};
pub use events::{
    AdminMetadata, EventSelection, EventsDoc, EventsError, fetch_admin_metadata, upload_events,
};
pub use upload::{UploadError, UploadReport, UploadResume, upload_repository};
