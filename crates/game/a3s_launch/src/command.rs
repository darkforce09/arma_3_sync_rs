//! Turning (OS, executable, parameters, options) into the exact command to spawn. Pure, so every
//! OS's plan is testable on any host.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use a3s_foundation::paths::Os;
use a3s_types::java_string::java_trim;
use a3s_types::local::{GameStart, LauncherOptions};

use crate::LaunchError;
use crate::elevation::elevated_command;
use crate::executable::{ExecutableKind, executable_kind, file_name, is_battleye, is_elevatable};
use crate::steam::{ARMA3_APP_ID, SteamCommand};

/// A process to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,

    /// Working directory; `None` inherits Arma3Sync's.
    pub current_dir: Option<PathBuf>,

    /// Windows: start without a console window (`CREATE_NO_WINDOW`).
    pub hide_window: bool,

    /// Send the child's output to the null device instead of Arma3Sync's own output.
    pub discard_output: bool,
}

impl CommandSpec {
    /// A command whose output goes to Arma3Sync's own output.
    fn new(
        program: impl Into<OsString>,
        args: Vec<OsString>,
        current_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            args,
            current_dir,
            hide_window: false,
            discard_output: false,
        }
    }

    /// The program as text, for messages.
    pub fn program_text(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }

    /// The program followed by its arguments, as text.
    pub fn elements(&self) -> Vec<String> {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|element| element.to_string_lossy().into_owned())
            .collect()
    }

    /// A [`Command`] for this spec. Standard input is the null device; the child never gets a
    /// pipe to Arma3Sync, so it survives Arma3Sync exiting.
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).stdin(Stdio::null());
        if self.discard_output {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        } else {
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        if self.hide_window {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }
}

/// How the game is started on Linux.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinuxLaunchMode {
    /// Through Steam for a Windows `.exe` (the game then runs under Proton), directly otherwise.
    #[default]
    Auto,

    /// Run the executable itself, as the Java client does.
    Direct,

    /// Always through Steam.
    Steam,
}

impl From<GameStart> for LinuxLaunchMode {
    /// The profile's stored choice.
    fn from(start: GameStart) -> Self {
        match start {
            GameStart::Automatic => Self::Auto,
            GameStart::Steam => Self::Steam,
            GameStart::Executable => Self::Direct,
        }
    }
}

/// How a launch starts the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMethod {
    /// The executable (or its interpreter) is the spawned process.
    Direct,

    /// Windows: through PowerShell's UAC elevation, which waits for the game.
    Elevated,

    /// Through the Steam client, which starts the game and may outlive it.
    Steam,
}

/// A planned game launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub method: LaunchMethod,
    pub command: CommandSpec,

    /// The command line as logged. For an elevated launch this is the plain game command, not
    /// the PowerShell one, as in Java.
    pub shown_command: Vec<String>,
}

/// How the game executable `exe` is started on `os`.
///
/// Windows: elevated for the game executables when "launch as administrator" is set, else
/// direct; only `.exe` and `.bat` run. Linux: by `mode`. Any other OS runs nothing.
// Mirrors dao/LauncherDAO.java:204-210, 231-232, 287-291, 346-348
pub fn arma3_method(
    os: Os,
    exe: &Path,
    options: &LauncherOptions,
    mode: LinuxLaunchMode,
) -> Result<LaunchMethod, LaunchError> {
    let name = file_name(exe);
    let kind = executable_kind(&name);
    match os {
        Os::Windows => match kind {
            ExecutableKind::Exe
                if options.launch_arma3_as_administrator && is_elevatable(&name) =>
            {
                Ok(LaunchMethod::Elevated)
            }
            ExecutableKind::Exe | ExecutableKind::Bat => Ok(LaunchMethod::Direct),
            ExecutableKind::Sh | ExecutableKind::Other => Err(invalid(name)),
        },
        Os::Linux => Ok(match (mode, kind) {
            (LinuxLaunchMode::Steam, _) | (LinuxLaunchMode::Auto, ExecutableKind::Exe) => {
                LaunchMethod::Steam
            }
            _ => LaunchMethod::Direct,
        }),
        Os::MacOs | Os::OtherUnix | Os::Other => Err(invalid(name)),
    }
}

