//! Read-only discovery of the Java client's data, for a one-time import.
//!
//! Nothing here creates, writes, renames or deletes anything. Java's copy-on-start
//! `migrateLegacyData` is not ported; the importer reads the legacy files in place.

use std::path::{Component, Path, PathBuf};

use crate::paths::{AppDirs, JAVA_APP_NAME, Os, PlatformEnv};

/// Java configuration file name.
pub const CONFIGURATION_FILE_NAME: &str = "a3s.cfg";

/// Java preferences file name.
pub const PREFERENCES_FILE_NAME: &str = "a3s.prefs";

/// Java workshop metadata cache file name.
// Mirrors service/WorkshopMetadataStore.java:28
pub const WORKSHOP_METADATA_FILE_NAME: &str = "known-workshop-mods.properties";

/// Where a set of Java data was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacySource {
    /// The per-user layout (`APPDATA` / XDG folders + `Arma3Sync`).
    PerUser,

    /// An installation folder (portable or pre-modern layout).
    Installation(PathBuf),
}

/// The Java client's per-user roots, each already joined with `Arma3Sync`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaRoots {
    /// Java `configRoot()`.
    pub config: PathBuf,

    /// Java `dataRoot()`.
    pub data: PathBuf,

    /// Java `cacheRoot()`.
    pub cache: PathBuf,
}

/// Java data found in one location. Item paths are `Some` only when the item exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyJavaData {
    /// Where the data was found.
    pub source: LegacySource,

    /// The configuration folder Java would use (exists or not).
    pub configuration_dir: PathBuf,

    /// `a3s.cfg`, if it is a regular file.
    pub configuration_file: Option<PathBuf>,

    /// `a3s.prefs`, if it is a regular file.
    pub preferences_file: Option<PathBuf>,

    /// `known-workshop-mods.properties`, if it is a regular file.
    pub workshop_metadata_file: Option<PathBuf>,

    /// Profiles folder, if it is a directory.
    pub profiles_dir: Option<PathBuf>,

    /// Repositories folder, if it is a directory.
    pub repositories_dir: Option<PathBuf>,
}

impl LegacyJavaData {
    /// Whether no item was found.
    pub fn is_empty(&self) -> bool {
        self.configuration_file.is_none()
            && self.preferences_file.is_none()
            && self.workshop_metadata_file.is_none()
            && self.profiles_dir.is_none()
            && self.repositories_dir.is_none()
    }

    fn probe(
        source: LegacySource,
        configuration_dir: PathBuf,
        profiles_dir: &Path,
        repositories_dir: &Path,
    ) -> Self {
        Self {
            source,
            configuration_file: existing_file(configuration_dir.join(CONFIGURATION_FILE_NAME)),
            preferences_file: existing_file(configuration_dir.join(PREFERENCES_FILE_NAME)),
            workshop_metadata_file: existing_file(
                configuration_dir.join(WORKSHOP_METADATA_FILE_NAME),
            ),
            configuration_dir,
            profiles_dir: existing_dir(profiles_dir.to_path_buf()),
            repositories_dir: existing_dir(repositories_dir.to_path_buf()),
        }
    }
}

/// The Java client's per-user roots for `env`, without touching the file system.
///
/// `None` when a needed home directory is unknown (Java fell back to `.`).
// Mirrors dao/ApplicationPaths.java:197-239 (`platformRoot`) and :82-101
pub fn java_per_user_roots(env: &PlatformEnv) -> Option<JavaRoots> {
    let root = |kind: &str| -> Option<PathBuf> {
        let base = match env.os() {
            Os::Windows => {
                let (var, fallback) = if kind == "config" {
                    ("APPDATA", "Roaming")
                } else {
                    ("LOCALAPPDATA", "Local")
                };
                match env.var_path(var) {
                    Some(path) => path,
                    None => env.home()?.join("AppData").join(fallback),
                }
            }
            // Quirk: Java uses XDG paths on macOS too, and accepts relative values.
            Os::Linux | Os::MacOs | Os::OtherUnix => {
                let (var, fallback) = match kind {
                    "config" => ("XDG_CONFIG_HOME", ".config"),
                    "cache" => ("XDG_CACHE_HOME", ".cache"),
                    _ => ("XDG_DATA_HOME", ".local/share"),
                };
                match env.var_path(var) {
                    Some(path) => path,
                    None => env.home()?.join(fallback),
                }
            }
            Os::Other => env.home()?.join(".arma3sync"),
        };
        Some(base.join(JAVA_APP_NAME))
    };
    Some(JavaRoots {
        config: root("config")?,
        data: root("data")?,
        cache: root("cache")?,
    })
}

