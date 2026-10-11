//! Desktop shortcuts that start the game with a profile: Windows `.lnk` files and Linux
//! `.desktop` launchers.

use std::fs;
use std::path::{Component, Path, PathBuf};

use a3s_foundation::command_line::windows_args;
use a3s_foundation::paths::{Os, PlatformEnv};

use crate::folders::SpecialFolders;
use crate::fs_write::write_atomic;
use crate::{Error, Result};

/// The texts of the "export profile as shortcut" flow.
// Mirrors ui/main/MainPanel.java:145-148, 690-734
pub mod strings {
    /// Title of every dialog of the flow.
    pub const TITLE: &str = "Export profile as shortcut";

    /// Shown where shortcuts are unsupported.
    pub const NOT_AVAILABLE: &str = "This feature is not available for your system.";

    /// The menu item in the Profiles menu.
    pub const MENU_ITEM: &str = "Shortcut";

    /// Shown when the profile has no Arma 3 executable.
    pub fn exe_missing(profile: &str) -> String {
        format!(
            "ArmA 3 Executable location is missing for profile name {profile}.\nPlease checkout Launcher Options panel."
        )
    }

    /// Shown after the shortcut was written.
    pub fn created(profile: &str) -> String {
        format!("Shortcut has been created on desktop for profile {profile}.")
    }

    /// Shown when writing failed.
    pub fn failed(message: &str) -> String {
        format!("Failed to create shortcut\n{message}")
    }
}

/// Longest command line Windows accepts (`CreateProcess`), in UTF-16 units; `.lnk` files store
/// string lengths as 16-bit counts.
const MAX_COMMAND_LINE_UNITS: usize = 32_767;

/// Longest `.lnk` target path, in UTF-16 units. The writer sums the target's per-folder records
/// into a 16-bit size, which a longer path could overflow.
const MAX_LNK_TARGET_UNITS: usize = 1_024;

/// What a shortcut starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutSpec {
    /// The shortcut's name, used as its file name.
    pub name: String,

    /// The program to start.
    pub target: PathBuf,

    /// The program's arguments.
    pub args: Vec<String>,

    /// The working directory; the target's folder when `None` (`.lnk` only).
    pub working_dir: Option<PathBuf>,

    /// The icon file.
    pub icon: Option<PathBuf>,

    /// A description shown as tooltip.
    pub comment: Option<String>,
}

/// Checks that `name` can be a file name on every OS: not blank, not `.` or `..`, and free of
/// `\ / : * ? " < > |` and control characters.
pub fn validate_shortcut_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    let invalid = trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if invalid {
        return Err(Error::InvalidShortcutName(name.to_owned()));
    }
    Ok(())
}

/// The `Exec` command of a desktop entry, before [`desktop_escape_value`]: the program and its
/// arguments quoted by the Desktop Entry `Exec` rules, `%` doubled.
pub fn desktop_exec(program: &Path, args: &[String]) -> Result<String> {
    let program = program
        .to_str()
        .ok_or_else(|| Error::InvalidInput(format!("{} is not UTF-8", program.display())))?;
    let mut words = Vec::with_capacity(args.len() + 1);
    for word in std::iter::once(program).chain(args.iter().map(String::as_str)) {
        if word.contains(['\n', '\0']) {
            return Err(Error::InvalidInput(format!(
                "{word:?} cannot be part of a desktop entry command"
            )));
        }
        let needs_quotes = word.is_empty()
            || word.contains([
                ' ', '\t', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
                ')', '`',
            ]);
        let word = word.replace('%', "%%");
        if needs_quotes {
            let mut quoted = String::with_capacity(word.len() + 2);
            quoted.push('"');
            for c in word.chars() {
                if matches!(c, '"' | '`' | '$' | '\\') {
                    quoted.push('\\');
                }
                quoted.push(c);
            }
            quoted.push('"');
            words.push(quoted);
        } else {
            words.push(word);
        }
    }
    Ok(words.join(" "))
}

/// Escapes a desktop entry string value: `\` → `\\`, newline → `\n`, tab → `\t`, CR → `\r`, and
/// a leading space → `\s`.
pub fn desktop_escape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (index, c) in s.chars().enumerate() {
        match c {
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\t' => out.push_str(r"\t"),
            '\r' => out.push_str(r"\r"),
            ' ' if index == 0 => out.push_str(r"\s"),
            _ => out.push(c),
        }
    }
    out
}

fn utf8_path(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::InvalidInput(format!("{} is not UTF-8", path.display())))
}

/// The text of a `.desktop` launcher for `spec`.
pub fn render_desktop_entry(spec: &ShortcutSpec) -> Result<String> {
    let mut text = String::from("[Desktop Entry]\nType=Application\nVersion=1.0\n");
    text.push_str(&format!("Name={}\n", desktop_escape_value(&spec.name)));
    text.push_str(&format!(
        "Exec={}\n",
        desktop_escape_value(&desktop_exec(&spec.target, &spec.args)?)
    ));
    if let Some(dir) = &spec.working_dir {
        text.push_str(&format!("Path={}\n", desktop_escape_value(utf8_path(dir)?)));
    }
    if let Some(icon) = &spec.icon {
        text.push_str(&format!(
            "Icon={}\n",
            desktop_escape_value(utf8_path(icon)?)
        ));
    }
    if let Some(comment) = &spec.comment {
        text.push_str(&format!("Comment={}\n", desktop_escape_value(comment)));
    }
    text.push_str("Terminal=false\n");
    Ok(text)
}

