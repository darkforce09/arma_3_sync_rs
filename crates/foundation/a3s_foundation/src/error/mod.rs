//! Shared error taxonomy, error-chain formatting and the user-facing message texts of the Java
//! client.
//!
//! Every library crate keeps its own `thiserror` enum and implements [`UserFacing`] on it, so the
//! front-ends can decide between a "managed" error (shown as a message) and an unexpected one
//! (shown with a report, see [`ErrorReport`]) without knowing every concrete type.
//!
//! The verbatim Java texts (including their typos) live in [`messages`].

mod chain;
mod kind;
pub mod messages;
mod report;

pub use chain::{DisplayChain, format_chain, format_chain_lines};
pub use kind::{ErrorKind, classify_timeout, kind_of};
pub use report::{ErrorReport, repository_managed_error, repository_unexpected_error};

/// An error that can be shown to the user.
///
/// Implemented by the error enums of the library crates.
pub trait UserFacing: std::error::Error {
    /// Classification of the error.
    fn kind(&self) -> ErrorKind;

    /// The text shown to the user. Defaults to the error chain joined with `": "`.
    fn user_message(&self) -> String {
        chain::join_chain(self.to_string(), self.source(), ": ")
    }

    /// Whether the error is "managed" (expected, shown as a plain message) rather than an
    /// unexpected internal failure. See [`ErrorKind::is_managed`].
    fn is_managed(&self) -> bool {
        self.kind().is_managed()
    }
}