/// Inspects the Java per-user layout: `config/configuration/{a3s.cfg,a3s.prefs,…}`,
/// `config/profiles` and `data/repositories`.
// Mirrors dao/ApplicationPaths.java:103-137 (non-portable branches)
pub fn inspect_java_per_user(env: &PlatformEnv) -> Option<LegacyJavaData> {
    let roots = java_per_user_roots(env)?;
    Some(LegacyJavaData::probe(
        LegacySource::PerUser,
        roots.config.join("configuration"),
        &roots.config.join("profiles"),
        &roots.data.join("repositories"),
    ))
}

/// Inspects a Java installation folder (portable or pre-modern layout).
///
/// The configuration folder is the installation folder itself when `a3s.cfg` **or**
/// `a3s.prefs` is a regular file there (so with only `a3s.prefs` present, Java also looks for
/// `a3s.cfg` there), else `resources/configuration`. Profiles are in `profiles`, repositories
/// in `resources/ftp`.
// Mirrors dao/ApplicationPaths.java:250-257, 117-120, 131-135
pub fn inspect_java_installation(dir: &Path) -> LegacyJavaData {
    let compact =
        dir.join(CONFIGURATION_FILE_NAME).is_file() || dir.join(PREFERENCES_FILE_NAME).is_file();
    let configuration_dir = if compact {
        dir.to_path_buf()
    } else {
        dir.join("resources").join("configuration")
    };
    LegacyJavaData::probe(
        LegacySource::Installation(dir.to_path_buf()),
        configuration_dir,
        &dir.join("profiles"),
        &dir.join("resources").join("ftp"),
    )
}

/// Finds every location with Java data.
///
/// The order is: the per-user layout, each of `extra_install_dirs` (registry `InstallDir`, user-picked folders), and on Windows the default
/// `%ProgramFiles%\Arma3Sync`, `%ProgramFiles(x86)%\Arma3Sync`, `%ProgramW6432%\Arma3Sync`.
///
/// Empty results and duplicates (same normalized configuration folder) are dropped, as are
/// locations inside the Rust client's own directories.
pub fn discover_legacy_java(
    env: &PlatformEnv,
    extra_install_dirs: &[PathBuf],
) -> Vec<LegacyJavaData> {
    let mut candidates: Vec<LegacyJavaData> = Vec::new();
    candidates.extend(inspect_java_per_user(env));
    candidates.extend(
        extra_install_dirs
            .iter()
            .map(|dir| inspect_java_installation(dir)),
    );
    if env.os() == Os::Windows {
        // NSIS installs to `$PROGRAMFILES64\Arma3Sync` (installer/nsis/Arma3Sync.nsi:33).
        // Mirrors dao/ApplicationPaths.java:274-278 (variable names)
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(root) = env.var_path(var) {
                candidates.push(inspect_java_installation(&root.join(JAVA_APP_NAME)));
            }
        }
    }

    let rust_dirs: Vec<PathBuf> = AppDirs::resolve(env)
        .map(|dirs| {
            [dirs.config_dir(), dirs.data_dir(), dirs.cache_dir()]
                .into_iter()
                .map(normalize)
                .collect()
        })
        .unwrap_or_default();
    let inside_rust_dirs =
        |path: &Path| -> bool { rust_dirs.iter().any(|dir| normalize(path).starts_with(dir)) };

    let mut seen: Vec<PathBuf> = Vec::new();
    let mut found = Vec::new();
    for mut candidate in candidates {
        if inside_rust_dirs(&candidate.configuration_dir) {
            continue;
        }
        for item in [&mut candidate.profiles_dir, &mut candidate.repositories_dir] {
            if item.as_deref().is_some_and(inside_rust_dirs) {
                *item = None;
            }
        }
        if candidate.is_empty() {
            continue;
        }
        let key = normalize(&candidate.configuration_dir);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        found.push(candidate);
    }
    found
}

