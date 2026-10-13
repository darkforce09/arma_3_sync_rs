//! Launching Arma 3: the run parameters of a profile, the `Arma3.cfg` clean-up, the command for
//! each OS (BattlEye, UAC elevation, `.bat`/`.sh`, Steam/Proton), the launch thread with
//! auto-restart, and the external applications started with the game.
//!
//! This is the one place that spawns the game. Planning is pure and testable for every OS on any
//! host; [`launch_arma3`] and [`launch_external_applications`] start processes.

mod error;

pub mod cfg;
pub mod command;
pub mod elevation;
pub mod executable;
pub mod external;
pub mod params;
pub mod runner;
pub mod steam;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_addons::AddonRegistry;
use a3s_foundation::paths::Os;
use a3s_platform::process::ProcessProbe;
use a3s_types::local::{Configuration, Profile};

pub use command::{CommandSpec, LaunchMethod, LaunchPlan, LinuxLaunchMode};
pub use error::LaunchError;
pub use external::launch_external_applications;
pub use params::{ModPathStyle, RunParameterInput, determine_run_parameters, launch_addon_names};
pub use runner::{
    EventSink, LaunchEvent, LaunchHandle, LaunchTarget, RestartGuard, RunOptions, spawn_plan,
};
pub use steam::{SteamCommand, SteamWatch, find_steam_command};

/// Everything a game launch needs.
#[derive(Clone, Debug)]
pub struct Arma3LaunchRequest<'a> {
    pub os: Os,

    /// The active profile; nothing is launched without one.
    pub profile: Option<&'a Profile>,
    pub configuration: &'a Configuration,
    pub registry: &'a AddonRegistry,

    /// How the game starts on Linux.
    pub linux_mode: LinuxLaunchMode,

    /// The Steam client, for a Steam launch ([`find_steam_command`]).
    pub steam: Option<SteamCommand>,

    /// The Arma 3 user folder holding `Arma3.cfg`; `None` skips the clean-up.
    pub arma3_documents_dir: Option<PathBuf>,

    /// When auto-restart gives up on a game that keeps exiting.
    pub restart_guard: RestartGuard,

    /// How auto-restart follows a game started through Steam.
    pub steam_watch: SteamWatch,
}

/// A planned game launch: the command and how the launch thread follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedLaunch {
    /// The command that starts the game.
    pub plan: LaunchPlan,

    /// Auto-restart and the Steam watch.
    pub options: RunOptions,
}

/// Plans the game launch of the active profile without starting anything; `Ok(None)` without a
/// profile.
///
/// Computes the run parameters ([`determine_run_parameters`], mod paths in Wine form for a Steam
/// launch) and plans the command, with auto-restart as the profile says. A missing executable
/// setting, an executable that cannot run on this OS and a missing Steam client fail here.
// Mirrors service/LaunchService.java:129-155
pub fn plan_launch(request: &Arma3LaunchRequest<'_>) -> Result<Option<PlannedLaunch>, LaunchError> {
    let Some(profile) = request.profile else {
        return Ok(None);
    };
    let options = &profile.launcher_options;
    let exe = options
        .arma3_exe_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(Path::new)
        .ok_or(LaunchError::ExecutableLocation)?;

    let method = command::arma3_method(request.os, exe, options, request.linux_mode)?;
    let mod_path_style = if method == LaunchMethod::Steam {
        ModPathStyle::WineZDrive
    } else {
        ModPathStyle::native()
    };
    let params = determine_run_parameters(&RunParameterInput {
        profile: Some(profile),
        configuration: request.configuration,
        registry: request.registry,
        mod_path_style,
    });
    let plan = command::plan_arma3(request.os, exe, &params, method, request.steam.as_ref())?;
    Ok(Some(PlannedLaunch {
        plan,
        options: RunOptions {
            auto_restart: options.auto_restart,
            guard: request.restart_guard,
            steam_watch: request.steam_watch,
        },
    }))
}

/// Starts the game of the active profile and returns at once; `Ok(None)` without a profile.
///
/// Plans the launch ([`plan_launch`]), strips the launcher's mod list from `Arma3.cfg` (a failure
/// is logged and the launch goes on) and runs the command on the `a3s-launch-arma3` thread
/// ([`spawn_plan`]). Planning errors come back before any thread starts.
pub fn launch_arma3(
    request: &Arma3LaunchRequest<'_>,
    probe: Arc<dyn ProcessProbe>,
    events: EventSink,
) -> Result<Option<LaunchHandle>, LaunchError> {
    let Some(planned) = plan_launch(request)? else {
        return Ok(None);
    };
    if let Some(dir) = &request.arma3_documents_dir
        && let Err(err) = cfg::erase_arma3_cfg_mod_launcher_list(dir)
    {
        tracing::warn!(
            "cannot remove the mod list from {}: {err}",
            dir.join(cfg::ARMA3_CFG).display()
        );
    }
    spawn_plan(planned.plan, planned.options, probe, events).map(Some)
}