/// Writes a `.desktop` launcher for `spec` to `path` (atomically), executable as KDE and GNOME
/// require for launchers on the desktop.
pub fn write_desktop_entry(spec: &ShortcutSpec, path: &Path) -> Result<()> {
    let text = render_desktop_entry(spec)?;
    write_atomic(path, text.as_bytes(), Some(0o755))
}

fn check_lnk_string(what: &str, text: &str, max_units: usize) -> Result<()> {
    if text.encode_utf16().count() > max_units {
        return Err(Error::InvalidInput(format!(
            "the shortcut {what} is longer than {max_units} characters"
        )));
    }
    Ok(())
}

/// The validated strings of a `.lnk` file.
#[cfg_attr(
    not(windows),
    expect(dead_code, reason = "only the Windows writer reads them")
)]
struct LnkParts<'a> {
    arguments: Option<String>,
    working_dir: Option<&'a str>,
    icon: Option<&'a str>,
}

/// Writes a Windows shortcut for `spec` to `path`, replacing an existing file.
///
/// The target must be an existing file or folder given as an absolute UTF-8 path with at least
/// two components (on Windows: on a drive letter, not a UNC share). These checks run on every OS,
/// before the `.lnk` writer, whose internal assumptions they guarantee. The writer itself only
/// exists on Windows; elsewhere a valid spec gives [`Error::Unsupported`].
pub fn write_lnk(spec: &ShortcutSpec, path: &Path) -> Result<()> {
    let parts = check_lnk(spec)?;
    save_lnk(spec, &parts, path)
}

#[cfg(windows)]
fn save_lnk(spec: &ShortcutSpec, parts: &LnkParts<'_>, path: &Path) -> Result<()> {
    let lnk_error = |err: mslnk::MSLinkError| Error::Lnk(err.to_string());
    let mut link = mslnk::ShellLink::new(&spec.target).map_err(lnk_error)?;
    if let Some(dir) = parts.working_dir {
        link.set_working_dir(Some(dir.to_owned()));
    }
    link.set_arguments(parts.arguments.clone());
    if let Some(icon) = parts.icon {
        link.set_icon_location(Some(icon.to_owned()));
    }
    if let Some(comment) = &spec.comment {
        link.set_name(Some(comment.clone()));
    }
    link.create_lnk(path).map_err(lnk_error)
}

#[cfg(not(windows))]
fn save_lnk(_spec: &ShortcutSpec, _parts: &LnkParts<'_>, _path: &Path) -> Result<()> {
    Err(Error::Unsupported)
}

fn check_lnk(spec: &ShortcutSpec) -> Result<LnkParts<'_>> {
    let target = &spec.target;
    let target_text = utf8_path(target)?;
    let normal_components = target
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    if !target.is_absolute() || target.parent().is_none() || normal_components < 2 {
        return Err(Error::InvalidInput(format!(
            "the shortcut target {} is not an absolute file path",
            target.display()
        )));
    }
    if fs::metadata(target).is_err() {
        return Err(Error::ShortcutTargetMissing(target.clone()));
    }
    // The writer records the target below a drive letter; UNC and verbatim paths have none.
    let bytes = target_text.as_bytes();
    let on_drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if cfg!(windows) && !on_drive {
        return Err(Error::Unsupported);
    }
    check_lnk_string("target", target_text, MAX_LNK_TARGET_UNITS)?;
    let arguments = windows_args(&spec.args);
    check_lnk_string("arguments", &arguments, MAX_COMMAND_LINE_UNITS)?;
    let working_dir = spec.working_dir.as_deref().map(utf8_path).transpose()?;
    let icon = spec.icon.as_deref().map(utf8_path).transpose()?;
    for (what, text) in [
        ("working folder", working_dir),
        ("icon", icon),
        ("comment", spec.comment.as_deref()),
    ] {
        if let Some(text) = text {
            check_lnk_string(what, text, MAX_COMMAND_LINE_UNITS)?;
        }
    }
    Ok(LnkParts {
        arguments: (!spec.args.is_empty()).then_some(arguments),
        working_dir,
        icon,
    })
}

/// Creates a shortcut for `spec` on the user's desktop and returns its path: `<name>.lnk` on
/// Windows, `<name>.desktop` on Linux and BSD. An existing shortcut is replaced.
pub fn create_desktop_shortcut(
    env: &PlatformEnv,
    folders: &dyn SpecialFolders,
    spec: &ShortcutSpec,
) -> Result<PathBuf> {
    let extension = match env.os() {
        Os::Windows => "lnk",
        Os::Linux | Os::OtherUnix => "desktop",
        Os::MacOs | Os::Other => return Err(Error::Unsupported),
    };
    validate_shortcut_name(&spec.name)?;
    let desktop = folders.desktop().ok_or(Error::NoDesktopDir)?;
    let path = desktop.join(format!("{}.{extension}", spec.name));
    if env.os() == Os::Windows {
        write_lnk(spec, &path)?;
    } else {
        write_desktop_entry(spec, &path)?;
    }
    Ok(path)
}
