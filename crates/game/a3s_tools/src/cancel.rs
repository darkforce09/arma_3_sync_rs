use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative cancellation flag shared between a worker and the thread that cancels it.
///
/// Each operation gets its own token, so a cancelled run never affects a later one (Java keeps
/// one process-wide flag that is never reset).
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps an existing flag, for callers that already share an `Arc<AtomicBool>`.
    pub fn from_arc(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }

    /// The underlying flag.
    pub fn as_arc(&self) -> &Arc<AtomicBool> {
        &self.0
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
