use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

use crate::paths::{CACHE_DIR_VAR, CONFIG_DIR_VAR, DATA_DIR_VAR, Error, PORTABLE_VAR};

/// Operating-system family, deciding which directory conventions apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Os {
    /// Windows (`APPDATA` / `LOCALAPPDATA`).
    Windows,

    /// Linux (XDG base directories).
    Linux,

    /// macOS (`~/Library`; Java used XDG paths here).
    MacOs,

    /// BSD systems (XDG base directories).
    OtherUnix,

    /// Anything else.
    Other,
}

impl Os {
    /// The operating system this binary was compiled for.
    ///
    /// Only the families Java's `os.name` checks recognise (`win`, `linux`, `bsd`, `mac`) get a
    /// dedicated variant; other Unix systems (Solaris/illumos, AIX, …) are [`Self::Other`].
    // Mirrors dao/ApplicationPaths.java:204, 213-238
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(any(target_os = "linux", target_os = "android")) {
            Self::Linux
        } else if cfg!(any(
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )) {
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
    "APPIMAGE",
    "PATH",
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
    /// Why the executable's directory is unknown (`current_exe` failure), for portable mode.
    exe_error: Option<(io::ErrorKind, String)>,
}

impl PlatformEnv {
    /// Reads the current process: OS, `HOME` (Unix) / `USERPROFILE` (Windows), the relevant
    /// environment variables and the directory of the current executable.
    ///
    /// Currently never fails: an unknown executable path leaves [`Self::exe_dir`] empty and is
    /// reported as [`Error::CurrentExe`] only if portable mode needs it.
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
        // A missing executable path only matters for portable mode, which reports it.
        let (exe_dir, exe_error) = match std::env::current_exe() {
            Ok(exe) => (exe.parent().map(Path::to_path_buf), None),
            Err(err) => (None, Some((err.kind(), err.to_string()))),
        };
        Ok(Self {
            os,
            home,
            vars,
            exe_dir,
            exe_error,
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
            exe_error: None,
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

    /// The executable's directory, or [`Error::CurrentExe`] explaining why it is unknown.
    pub(crate) fn require_exe_dir(&self) -> Result<&Path, Error> {
        self.exe_dir().ok_or_else(|| {
            let (kind, message) = self.exe_error.clone().unwrap_or_else(|| {
                (
                    io::ErrorKind::NotFound,
                    "the executable's directory is unknown".to_owned(),
                )
            });
            Error::CurrentExe(io::Error::new(kind, message))
        })
    }

    /// An environment variable; `None` when unset, blank (Java `isBlank`), or not valid UTF-8.
    pub fn var(&self, name: &str) -> Option<&str> {
        self.var_os(name).and_then(OsStr::to_str)
    }

    /// An environment variable; `None` when unset or blank (Java `String.isBlank`).
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

/// Java `String.isBlank`: empty or only `Character.isWhitespace` code points.
fn is_blank(value: &OsStr) -> bool {
    value.to_string_lossy().chars().all(is_java_whitespace)
}

/// Java `Character.isWhitespace`: the Unicode space, line and paragraph separators except the
/// non-breaking ones (U+00A0, U+2007, U+202F), plus `\t`, `\n`, U+000B, `\f`, `\r` and
/// U+001C–U+001F. U+0085 is not whitespace.
fn is_java_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{0B}'
            | '\u{0C}'
            | '\r'
            | '\u{1C}'..='\u{1F}'
            | ' '
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{205F}'
            | '\u{3000}'
    )
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
    fn blank_matches_java_character_is_whitespace() {
        for blank in [
            "\u{0B}\u{0C}",
            "\u{1C}\u{1D}\u{1E}\u{1F}",
            "\u{1680}",
            "\u{2000}\u{2006}\u{2008}\u{200A}",
            "\u{2028}\u{2029}",
            "\u{205F}\u{3000}",
        ] {
            assert!(is_blank(OsStr::new(blank)), "{blank:?}");
        }
        for not_blank in [
            "\u{A0}", "\u{2007}", "\u{202F}", "\u{85}", "\u{200B}", "\u{FEFF}",
        ] {
            assert!(!is_blank(OsStr::new(not_blank)), "{not_blank:?}");
        }
    }

    #[test]
    fn missing_exe_dir_is_reported_as_current_exe() {
        let env = PlatformEnv {
            exe_error: Some((io::ErrorKind::PermissionDenied, "no /proc".to_owned())),
            ..PlatformEnv::for_tests(Os::Linux, None, &[], None)
        };
        let Err(Error::CurrentExe(err)) = env.require_exe_dir() else {
            panic!("expected CurrentExe");
        };
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(err.to_string(), "no /proc");

        let env = PlatformEnv::for_tests(Os::Linux, None, &[], None);
        assert!(matches!(env.require_exe_dir(), Err(Error::CurrentExe(_))));
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