fn existing_file(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

/// Comparable form of `path`: lexically normalized (`.` removed, `..` applied), then its longest
/// existing ancestor replaced by its canonical path. Existing and missing paths thus compare
/// alike, also when they are spelled through different symbolic links.
fn normalize(path: &Path) -> PathBuf {
    let mut lexical = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !lexical.pop() {
                    lexical.push(component);
                }
            }
            other => lexical.push(other),
        }
    }
    let mut missing = Vec::new();
    let mut ancestor = lexical.as_path();
    loop {
        if let Ok(mut canonical) = std::fs::canonicalize(ancestor) {
            canonical.extend(missing.iter().rev());
            return canonical;
        }
        match (ancestor.parent(), ancestor.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                ancestor = parent;
            }
            _ => return lexical,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
        fs::write(path, contents).expect("write");
    }

    /// Recursive listing of a tree: relative path → file contents (`None` for directories).
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir") {
                let path = entry.expect("entry").path();
                let rel = path.strip_prefix(root).expect("inside root").to_path_buf();
                if path.is_dir() {
                    out.insert(rel, None);
                    stack.push(path);
                } else {
                    out.insert(rel, Some(fs::read(&path).expect("read")));
                }
            }
        }
        out
    }

    #[test]
    fn a_p1_discovery_is_read_only() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let installation = root.join("installation");
        write(&installation.join("profiles/Default.a3s.profile"), "legacy");
        write(
            &installation.join("resources/ftp/repository.a3s.repository"),
            "repository",
        );
        let config_home = root.join("config");
        write(
            &config_home.join("Arma3Sync/profiles/Default.a3s.profile"),
            "current",
        );

        let env = PlatformEnv::for_tests(
            Os::Linux,
            Some(root.join("home")),
            &[
                ("XDG_CONFIG_HOME", config_home.to_str().expect("utf-8")),
                ("XDG_DATA_HOME", root.join("data").to_str().expect("utf-8")),
                (
                    "XDG_CACHE_HOME",
                    root.join("cache").to_str().expect("utf-8"),
                ),
            ],
            None,
        );
        let before = snapshot(root);
        let found = discover_legacy_java(&env, std::slice::from_ref(&installation));
        assert_eq!(
            snapshot(root),
            before,
            "discovery must not touch the file system"
        );
        assert!(!root.join("config/arma3sync-rs").exists());
        assert!(!root.join("data/arma3sync-rs").exists());
        assert!(!root.join("cache/arma3sync-rs").exists());

        assert_eq!(found.len(), 2, "{found:#?}");
        assert_eq!(found[0].source, LegacySource::PerUser);
        assert_eq!(
            found[0].profiles_dir,
            Some(config_home.join("Arma3Sync/profiles"))
        );
        assert_eq!(found[0].repositories_dir, None);
        assert_eq!(
            found[1].source,
            LegacySource::Installation(installation.clone())
        );
        assert_eq!(found[1].profiles_dir, Some(installation.join("profiles")));
        assert_eq!(
            found[1].repositories_dir,
            Some(installation.join("resources/ftp"))
        );
        assert_eq!(found[1].configuration_file, None);
    }

    #[test]
    fn a_p2_compact_portable_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("compact-installation");
        fs::create_dir_all(dir.join("resources/ftp")).expect("mkdir");
        fs::write(dir.join("a3s.cfg"), "configuration").expect("write");
        fs::write(dir.join("a3s.prefs"), "preferences").expect("write");

        let data = inspect_java_installation(&dir);
        assert_eq!(data.configuration_dir, dir);
        assert_eq!(data.configuration_file, Some(dir.join("a3s.cfg")));
        assert_eq!(data.preferences_file, Some(dir.join("a3s.prefs")));
        assert_eq!(data.repositories_dir, Some(dir.join("resources/ftp")));
        assert_eq!(data.profiles_dir, None);
    }

    #[test]
    fn a_p3_resources_configuration_and_prefs_only_quirk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("install");
        let nested_cfg = dir.join("resources/configuration/a3s.cfg");
        write(&nested_cfg, "cfg");
        write(
            &dir.join("resources/configuration/known-workshop-mods.properties"),
            "mod.1=x",
        );

        let data = inspect_java_installation(&dir);
        assert_eq!(data.configuration_dir, dir.join("resources/configuration"));
        assert_eq!(data.configuration_file, Some(nested_cfg.clone()));
        assert_eq!(data.preferences_file, None);
        assert_eq!(
            data.workshop_metadata_file,
            Some(dir.join("resources/configuration/known-workshop-mods.properties"))
        );

        // Only `a3s.prefs` at the root: Java switches the whole configuration folder to the root
        // and ignores `resources/configuration/a3s.cfg`.
        write(&dir.join("a3s.prefs"), "prefs");
        let data = inspect_java_installation(&dir);
        assert_eq!(data.configuration_dir, dir);
        assert_eq!(data.configuration_file, None);
        assert_eq!(data.preferences_file, Some(dir.join("a3s.prefs")));
    }

    #[test]
    fn a_p4_java_per_user_roots() {
        fn roots(os: Os, home: Option<&str>, vars: &[(&str, &str)]) -> Option<JavaRoots> {
            java_per_user_roots(&PlatformEnv::for_tests(os, home.map(p), vars, None))
        }
        #[expect(clippy::unnecessary_wraps, reason = "compared with `Option` results")]
        fn expect(config: &Path, data: &Path, cache: &Path) -> Option<JavaRoots> {
            Some(JavaRoots {
                config: config.join("Arma3Sync"),
                data: data.join("Arma3Sync"),
                cache: cache.join("Arma3Sync"),
            })
        }

        // Windows: env set, data and cache share LOCALAPPDATA.
        assert_eq!(
            roots(
                Os::Windows,
                Some("C:\\Users\\u"),
                &[("APPDATA", "R:\\Roaming"), ("LOCALAPPDATA", "L:\\Local")]
            ),
            expect(&p("R:\\Roaming"), &p("L:\\Local"), &p("L:\\Local"))
        );
        // Windows: blank env → home fallback.
        let home = p("C:\\Users\\u");
        assert_eq!(
            roots(
                Os::Windows,
                Some("C:\\Users\\u"),
                &[("APPDATA", " "), ("LOCALAPPDATA", "")]
            ),
            expect(
                &home.join("AppData").join("Roaming"),
                &home.join("AppData").join("Local"),
                &home.join("AppData").join("Local"),
            )
        );

        // Linux: XDG set (relative accepted as-is), unset, blank.
        assert_eq!(
            roots(
                Os::Linux,
                Some("/home/u"),
                &[
                    ("XDG_CONFIG_HOME", "/x/c"),
                    ("XDG_DATA_HOME", "rel/d"),
                    ("XDG_CACHE_HOME", "/x/k"),
                ]
            ),
            expect(&p("/x/c"), &p("rel/d"), &p("/x/k"))
        );
        let fallback = expect(
            &p("/home/u/.config"),
            &p("/home/u/.local/share"),
            &p("/home/u/.cache"),
        );
        assert_eq!(roots(Os::Linux, Some("/home/u"), &[]), fallback);
        assert_eq!(
            roots(Os::Linux, Some("/home/u"), &[("XDG_CONFIG_HOME", "  ")]),
            fallback
        );

        // macOS quirk: XDG paths.
        assert_eq!(
            roots(Os::MacOs, Some("/home/u"), &[]),
            fallback,
            "Java uses XDG paths on macOS"
        );
        assert_eq!(roots(Os::OtherUnix, Some("/home/u"), &[]), fallback);

        // Other: ~/.arma3sync/Arma3Sync.
        let other = p("/home/u/.arma3sync");
        assert_eq!(
            roots(Os::Other, Some("/home/u"), &[]),
            expect(&other, &other, &other)
        );

        // No home and nothing set → None.
        assert_eq!(roots(Os::Linux, None, &[]), None);
    }

    /// Lower-cased components, for case-insensitive comparisons.
    fn lower(path: &Path) -> Vec<String> {
        path.to_string_lossy()
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    fn related(a: &Path, b: &Path) -> bool {
        let (a, b) = (lower(a), lower(b));
        a.starts_with(&b) || b.starts_with(&a)
    }

    #[test]
    fn a_p6_rust_dirs_never_collide_with_java_dirs() {
        type Case<'a> = (Os, &'a str, &'a [(&'a str, &'a str)]);
        let cases: &[Case<'_>] = &[
            (
                Os::Windows,
                "C:\\Users\\u",
                &[
                    ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
                    ("LOCALAPPDATA", "C:\\L"),
                ],
            ),
            (Os::Windows, "C:\\Users\\u", &[]),
            (Os::Linux, "/home/u", &[]),
            (
                Os::Linux,
                "/home/u",
                &[
                    ("XDG_CONFIG_HOME", "/x/c"),
                    ("XDG_DATA_HOME", "/x/d"),
                    ("XDG_CACHE_HOME", "/x/k"),
                ],
            ),
            (Os::MacOs, "/Users/u", &[]),
            (Os::MacOs, "/Users/u", &[("XDG_CONFIG_HOME", "/x/c")]),
            (Os::OtherUnix, "/home/u", &[]),
            (Os::Other, "/home/u", &[]),
        ];
        for &(os, home, vars) in cases {
            let env = PlatformEnv::for_tests(os, Some(p(home)), vars, None);
            let rust = AppDirs::resolve(&env).expect("resolves");
            let java = java_per_user_roots(&env).expect("resolves");
            for rust_dir in [rust.config_dir(), rust.data_dir(), rust.cache_dir()] {
                for java_dir in [&java.config, &java.data, &java.cache] {
                    assert!(
                        !related(rust_dir, java_dir),
                        "{os:?}: {} vs {}",
                        rust_dir.display(),
                        java_dir.display()
                    );
                }
            }
        }
    }

    #[test]
    fn a_p8_dedup_order_and_windows_defaults() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let appdata = root.join("Roaming");
        let per_user_cfg = appdata.join("Arma3Sync/configuration");
        write(&per_user_cfg.join("a3s.cfg"), "cfg");
        let program_files = root.join("PF");
        write(
            &program_files.join("Arma3Sync/resources/configuration/a3s.prefs"),
            "p",
        );
        let program_files_x86 = root.join("PF86");
        fs::create_dir_all(program_files_x86.join("Arma3Sync")).expect("mkdir");
        let first = root.join("first");
        write(&first.join("a3s.cfg"), "cfg");
        let second = root.join("second");
        fs::create_dir_all(second.join("profiles")).expect("mkdir");
        let empty = root.join("empty");
        fs::create_dir_all(&empty).expect("mkdir");

        let env = PlatformEnv::for_tests(
            Os::Windows,
            Some(root.join("home")),
            &[
                ("APPDATA", appdata.to_str().expect("utf-8")),
                ("LOCALAPPDATA", root.join("Local").to_str().expect("utf-8")),
                ("ProgramFiles", program_files.to_str().expect("utf-8")),
                (
                    "ProgramFiles(x86)",
                    program_files_x86.to_str().expect("utf-8"),
                ),
                // Same folder again through a different spelling.
                (
                    "ProgramW6432",
                    program_files.join(".").to_str().expect("utf-8"),
                ),
            ],
            None,
        );
        let extra = [
            second.clone(),
            empty,
            // Equals the per-user configuration folder.
            per_user_cfg.clone(),
            first.clone(),
            second.join("profiles/.."),
        ];
        let found = discover_legacy_java(&env, &extra);
        let sources: Vec<_> = found.iter().map(|data| data.source.clone()).collect();
        assert_eq!(
            sources,
            vec![
                LegacySource::PerUser,
                LegacySource::Installation(second),
                LegacySource::Installation(first),
                LegacySource::Installation(program_files.join("Arma3Sync")),
            ]
        );
        assert_eq!(found[0].configuration_dir, per_user_cfg);
    }

    #[test]
    fn discovery_skips_rust_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let rust_config = root.join("rs-config");
        write(&rust_config.join("a3s.cfg"), "cfg");
        let env = PlatformEnv::for_tests(
            Os::Linux,
            Some(root.join("home")),
            &[("A3S_CONFIG_DIR", rust_config.to_str().expect("utf-8"))],
            None,
        );
        assert!(discover_legacy_java(&env, &[rust_config]).is_empty());
    }
}