/// The command that starts the game `exe` with `params` by `method`.
///
/// * Windows `.exe`: `[exe, params…]`, `[exe, "2", "1", params…]` for the BattlEye launcher;
///   parameters are not trimmed. Elevated: the same arguments through [`elevated_command`].
/// * Windows `.bat`: `cmd.exe /C start "" exe params…` (the empty title keeps a quoted path
///   from becoming the window title).
/// * Linux `.sh`: `/bin/bash exe params…`, so the script receives its arguments.
/// * Linux, anything else: `[exe, params…]`.
/// * Steam: `<steam> -applaunch 107410 -noLauncher params…`.
///
/// Except for the Windows `.exe`, parameters are Java-trimmed. Direct launches run in the
/// executable's folder.
// Mirrors dao/LauncherDAO.java:193-358
pub fn plan_arma3(
    os: Os,
    exe: &Path,
    params: &[String],
    method: LaunchMethod,
    steam: Option<&SteamCommand>,
) -> Result<LaunchPlan, LaunchError> {
    let name = file_name(exe);
    let exe_text = exe.to_string_lossy().into_owned();
    let mut game_args: Vec<String> = Vec::new();
    if is_battleye(&name) {
        game_args.extend(["2".to_owned(), "1".to_owned()]);
    }
    game_args.extend_from_slice(params);

    match method {
        LaunchMethod::Steam => {
            let steam = steam.ok_or(LaunchError::SteamNotFound)?;
            let (program, prefix_args) = steam
                .prefix
                .split_first()
                .ok_or(LaunchError::SteamNotFound)?;
            let mut args: Vec<OsString> = prefix_args.to_vec();
            args.extend(
                ["-applaunch", &ARMA3_APP_ID.to_string(), "-noLauncher"]
                    .into_iter()
                    .map(OsString::from),
            );
            args.extend(trimmed(params).map(OsString::from));
            let command = CommandSpec::new(program, args, None);
            Ok(LaunchPlan {
                method,
                shown_command: command.elements(),
                command,
            })
        }
        LaunchMethod::Elevated => {
            let working_dir = std::path::absolute(exe)
                .ok()
                .and_then(|absolute| absolute.parent().map(Path::to_path_buf))
                .map(|dir| dir.to_string_lossy().into_owned());
            let mut shown_command = vec![exe_text.clone()];
            shown_command.extend_from_slice(&game_args);
            Ok(LaunchPlan {
                method,
                command: elevated_command(&exe_text, working_dir.as_deref(), &game_args),
                shown_command,
            })
        }
        LaunchMethod::Direct => {
            let current_dir = parent_dir(exe);
            let command = match (os, executable_kind(&name)) {
                (Os::Windows, ExecutableKind::Exe) => CommandSpec::new(
                    exe.as_os_str(),
                    game_args.into_iter().map(OsString::from).collect(),
                    current_dir,
                ),
                (Os::Windows, ExecutableKind::Bat) => batch_command(exe, params, current_dir),
                (Os::Linux, ExecutableKind::Sh) => bash_command(exe, params, current_dir),
                (Os::Linux, _) => CommandSpec::new(
                    exe.as_os_str(),
                    trimmed(params).map(OsString::from).collect(),
                    current_dir,
                ),
                _ => return Err(invalid(name)),
            };
            Ok(LaunchPlan {
                method,
                shown_command: command.elements(),
                command,
            })
        }
    }
}

/// The command that starts the external application `exe` with `params`, or `None` when its
/// kind does not run on `os` (Java silently does nothing then).
///
/// Windows: `.exe` directly, `.bat` through `cmd.exe /C start ""`. Linux: `.sh` through
/// `/bin/bash`. Parameters are Java-trimmed; the application runs in its own folder.
// Mirrors dao/LauncherDAO.java:117-191
pub fn plan_external(
    os: Os,
    exe: &Path,
    params: &[String],
) -> Result<Option<CommandSpec>, LaunchError> {
    let name = file_name(exe);
    let current_dir = parent_dir(exe);
    Ok(match (executable_kind(&name), os) {
        (ExecutableKind::Exe, Os::Windows) => Some(CommandSpec::new(
            exe.as_os_str(),
            trimmed(params).map(OsString::from).collect(),
            current_dir,
        )),
        (ExecutableKind::Bat, Os::Windows) => Some(batch_command(exe, params, current_dir)),
        (ExecutableKind::Sh, Os::Linux) => Some(bash_command(exe, params, current_dir)),
        (ExecutableKind::Exe | ExecutableKind::Bat | ExecutableKind::Sh, _) => None,
        (ExecutableKind::Other, _) => return Err(invalid(name)),
    })
}

/// `cmd.exe /C start "" <exe> params…`.
fn batch_command(exe: &Path, params: &[String], current_dir: Option<PathBuf>) -> CommandSpec {
    let mut args: Vec<OsString> = ["/C", "start", ""]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.push(exe.as_os_str().to_owned());
    args.extend(trimmed(params).map(OsString::from));
    CommandSpec::new("cmd.exe", args, current_dir)
}

/// `/bin/bash <exe> params…`.
fn bash_command(exe: &Path, params: &[String], current_dir: Option<PathBuf>) -> CommandSpec {
    let mut args = vec![exe.as_os_str().to_owned()];
    args.extend(trimmed(params).map(OsString::from));
    CommandSpec::new("/bin/bash", args, current_dir)
}

fn trimmed(params: &[String]) -> impl Iterator<Item = &str> {
    params.iter().map(|param| java_trim(param))
}

/// The folder of `exe`, `None` for a bare file name.
fn parent_dir(exe: &Path) -> Option<PathBuf> {
    exe.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

fn invalid(name: String) -> LaunchError {
    LaunchError::InvalidExecutable { name }
}

const PASSWORD_PREFIX: &str = "-password=";

/// The command elements joined by single spaces, the value of every `-password=` replaced by
/// `*****`.
pub fn command_line(elements: &[String]) -> String {
    elements
        .iter()
        .map(|element| redact(element))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The line logged before the game starts: Java's text, each element followed by a space, with
/// `-password=` values replaced by `*****`.
// Mirrors dao/LauncherDAO.java:224-229
pub fn log_line(elements: &[String]) -> String {
    let mut line = String::from("Starting ArmA 3 with command line: ");
    for element in elements {
        line.push_str(&redact(element));
        line.push(' ');
    }
    line
}

fn redact(element: &str) -> String {
    match element.get(..PASSWORD_PREFIX.len()) {
        Some(prefix) if prefix.eq_ignore_ascii_case(PASSWORD_PREFIX) => {
            format!("{prefix}*****")
        }
        _ => element.to_owned(),
    }
}
