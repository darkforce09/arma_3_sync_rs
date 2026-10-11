use std::io;
use std::path::{Path, PathBuf};

use crate::paths::{
    APP_DIR_NAME, CACHE_DIR_VAR, CONFIG_DIR_VAR, DATA_DIR_VAR, Error, Os, PORTABLE_DATA_DIR,
    PORTABLE_MARKER, PORTABLE_VAR, PlatformEnv,
};

/// The Rust client's own directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    portable: bool,
}

/// Which of the three roots is being resolved.
#[derive(Clone, Copy)]
enum Kind {
    Config,
    Data,
    Cache,
}

impl AppDirs {
    /// Resolves the directories for `env`.
    ///
    /// Precedence per directory: the `A3S_CONFIG_DIR` / `A3S_DATA_DIR` / `A3S_CACHE_DIR`
    /// override (used as the final directory), then portable mode
    /// (`<exe_dir>/arma3sync-rs-data/{config,data,cache}`), then the platform convention.
    pub fn resolve(env: &PlatformEnv) -> Result<Self, Error> {
        let portable = is_portable(env);
        let resolve_one = |kind: Kind| -> Result<PathBuf, Error> {
            let override_var = match kind {
                Kind::Config => CONFIG_DIR_VAR,
                Kind::Data => DATA_DIR_VAR,
                Kind::Cache => CACHE_DIR_VAR,
            };
            if let Some(path) = env.var_path(override_var) {
                return Ok(path);
            }
            if portable {
                return portable_root(env, kind);
            }
            platform_root(env, kind)
        };
        Ok(Self {
            config: resolve_one(Kind::Config)?,
            data: resolve_one(Kind::Data)?,
            cache: resolve_one(Kind::Cache)?,
            portable,
        })
    }

    /// Builds directories from explicit roots (tests, importers). Not portable.
    pub fn from_roots(config: PathBuf, data: PathBuf, cache: PathBuf) -> Self {
        Self {
            config,
            data,
            cache,
            portable: false,
        }
    }

    /// Whether portable mode is active.
    pub fn is_portable(&self) -> bool {
        self.portable
    }

    /// Configuration directory (small, roamable settings).
    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    /// Data directory (repositories, logs).
    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    /// Cache directory (temporary and re-creatable files).
    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    /// Profiles directory, `config/profiles`.
    // Mirrors dao/ApplicationPaths.java:117-122
    pub fn profiles_dir(&self) -> PathBuf {
        self.config.join("profiles")
    }

    /// Repositories directory, `data/repositories`.
    // Mirrors dao/ApplicationPaths.java:131-137
    pub fn repositories_dir(&self) -> PathBuf {
        self.data.join("repositories")
    }

    /// Temporary files directory, `cache/temp`.
    // Mirrors dao/ApplicationPaths.java:139-141
    pub fn temp_dir(&self) -> PathBuf {
        self.cache.join("temp")
    }

    /// Debug log directory, `data/logs` (Java wrote its log into the configuration folder).
    pub fn log_dir(&self) -> PathBuf {
        self.data.join("logs")
    }

    /// All directories [`Self::ensure_created`] creates.
    pub fn all_dirs(&self) -> [PathBuf; 7] {
        [
            self.config.clone(),
            self.data.clone(),
            self.cache.clone(),
            self.temp_dir(),
            self.log_dir(),
            self.profiles_dir(),
            self.repositories_dir(),
        ]
    }

    /// Creates the config, data, cache, temp, log, profiles and repositories directories.
    // Mirrors main/ArmA3Sync.java:105-116
    pub fn ensure_created(&self) -> Result<(), Error> {
        for dir in self.all_dirs() {
            create_dir_all(&dir)?;
        }
        Ok(())
    }
}

fn create_dir_all(path: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(path).map_err(|source: io::Error| Error::CreateDir {
        path: path.to_path_buf(),
        source,
    })
}

