//! Starting the client with the OS: the per-user `Run` registry value on Windows, an XDG
//! autostart entry on Linux and BSD, a `LaunchAgent` on macOS.
//!
//! The entries are named `arma3sync-rs`, so they never collide with the Java client's
//! machine-wide `Run\ArmA3Sync` value, which is never read or touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use a3s_foundation::paths::{Os, PlatformEnv};

use crate::fs_write::{remove_if_exists, write_atomic};
use crate::registry::{Hive, RegQuery, RegistryRead, RegistryWrite};
use crate::shortcut::{desktop_escape_value, desktop_exec, quote_windows_arg};
use crate::{Error, Result};

/// Name of the `Run` value and of the XDG autostart file (`<name>.desktop`).
pub const AUTOSTART_NAME: &str = "arma3sync-rs";

/// The per-user registry key whose values Windows runs at logon.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Label (and file stem) of the macOS `LaunchAgent`.
pub const LAUNCH_AGENT_LABEL: &str = "com.github.vpzbrig21.arma3sync-rs";

/// Shown when the `Run` value cannot be written.
// Mirrors dao/PreferencesDAO.java:69
pub const FAILED_TO_WRITE_REGISTRY: &str = "Failed to write into Windows registry.";

/// The command an autostart entry runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutostartEntry {
    /// The executable.
    pub exe: PathBuf,

    /// Its arguments (`["-run"]` starts minimised to the tray).
    pub args: Vec<String>,
}

impl AutostartEntry {
    /// An entry that starts this executable: `$APPIMAGE` on Linux when set (the running binary
    /// lives in a mount that disappears on exit), else [`std::env::current_exe`].
    pub fn for_current_process(env: &PlatformEnv, args: Vec<String>) -> Result<Self> {
        let appimage = (env.os() == Os::Linux)
            .then(|| env.var("APPIMAGE"))
            .flatten();
        let exe = match appimage {
            Some(appimage) => PathBuf::from(appimage),
            None => std::env::current_exe().map_err(|source| Error::Io { path: None, source })?,
        };
        Ok(Self { exe, args })
    }
}

/// Whether autostart is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutostartStatus {
    /// No (active) entry.
    Disabled,

    /// An entry exists and runs `command`.
    Enabled {
        /// The command line the entry runs.
        command: String,
    },
}

/// A registry that can be read and written.
pub trait RegistryRw: RegistryRead + RegistryWrite {}

impl<T: RegistryRead + RegistryWrite> RegistryRw for T {}

/// Where this OS keeps autostart entries.
pub enum AutostartBackend<'a> {
    /// The `HKCU\…\Run` value [`AUTOSTART_NAME`].
    WindowsRun(&'a dyn RegistryRw),

    /// `<dir>/arma3sync-rs.desktop`, `dir` being the XDG autostart folder.
    XdgAutostart {
        /// The autostart folder.
        dir: PathBuf,
    },

    /// `<dir>/com.github.vpzbrig21.arma3sync-rs.plist`, `dir` being `~/Library/LaunchAgents`.
    LaunchAgent {
        /// The `LaunchAgents` folder.
        dir: PathBuf,
    },

    /// No autostart on this OS (or no home folder to put it in).
    Unsupported,
}

/// The backend for `env`'s OS. The Linux and macOS folders come from `env` (home and
/// `XDG_CONFIG_HOME`, used only when absolute).
pub fn autostart_backend<'a>(
    env: &PlatformEnv,
    registry: &'a dyn RegistryRw,
) -> AutostartBackend<'a> {
    match env.os() {
        Os::Windows => AutostartBackend::WindowsRun(registry),
        Os::Linux | Os::OtherUnix => env
            .var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| env.home().map(|home| home.join(".config")))
            .map_or(AutostartBackend::Unsupported, |config| {
                AutostartBackend::XdgAutostart {
                    dir: config.join("autostart"),
                }
            }),
        Os::MacOs => env.home().map_or(AutostartBackend::Unsupported, |home| {
            AutostartBackend::LaunchAgent {
                dir: home.join("Library").join("LaunchAgents"),
            }
        }),
        Os::Other => AutostartBackend::Unsupported,
    }
}

