//! The seam that starts processes: the companion apps and the game.

use std::path::PathBuf;

use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_launch::{
    EventSink, LaunchError, LaunchHandle, LaunchPlan, RunOptions, SteamCommand, find_steam_command,
    launch_external_applications, spawn_plan,
};
use a3s_platform::folders::{SystemFolders, arma3_documents_dir};
use a3s_platform::process::SystemProcessProbe;
use a3s_platform::registry::SystemRegistry;
use a3s_types::local::ExternalApplication;

/// Where the game runs on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchEnvironment {
    /// The operating system.
    pub os: Os,

    /// The Steam client, if found.
    pub steam: Option<SteamCommand>,

    /// The Arma 3 user folder holding `Arma3.cfg`, if known.
    pub arma3_documents_dir: Option<PathBuf>,
}

/// Starts the processes of a game launch.
pub trait GameLauncher: Send + Sync + 'static {
    /// This computer's launch environment. May block (registry and folder lookups).
    fn environment(&self) -> LaunchEnvironment;

    /// Starts the enabled companion apps that are not running yet.
    fn start_companions(&self, os: Os, apps: &[ExternalApplication], events: &EventSink);

    /// Starts the game; `None` when nothing was spawned to follow.
    fn start_game(
        &self,
        plan: LaunchPlan,
        options: RunOptions,
        events: EventSink,
    ) -> Result<Option<LaunchHandle>, LaunchError>;
}

/// The [`GameLauncher`] that starts real processes.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLauncher;

impl GameLauncher for SystemLauncher {
    fn environment(&self) -> LaunchEnvironment {
        let os = Os::current();
        let env = PlatformEnv::current().ok();
        LaunchEnvironment {
            os,
            steam: env.as_ref().and_then(find_steam_command),
            arma3_documents_dir: env
                .as_ref()
                .and_then(|env| arma3_documents_dir(env, &SystemFolders, &SystemRegistry)),
        }
    }

    fn start_companions(&self, os: Os, apps: &[ExternalApplication], events: &EventSink) {
        // The threads wait for the apps and end on their own.
        drop(launch_external_applications(
            os,
            apps,
            &SystemProcessProbe,
            events,
        ));
    }

    fn start_game(
        &self,
        plan: LaunchPlan,
        options: RunOptions,
        events: EventSink,
    ) -> Result<Option<LaunchHandle>, LaunchError> {
        spawn_plan(
            plan,
            options,
            std::sync::Arc::new(SystemProcessProbe),
            events,
        )
        .map(Some)
    }
}