/// Portable iff `A3S_PORTABLE` is `true` (ignoring ASCII case, Java `Boolean.parseBoolean`) or
/// the marker file exists next to the executable.
fn is_portable(env: &PlatformEnv) -> bool {
    let by_var = env
        .var(PORTABLE_VAR)
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    by_var
        || env
            .exe_dir()
            .is_some_and(|dir| dir.join(PORTABLE_MARKER).is_file())
}

fn portable_root(env: &PlatformEnv, kind: Kind) -> Result<PathBuf, Error> {
    let exe_dir = env.require_exe_dir()?;
    let leaf = match kind {
        Kind::Config => "config",
        Kind::Data => "data",
        Kind::Cache => "cache",
    };
    Ok(exe_dir.join(PORTABLE_DATA_DIR).join(leaf))
}

/// The platform convention (no overrides, not portable).
fn platform_root(env: &PlatformEnv, kind: Kind) -> Result<PathBuf, Error> {
    match env.os() {
        Os::Windows => {
            let (var, fallback) = match kind {
                Kind::Config => ("APPDATA", "Roaming"),
                Kind::Data | Kind::Cache => ("LOCALAPPDATA", "Local"),
            };
            let base = match env.var_path(var) {
                Some(path) => path,
                None => env.require_home()?.join("AppData").join(fallback),
            };
            let app = base.join(APP_DIR_NAME);
            Ok(match kind {
                Kind::Config => app,
                Kind::Data => app.join("data"),
                Kind::Cache => app.join("cache"),
            })
        }
        Os::MacOs => {
            let home = env.require_home()?;
            Ok(match kind {
                Kind::Config => mac_support(home).join("config"),
                Kind::Data => mac_support(home).join("data"),
                Kind::Cache => home.join("Library").join("Caches").join(APP_DIR_NAME),
            })
        }
        Os::Linux | Os::OtherUnix | Os::Other => {
            let (var, fallback) = match kind {
                Kind::Config => ("XDG_CONFIG_HOME", ".config"),
                Kind::Data => ("XDG_DATA_HOME", ".local/share"),
                Kind::Cache => ("XDG_CACHE_HOME", ".cache"),
            };
            // Relative XDG values are ignored, as the XDG base directory spec requires.
            let base = match env.var_os(var).filter(|value| env.os().is_absolute(value)) {
                Some(value) => PathBuf::from(value),
                None => env.require_home()?.join(fallback),
            };
            Ok(base.join(APP_DIR_NAME))
        }
    }
}

