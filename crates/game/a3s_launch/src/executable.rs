//! Checks on the configured game executable.

use std::path::{Path, PathBuf};

use a3s_foundation::paths::Os;
use a3s_types::constants::GameExecutable;
use a3s_types::local::Profile;

use crate::LaunchError;

/// How an executable is started, from its file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutableKind {
    /// `*.exe`.
    Exe,

    /// `*.bat`, through `cmd.exe`.
    Bat,

    /// `*.sh`, through `bash`.
    Sh,

    /// Anything else (a Linux binary such as `arma3server`).
    Other,
}

/// The kind of `file_name` by its extension, ignoring ASCII case.
///
/// Java tests `contains(".exe")`, `.bat`, `.sh` case-sensitively, which calls `ARMA3.EXE`
/// invalid and `x.exe.sh` an `.exe`; this tests the real extension.
// Mirrors dao/LauncherDAO.java:126, 142, 161, 205, 258, 291
pub fn executable_kind(file_name: &str) -> ExecutableKind {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".exe") {
        ExecutableKind::Exe
    } else if lower.ends_with(".bat") {
        ExecutableKind::Bat
    } else if lower.ends_with(".sh") {
        ExecutableKind::Sh
    } else {
        ExecutableKind::Other
    }
}

/// The file name of `path` as text (Java `File.getName`), `""` when there is none.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether `file_name` is the BattlEye launcher, ignoring ASCII case.
pub fn is_battleye(file_name: &str) -> bool {
    file_name.eq_ignore_ascii_case(GameExecutable::Battleye.file_name())
}

/// Whether `file_name` is a game executable that may be started elevated: the BattlEye
/// launcher, `arma3.exe` or `arma3_x64.exe`, ignoring ASCII case.
// Mirrors dao/LauncherDAO.java:416-420
pub fn is_elevatable(file_name: &str) -> bool {
    [
        GameExecutable::Game,
        GameExecutable::GameX64,
        GameExecutable::Battleye,
    ]
    .iter()
    .any(|exe| file_name.eq_ignore_ascii_case(exe.file_name()))
}

/// The executable the "Enable BattlEye" checkbox switches to.
///
/// `arma3battleye.exe` for `arma3.exe`/`arma3_x64.exe`, and back to `arma3_x64.exe` (Windows
/// with `ProgramFiles(x86)` set) or `arma3.exe`. The names compare case-sensitively, as in Java. `None` for any other
/// name or a path without a parent. The caller checks that the result exists.
// Mirrors ui/main/LauncherOptionsPanel.java:966-995
pub fn battleye_toggle_path(exe: &Path, os: Os, program_files_x86_set: bool) -> Option<PathBuf> {
    let parent = exe.parent()?;
    let name = exe.file_name()?.to_str()?;
    let target = if name == GameExecutable::Battleye.file_name() {
        if os == Os::Windows && program_files_x86_set {
            GameExecutable::GameX64
        } else {
            GameExecutable::Game
        }
    } else if name == GameExecutable::Game.file_name()
        || name == GameExecutable::GameX64.file_name()
    {
        GameExecutable::Battleye
    } else {
        return None;
    };
    Some(parent.join(target.file_name()))
}

/// The profile's Arma 3 executable, which must be set and exist (as a file or a directory).
// Mirrors service/LaunchService.java:43-56
pub fn check_arma3_executable_location(profile: Option<&Profile>) -> Result<PathBuf, LaunchError> {
    profile
        .and_then(|profile| profile.launcher_options.arma3_exe_path.as_deref())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .ok_or(LaunchError::ExecutableLocation)
}
