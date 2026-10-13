use a3s_foundation::error::UserFacing as _;
use a3s_sync::EventError;
use tokio::sync::mpsc;

use crate::event::AppEvent;
use crate::tasks::Cx;
use crate::{admin, launch, profiles, repositories, sync, tools};

/// A request from the UI to the background, handled in send order.
#[derive(Clone, Debug)]
pub enum Command {
    /// Repository checks.
    Repositories(repositories::Command),

    /// Synchronization.
    Sync(sync::Command),

    /// Repository administration.
    Admin(admin::Command),

    /// Profiles and addons.
    Profiles(profiles::Command),

    /// Game launch.
    Launch(launch::Command),

    /// Game tools.
    Tools(tools::Command),
}

/// Handles commands in order until the channel closes. Handlers only do quick in-memory work
/// or start tasks; a handler error is emitted as [`AppEvent::Error`].
pub(crate) async fn dispatch(cx: Cx, mut commands: mpsc::UnboundedReceiver<Command>) {
    while let Some(command) = commands.recv().await {
        let context = format!("{command:?}");
        let handled = match command {
            Command::Repositories(command) => repositories::handle(&cx, command),
            Command::Sync(command) => {
                sync::handle(&cx, command);
                Ok(())
            }
            Command::Admin(command) => admin::handle(&cx, command),
            Command::Profiles(command) => profiles::handle(&cx, command),
            Command::Launch(command) => launch::handle(&cx, command),
            Command::Tools(command) => tools::handle(&cx, command),
        };
        if let Err(err) = handled {
            tracing::warn!("{context}: {err}");
            cx.emit(AppEvent::Error {
                context,
                error: EventError::new(err.kind(), err.user_message()),
            });
        }
    }
}
