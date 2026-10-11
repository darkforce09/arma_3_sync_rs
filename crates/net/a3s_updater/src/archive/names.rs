//! Archive entry names that are safe on every platform.

use crate::UpdateError;

/// Windows device names, reserved with any extension (`CON`, `nul.txt`, `COM1.log`).
const DEVICE_NAMES: [&str; 4] = ["con", "prn", "aux", "nul"];
const NUMBERED_DEVICES: [&str; 2] = ["com", "lpt"];

/// Normalizes an archive entry name to a `/`-separated path strictly inside the extraction
/// root; `""` is the root itself.
///
/// `\` counts as a separator; `.` components are dropped and `a/../b` becomes `b`. Rejected: a
/// blank name, a rooted name (`/x`, `//server/x`), any `..` that leaves the root, and any
/// component that is not a portable file name (see [`is_portable_file_name`]).
// Mirrors a3sUpdater/dao/FtpDAO.java:246-260
pub(crate) fn normalize_entry_name(name: &str) -> Result<String, UpdateError> {
    if name.trim().is_empty() {
        return Err(UpdateError::EmptyEntryName);
    }
    let unsafe_entry = || UpdateError::UnsafeEntry(name.to_owned());
    let portable = name.replace('\\', "/");
    if portable.starts_with('/') {
        return Err(unsafe_entry());
    }
    let mut parts: Vec<&str> = Vec::new();
    for component in portable.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or_else(unsafe_entry)?;
            }
            _ if !is_portable_file_name(component) => return Err(unsafe_entry()),
            _ => parts.push(component),
        }
    }
    Ok(parts.join("/"))
}

/// Whether `name` is one path component every supported file system stores as written: no
/// control character or any of `<>:"/\|?*` (`:` also names NTFS alternate data streams), no
/// trailing `.` or space (Windows drops them), and not a Windows device name.
pub(crate) fn is_portable_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
        && !name.ends_with(['.', ' '])
        && !is_device_name(name)
}

fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_lowercase();
    DEVICE_NAMES.contains(&stem.as_str())
        || NUMBERED_DEVICES.iter().any(|device| {
            stem.strip_prefix(device).is_some_and(|number| {
                matches!(
                    number,
                    "0" | "1"
                        | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        | "\u{b9}"
                        | "\u{b2}"
                        | "\u{b3}"
                )
            })
        })
}
