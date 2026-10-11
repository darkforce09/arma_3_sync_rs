//! Repository administration: build, check and upload Arma3Sync repositories.
//!
//! * [`build`]: build a repository from a local folder ([`build_repository`]), writing `.a3s/`
//!   metadata, `.zsync` and `.zip` files exactly like the Java implementation.

pub mod build;

pub use build::{BuildError, BuildEvent, BuildOptions, BuildOutcome, build_repository};