fn mac_support(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join(APP_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use crate::error::{ErrorKind, UserFacing as _};

    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn resolve(os: Os, home: Option<&str>, vars: &[(&str, &str)]) -> Result<AppDirs, Error> {
        AppDirs::resolve(&PlatformEnv::for_tests(os, home.map(p), vars, None))
    }

    fn roots(dirs: &AppDirs) -> (PathBuf, PathBuf, PathBuf) {
        (
            dirs.config_dir().to_path_buf(),
            dirs.data_dir().to_path_buf(),
            dirs.cache_dir().to_path_buf(),
        )
    }

    #[test]
    fn a_p5_windows() {
        let dirs = resolve(
            Os::Windows,
            Some("C:\\Users\\u"),
            &[
                ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
                ("LOCALAPPDATA", "D:\\Local"),
            ],
        )
        .expect("resolves");
        let local = p("D:\\Local").join("arma3sync-rs");
        assert_eq!(
            roots(&dirs),
            (
                p("C:\\Users\\u\\AppData\\Roaming").join("arma3sync-rs"),
                local.join("data"),
                local.join("cache"),
            )
        );
        assert!(!dirs.is_portable());

        // Blank variables fall back to the home directory.
        let dirs = resolve(
            Os::Windows,
            Some("C:\\Users\\u"),
            &[("APPDATA", " "), ("LOCALAPPDATA", "")],
        )
        .expect("resolves");
        let home = p("C:\\Users\\u");
        let local = home.join("AppData").join("Local").join("arma3sync-rs");
        assert_eq!(
            roots(&dirs),
            (
                home.join("AppData").join("Roaming").join("arma3sync-rs"),
                local.join("data"),
                local.join("cache"),
            )
        );

        // With both variables set the home directory is not needed.
        assert!(
            resolve(
                Os::Windows,
                None,
                &[("APPDATA", "A"), ("LOCALAPPDATA", "L")]
            )
            .is_ok()
        );
        assert!(matches!(
            resolve(Os::Windows, None, &[]),
            Err(Error::NoHomeDir)
        ));
    }

    #[test]
    fn a_p5_linux() {
        let xdg = [
            ("XDG_CONFIG_HOME", "/xdg/config"),
            ("XDG_DATA_HOME", "/xdg/data"),
            ("XDG_CACHE_HOME", "/xdg/cache"),
        ];
        let dirs = resolve(Os::Linux, Some("/home/u"), &xdg).expect("resolves");
        assert_eq!(
            roots(&dirs),
            (
                p("/xdg/config/arma3sync-rs"),
                p("/xdg/data/arma3sync-rs"),
                p("/xdg/cache/arma3sync-rs"),
            )
        );

        let fallback = (
            p("/home/u/.config/arma3sync-rs"),
            p("/home/u/.local/share/arma3sync-rs"),
            p("/home/u/.cache/arma3sync-rs"),
        );
        let dirs = resolve(Os::Linux, Some("/home/u"), &[]).expect("resolves");
        assert_eq!(roots(&dirs), fallback);

        // Blank and relative XDG values are ignored.
        let ignored = [
            ("XDG_CONFIG_HOME", "  "),
            ("XDG_DATA_HOME", "relative/data"),
            ("XDG_CACHE_HOME", "cache"),
        ];
        let dirs = resolve(Os::OtherUnix, Some("/home/u"), &ignored).expect("resolves");
        assert_eq!(roots(&dirs), fallback);

        assert!(matches!(
            resolve(Os::Linux, None, &[]),
            Err(Error::NoHomeDir)
        ));
        assert_eq!(
            Error::NoHomeDir.kind(),
            ErrorKind::InvalidInput,
            "NoHomeDir is a configuration problem"
        );
    }

    #[test]
    fn a_p5_macos() {
        let dirs = resolve(
            Os::MacOs,
            Some("/Users/u"),
            &[("XDG_CONFIG_HOME", "/xdg/config")],
        )
        .expect("resolves");
        assert_eq!(
            roots(&dirs),
            (
                p("/Users/u/Library/Application Support/arma3sync-rs/config"),
                p("/Users/u/Library/Application Support/arma3sync-rs/data"),
                p("/Users/u/Library/Caches/arma3sync-rs"),
            )
        );
        assert!(matches!(
            resolve(Os::MacOs, None, &[]),
            Err(Error::NoHomeDir)
        ));
    }

    #[test]
    fn a_p5_overrides_win_individually() {
        let dirs = resolve(
            Os::Linux,
            Some("/home/u"),
            &[("A3S_CONFIG_DIR", "/cfg"), ("A3S_CACHE_DIR", "rel/cache")],
        )
        .expect("resolves");
        assert_eq!(
            roots(&dirs),
            (
                p("/cfg"),
                p("/home/u/.local/share/arma3sync-rs"),
                p("rel/cache"),
            )
        );

        // All three overridden: no home directory needed.
        let dirs = resolve(
            Os::MacOs,
            None,
            &[
                ("A3S_CONFIG_DIR", "/c"),
                ("A3S_DATA_DIR", "/d"),
                ("A3S_CACHE_DIR", "/k"),
            ],
        )
        .expect("resolves");
        assert_eq!(roots(&dirs), (p("/c"), p("/d"), p("/k")));
    }

    #[test]
    fn a_p5_portable() {
        let exe = tempfile::tempdir().expect("tempdir");
        let exe_dir = exe.path().to_path_buf();
        let data = exe_dir.join("arma3sync-rs-data");
        let env = |vars: &[(&str, &str)]| {
            PlatformEnv::for_tests(Os::Linux, Some(p("/home/u")), vars, Some(exe_dir.clone()))
        };

        // A3S_PORTABLE=TRUE (case-insensitive).
        let dirs = AppDirs::resolve(&env(&[("A3S_PORTABLE", "TRUE")])).expect("resolves");
        assert!(dirs.is_portable());
        assert_eq!(
            roots(&dirs),
            (data.join("config"), data.join("data"), data.join("cache"))
        );

        // `1` is not `true` (Java `Boolean.parseBoolean`).
        let dirs = AppDirs::resolve(&env(&[("A3S_PORTABLE", "1")])).expect("resolves");
        assert!(!dirs.is_portable());
        assert_eq!(dirs.config_dir(), Path::new("/home/u/.config/arma3sync-rs"));

        // Marker file next to the executable; overrides still win.
        std::fs::write(exe_dir.join(PORTABLE_MARKER), "").expect("write marker");
        let dirs = AppDirs::resolve(&env(&[("A3S_DATA_DIR", "/d")])).expect("resolves");
        assert!(dirs.is_portable());
        assert_eq!(
            roots(&dirs),
            (data.join("config"), p("/d"), data.join("cache"))
        );

        // Portable without an executable directory cannot resolve.
        let no_exe = PlatformEnv::for_tests(Os::Linux, None, &[("A3S_PORTABLE", "true")], None);
        assert!(matches!(
            AppDirs::resolve(&no_exe),
            Err(Error::CurrentExe(_))
        ));
    }

    #[test]
    fn a_p5_derived_dirs() {
        let dirs = AppDirs::from_roots(p("/c"), p("/d"), p("/k"));
        assert_eq!(dirs.profiles_dir(), p("/c/profiles"));
        assert_eq!(dirs.repositories_dir(), p("/d/repositories"));
        assert_eq!(dirs.temp_dir(), p("/k/temp"));
        assert_eq!(dirs.log_dir(), p("/d/logs"));
        assert!(!dirs.is_portable());
    }

    #[test]
    fn a_p7_ensure_created() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let dirs = AppDirs::from_roots(root.join("c"), root.join("d"), root.join("k"));
        dirs.ensure_created().expect("created");
        for dir in dirs.all_dirs() {
            assert!(dir.is_dir(), "{}", dir.display());
        }
        // Idempotent.
        dirs.ensure_created().expect("created again");
    }

    #[test]
    fn a_p7_create_dir_error_message_and_kind() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // A regular file where a directory must go fails even when running as root.
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, "x").expect("write");
        let dirs = AppDirs::from_roots(blocker.join("c"), blocker.join("d"), blocker.join("k"));
        let err = dirs
            .ensure_created()
            .expect_err("cannot create below a file");
        let Error::CreateDir { path, .. } = &err else {
            panic!("unexpected error {err:?}");
        };
        assert_eq!(path, &blocker.join("c"));
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn a_p7_read_only_parent() {
        use std::os::unix::fs::PermissionsExt as _;

        let tmp = tempfile::tempdir().expect("tempdir");
        let parent = tmp.path().join("ro");
        std::fs::create_dir(&parent).expect("mkdir");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).expect("chmod");
        let dirs = AppDirs::from_roots(parent.join("c"), parent.join("d"), parent.join("k"));
        let result = dirs.ensure_created();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755))
            .expect("chmod back");
        // Root ignores permissions; nothing to check then.
        if let Err(err) = result {
            assert_eq!(err.kind(), ErrorKind::PermissionDenied);
        }
    }
}
