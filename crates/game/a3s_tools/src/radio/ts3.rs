//! TS3 installation helpers and the installer fields' initial values.

use std::path::Path;

/// The 64-bit TS3 client executable.
pub const TS3_EXE_64: &str = "ts3client_win64.exe";

/// The 32-bit TS3 client executable.
pub const TS3_EXE_32: &str = "ts3client_win32.exe";

/// Architecture of a TS3 installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ts3Arch {
    /// [`TS3_EXE_64`] is present.
    X64,

    /// Only [`TS3_EXE_32`] is present.
    X86,
}

/// Whether `dir` holds the 64-bit client. An empty `dir` holds nothing.
// Mirrors service/ConfigurationService.java:271-281
pub fn is_ts3_64bit(dir: &Path) -> bool {
    has_file(dir, TS3_EXE_64)
}

/// Whether `dir` holds the 32-bit client. An empty `dir` holds nothing.
// Mirrors service/ConfigurationService.java:283-293
pub fn is_ts3_32bit(dir: &Path) -> bool {
    has_file(dir, TS3_EXE_32)
}

/// The installation's architecture; 64-bit wins when both executables are present.
pub fn ts3_arch(dir: &Path) -> Option<Ts3Arch> {
    if is_ts3_64bit(dir) {
        Some(Ts3Arch::X64)
    } else if is_ts3_32bit(dir) {
        Some(Ts3Arch::X86)
    } else {
        None
    }
}

/// The client version in a TS3 `changelog.txt`.
///
/// The first line that contains `client release` (any case) and has at least four tokens
/// separated by spaces (runs of spaces collapse; tabs do not separate) gives its fourth token:
/// `=== Client Release 3.1.10 ===` gives `3.1.10`.
// Mirrors dao/ConfigurationDAO.java:96-112
pub fn parse_ts3_version(changelog: &str) -> Option<String> {
    changelog
        .split('\n')
        .flat_map(|line| line.split('\r'))
        .filter(|line| line.to_lowercase().contains("client release"))
        .find_map(|line| line.split(' ').filter(|token| !token.is_empty()).nth(3))
        .map(str::to_owned)
}

/// The version from `dir/changelog.txt`, or `None` when it is missing, unreadable or has no
/// version line.
// Mirrors dao/ConfigurationDAO.java:87-120
pub fn ts3_version(dir: &Path) -> Option<String> {
    if dir.as_os_str().is_empty() {
        return None;
    }
    let bytes = std::fs::read(dir.join("changelog.txt")).ok()?;
    parse_ts3_version(&String::from_utf8_lossy(&bytes))
}

/// The stored TS3 folder when one is stored (even an empty one), else what `detect` finds
/// (on Windows, the TS3 registry entry).
// Mirrors service/ConfigurationService.java:188-198
pub fn stored_or_detected(
    stored: Option<&str>,
    detect: impl FnOnce() -> Option<String>,
) -> Option<String> {
    match stored {
        Some(path) => Some(path.to_owned()),
        None => detect(),
    }
}

/// The initial text of a folder field: `path` when it exists, else empty.
// Mirrors ui/tools/tfar/FirstPageTFARInstallerPanel.java:283-291
pub fn existing_or_empty(path: Option<&str>) -> String {
    path.filter(|path| Path::new(path).exists())
        .map(str::to_owned)
        .unwrap_or_default()
}

/// The Arma 3 folder field's initial value: the absolute parent folder of the profile's Arma 3
/// executable, or `None` when there is no executable or it has no parent folder.
// Mirrors ui/tools/tfar/FirstPageTFARInstallerPanel.java:273-280
pub fn arma3_dir_from_exe(exe: Option<&str>) -> Option<String> {
    let parent = Path::new(exe?).parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    let absolute = std::path::absolute(parent).ok()?;
    Some(absolute.to_string_lossy().into_owned())
}

/// Whether `dir/name` exists; an empty `dir` never contains anything.
fn has_file(dir: &Path, name: &str) -> bool {
    !dir.as_os_str().is_empty() && dir.join(name).exists()
}
