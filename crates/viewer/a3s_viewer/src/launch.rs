//! The Play button's entry point.

use a3s_viewer_context::ViewerContext;

/// What the Play button asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlayRequest {
    /// The profile to launch.
    pub(crate) profile: String,

    /// The profile's repositories have updates to download before the game starts.
    pub(crate) update_first: bool,
}

/// The hook behind the Play button, called with the active profile and whether its
/// repositories have updates. It only logs the request: it neither downloads nor starts the
/// game.
pub(crate) fn request(_viewer: &ViewerContext, play: &PlayRequest) {
    tracing::info!(
        "Play requested for profile {} (update first: {})",
        play.profile,
        play.update_first
    );
}
