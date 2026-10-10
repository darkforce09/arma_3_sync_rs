use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::{CACHE_DIR_VAR, CONFIG_DIR_VAR, DATA_DIR_VAR, Error, PORTABLE_VAR};

/// Operating-system family, deciding which directory conventions apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Os {
    /// Windows (`APPDATA` / `LOCALAPPDATA`).
    Windows,

    /// Linux (XDG base directories).
    Linux,

    /// macOS (`~/Library`; Java used XDG paths here).
    MacOs,

    /// BSD and other Unix systems (XDG base directories).
    OtherUnix,

    /// Anything else.
    Other,
}

impl Os {
    /// The operating system this binary was compiled for.
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(unix) {
            Self::OtherUnix
        } else {
            Self::Other
        }
    }

    /// Whether `value` is an absolute path on this OS, independently of the host running the
    /// code (so tests of every OS run anywhere).
    pub(crate) fn is_absolute(self, value: &OsStr) -> bool {
        let text = value.to_string_lossy();
        match self {
            Self::Windows => {
                let bytes = text.as_bytes();
                text.starts_with("\\\\")
                    || (bytes.len() >= 3
                        && bytes[0].is_ascii_alphabetic()
                        && bytes[1] == b':'
                        && (bytes[2] == b'\\' || bytes[2] == b'/'))
            }
            Self::Linux | Self::MacOs | Self::OtherUnix | Self::Other => text.starts_with('/'),
        }
    }
}

/// Environment variables read by [`PlatformEnv::current`].
const VARS: &[&str] = &[
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    CONFIG_DIR_VAR,
    DATA_DIR_VAR,
    CACHE_DIR_VAR,
    PORTABLE_VAR,
];

/// Snapshot of everything directory resolution depends on: OS, home directory, environment
/// variables and the directory of the executable.
///
/// [`Self::current`] reads the process; [`Self::for_tests`] injects values, so no test needs
/// `std::env::set_var`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformEnv {
    os: Os,
    home: Option<PathBuf>,
    vars: BTreeMap<String, OsString>,
    exe_dir: Option<PathBuf>,
}

impl PlatformEnv {
    /// Reads the current process: OS, `HOME` (Unix) / `USERPROFILE` (Windows), the relevant
    /// environment variables and the directory of the current executable.
    pub fn current() -> Result<Self, Error> {
        let os = Os::current();
        let home_var = if os == Os::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        };
        let home = std::env::var_os(home_var)
            .filter(|value| !is_blank(value))
            .map(PathBuf::from);
        let vars = VARS
            .iter()
            .filter_map(|&name| std::env::var_os(name).map(|value| (name.to_owned(), value)))
            .collect();
        let exe = std::env::current_exe().map_err(Error::CurrentExe)?;
        let exe_dir = exe.parent().map(Path::to_path_buf);
        Ok(Self {
            os,
            home,
            vars,
            exe_dir,
        })
    }

    /// Builds an environment from explicit values (for tests and tools).
    pub fn for_tests(
        os: Os,
        home: Option<PathBuf>,
        vars: &[(&str, &str)],
        exe_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            os,
            home,
            vars: vars
                .iter()
                .map(|&(name, value)| (name.to_owned(), OsString::from(value)))
                .collect(),
            exe_dir,
        }
    }

    /// The operating-system family.
    pub fn os(&self) -> Os {
        self.os
    }

    /// The user's home directory, if known.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// The directory of the current executable, if known.
    pub fn exe_dir(&self) -> Option<&Path> {
        self.exe_dir.as_deref()
    }

    /// An environment variable; `None` when unset, blank after trimming, or not valid UTF-8.
    pub fn var(&self, name: &str) -> Option<&str> {
        self.var_os(name).and_then(OsStr::to_str)
    }

    /// An environment variable; `None` when unset or blank after trimming (Java `isBlank`).
    // Mirrors dao/ApplicationPaths.java:207, 232
    pub fn var_os(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .get(name)
            .map(OsString::as_os_str)
            .filter(|value| !is_blank(value))
    }

    /// An environment variable as a path, `None` when unset or blank.
    pub(crate) fn var_path(&self, name: &str) -> Option<PathBuf> {
        self.var_os(name).map(PathBuf::from)
    }

    /// The home directory or [`Error::NoHomeDir`].
    pub(crate) fn require_home(&self) -> Result<&Path, Error> {
        self.home().ok_or(Error::NoHomeDir)
    }
}

fn is_blank(value: &OsStr) -> bool {
    value.to_string_lossy().trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_vars_count_as_unset() {
        let env = PlatformEnv::for_tests(
            Os::Linux,
            None,
            &[("A", " \t"), ("B", ""), ("C", " x ")],
            None,
        );
        assert_eq!(env.var("A"), None);
        assert_eq!(env.var("B"), None);
        assert_eq!(env.var("C"), Some(" x "));
        assert_eq!(env.var("D"), None);
    }

    #[test]
    fn getters_return_injected_values() {
        let env = PlatformEnv::for_tests(
            Os::MacOs,
            Some(PathBuf::from("/Users/u")),
            &[],
            Some(PathBuf::from("/opt/a3s")),
        );
        assert_eq!(env.os(), Os::MacOs);
        assert_eq!(env.home(), Some(Path::new("/Users/u")));
        assert_eq!(env.exe_dir(), Some(Path::new("/opt/a3s")));
        let env = PlatformEnv::for_tests(Os::Windows, None, &[], None);
        assert_eq!(env.os(), Os::Windows);
        assert_eq!(env.home(), None);
    }

    #[test]
    fn current_reads_the_process() {
        let env = PlatformEnv::current().expect("current executable is known in tests");
        assert_eq!(env.os(), Os::current());
        assert!(env.exe_dir().is_some());
    }

    #[test]
    fn absolute_paths_per_os() {
        assert!(Os::Linux.is_absolute(OsStr::new("/x")));
        assert!(!Os::Linux.is_absolute(OsStr::new("x/y")));
        assert!(Os::Windows.is_absolute(OsStr::new("C:\\Users")));
        assert!(Os::Windows.is_absolute(OsStr::new("\\\\server\\share")));
        assert!(!Os::Windows.is_absolute(OsStr::new("Users")));
    }
}