impl AutostartBackend<'_> {
    /// Creates or replaces the entry.
    // Mirrors dao/PreferencesDAO.java:55-73
    pub fn enable(&self, entry: &AutostartEntry) -> Result<()> {
        match self {
            Self::WindowsRun(registry) => registry
                .set_string(
                    Hive::CurrentUser,
                    RUN_KEY,
                    AUTOSTART_NAME,
                    &windows_run_command(entry),
                )
                .map_err(registry_error),
            Self::XdgAutostart { dir } => {
                let text = xdg_autostart_file(entry)?;
                create_dir(dir)?;
                write_atomic(&dir.join(xdg_file_name()), text.as_bytes(), None)
            }
            Self::LaunchAgent { dir } => {
                create_dir(dir)?;
                write_atomic(
                    &dir.join(launch_agent_file_name()),
                    launch_agent_plist(entry).as_bytes(),
                    None,
                )
            }
            Self::Unsupported => Err(Error::Unsupported),
        }
    }

    /// Removes the entry; a missing entry is not an error.
    // Mirrors dao/PreferencesDAO.java:76-87
    pub fn disable(&self) -> Result<()> {
        match self {
            Self::WindowsRun(registry) => registry
                .delete_value(Hive::CurrentUser, RUN_KEY, AUTOSTART_NAME)
                .map_err(registry_error),
            Self::XdgAutostart { dir } => remove_if_exists(&dir.join(xdg_file_name())),
            Self::LaunchAgent { dir } => remove_if_exists(&dir.join(launch_agent_file_name())),
            Self::Unsupported => Err(Error::Unsupported),
        }
    }

    /// Whether an active entry exists, and what it runs.
    pub fn status(&self) -> Result<AutostartStatus> {
        match self {
            Self::WindowsRun(registry) => {
                let query = RegQuery::new(Hive::CurrentUser, RUN_KEY, Some(AUTOSTART_NAME));
                Ok(
                    match registry.read_string(&query).map_err(registry_error)? {
                        Some(command) => AutostartStatus::Enabled { command },
                        None => AutostartStatus::Disabled,
                    },
                )
            }
            Self::XdgAutostart { dir } => Ok(read_optional(&dir.join(xdg_file_name()))?
                .map_or(AutostartStatus::Disabled, |text| xdg_status(&text))),
            Self::LaunchAgent { dir } => Ok(read_optional(&dir.join(launch_agent_file_name()))?
                .map_or(AutostartStatus::Disabled, |text| AutostartStatus::Enabled {
                    command: launch_agent_command(&text),
                })),
            Self::Unsupported => Err(Error::Unsupported),
        }
    }
}

fn registry_error(source: io::Error) -> Error {
    Error::Registry {
        key: format!(r"{}\{RUN_KEY}", Hive::CurrentUser),
        source,
    }
}

fn create_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|source| Error::io(dir, source))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::io(path, source)),
    }
}

fn xdg_file_name() -> String {
    format!("{AUTOSTART_NAME}.desktop")
}

fn launch_agent_file_name() -> String {
    format!("{LAUNCH_AGENT_LABEL}.plist")
}

/// The `Run` value: the quoted executable, then each argument quoted by Windows rules, all
/// separated by single spaces (`"C:\…\arma3sync.exe" -run`).
pub fn windows_run_command(entry: &AutostartEntry) -> String {
    let mut command = format!("\"{}\"", entry.exe.display());
    for arg in &entry.args {
        command.push(' ');
        command.push_str(&quote_windows_arg(arg));
    }
    command
}

/// The XDG autostart desktop entry for `entry`.
pub fn xdg_autostart_file(entry: &AutostartEntry) -> Result<String> {
    let exec = desktop_escape_value(&desktop_exec(&entry.exe, &entry.args)?);
    Ok(format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=ArmA3Sync\nExec={exec}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    ))
}

/// Reads an autostart desktop entry: disabled by `Hidden=true` or
/// `X-GNOME-Autostart-enabled=false`, otherwise enabled with its `Exec` value.
fn xdg_status(text: &str) -> AutostartStatus {
    let mut command = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (key.trim(), value.trim()) {
            ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => {
                return AutostartStatus::Disabled;
            }
            ("Exec", exec) => exec.clone_into(&mut command),
            _ => {}
        }
    }
    AutostartStatus::Enabled { command }
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The `LaunchAgent` property list for `entry`: runs the executable with its arguments at login.
pub fn launch_agent_plist(entry: &AutostartEntry) -> String {
    let mut arguments = format!(
        "\t\t<string>{}</string>\n",
        xml_escape(&entry.exe.to_string_lossy())
    );
    for arg in &entry.args {
        arguments.push_str(&format!("\t\t<string>{}</string>\n", xml_escape(arg)));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{LAUNCH_AGENT_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         {arguments}\
         \t</array>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         </dict>\n\
         </plist>\n"
    )
}

/// The program arguments of a `LaunchAgent` written by [`launch_agent_plist`], space-separated.
fn launch_agent_command(plist: &str) -> String {
    let Some(start) = plist.find("<array>") else {
        return String::new();
    };
    let array = &plist[start..];
    let array = &array[..array.find("</array>").unwrap_or(array.len())];
    array
        .split("<string>")
        .skip(1)
        .filter_map(|part| {
            part.split_once("</string>")
                .map(|(text, _)| xml_unescape(text))
        })
        .collect::<Vec<_>>()
        .join(" ")
}
