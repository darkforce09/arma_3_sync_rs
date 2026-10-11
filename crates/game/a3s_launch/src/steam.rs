//! Launching Arma 3 through the Steam client, which runs the Windows game under Proton on Linux.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use a3s_foundation::paths::PlatformEnv;

pub use a3s_platform::steam::ARMA3_APP_ID;

/// The Flatpak application id of the Steam client.
pub const FLATPAK_STEAM_APP: &str = "com.valvesoftware.Steam";

/// How to run the Steam client: the program and the arguments that come before Steam's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamCommand {
    /// `[<steam>]`, or `[<flatpak>, "run", "com.valvesoftware.Steam"]`.
    pub prefix: Vec<OsString>,
}

/// The Steam client of this user: `steam` on `PATH`, else the Flatpak Steam when `flatpak` is
/// on `PATH` and `~/.var/app/com.valvesoftware.Steam` exists. `None` when neither is found.
pub fn find_steam_command(env: &PlatformEnv) -> Option<SteamCommand> {
    let path = env.var_os("PATH");
    if let Some(steam) = find_on_path(path, "steam") {
        return Some(SteamCommand {
            prefix: vec![steam.into_os_string()],
        });
    }
    let flatpak_data = env.home()?.join(".var/app").join(FLATPAK_STEAM_APP);
    if !flatpak_data.is_dir() {
        return None;
    }
    let flatpak = find_on_path(path, "flatpak")?;
    Some(SteamCommand {
        prefix: vec![
            flatpak.into_os_string(),
            "run".into(),
            FLATPAK_STEAM_APP.into(),
        ],
    })
}

/// The first executable file `name` in the directories of `path` (a `PATH` value).
fn find_on_path(path: Option<&OsStr>, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// How auto-restart follows a game started through Steam, whose own process is not the game:
/// it polls the process list for the game, waiting `appear_timeout` for it to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SteamWatch {
    /// How long the game may take to appear after Steam was asked to start it.
    pub appear_timeout: Duration,

    /// Time between two process-list polls.
    pub poll: Duration,
}

impl Default for SteamWatch {
    fn default() -> Self {
        Self {
            appear_timeout: Duration::from_secs(120),
            poll: Duration::from_secs(2),
        }
    }
}
