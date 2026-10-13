//! Game launch: updates first when asked, then the companion apps and the game.

mod launcher;
mod play;

use std::fmt;
use std::sync::Arc;

use a3s_launch::{LaunchEvent, LaunchHandle};

pub use self::launcher::{GameLauncher, LaunchEnvironment, SystemLauncher};
pub use self::play::{enabled_modset_repositories, launch_order};
use crate::error::ContextError;
use crate::tasks::Cx;

/// Game launch commands.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Command {
    /// Starts the active profile's game as a [`crate::TaskKind::Launch`] task.
    Play {
        /// First download the repositories of the enabled addon groups that have updates.
        update_first: bool,
    },
}

/// Game launch events.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// Selected addons that are not on this computer; the game starts without them.
    MissingAddons {
        /// Their names.
        names: Vec<String>,
    },

    /// What happened to the game or a companion app.
    Game(LaunchEvent),
}

/// Game launch state.
pub struct State {
    /// Starts processes; tests put a recording launcher here.
    pub launcher: Arc<dyn GameLauncher>,

    /// The last game launch; dropping it detaches the launch thread.
    pub game: Option<LaunchHandle>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            launcher: Arc::new(SystemLauncher),
            game: None,
        }
    }
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("game", &self.game)
            .finish_non_exhaustive()
    }
}

pub(crate) fn handle(cx: &Cx, command: Command) -> Result<(), ContextError> {
    match command {
        Command::Play { update_first } => play::start(cx, update_first),
    }
}
