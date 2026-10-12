// Its own test binary: it re-executes itself filtered by the unprefixed test name.
//! Adversarial checks of the `paths` module: odd environment values, portable mode, and read-only
//! legacy Java discovery that never points into the Rust directories.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use a3s_foundation::paths::legacy::{
    LegacySource, discover_legacy_java, inspect_java_installation, java_per_user_roots,
};
use a3s_foundation::paths::{AppDirs, Error, Os, PORTABLE_MARKER, PlatformEnv};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn utf8(path: &Path) -> &str {
    path.to_str().expect("utf-8 temp path")
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, contents).expect("write");
}

/// Recursive listing: relative path → contents (`None` for directories, target for symlinks).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            let rel = path.strip_prefix(root).expect("inside").to_path_buf();
            let meta = fs::symlink_metadata(&path).expect("metadata");
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&path).expect("read_link");
                out.insert(rel, Some(target.as_os_str().as_encoded_bytes().to_vec()));
            } else if meta.is_dir() {
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
fn verify_portable_var_follows_java_parse_boolean() {
    let exe = tempfile::tempdir().expect("tempdir");
    let env = |value: &str| {
        PlatformEnv::for_tests(
            Os::Linux,
            Some(p("/home/u")),
            &[("A3S_PORTABLE", value)],
            Some(exe.path().to_path_buf()),
        )
    };
    for yes in ["true", "True", "tRuE"] {
        assert!(
            AppDirs::resolve(&env(yes)).expect("resolves").is_portable(),
            "{yes:?}"
        );
    }
    // Boolean.parseBoolean: no trimming, no `yes`/`1`/`on`.
    for no in [
        " true",
        "true ",
        "yes",
        "1",
        "on",
        "false",
        "trueX",
        "ｔｒｕｅ",
    ] {
        assert!(
            !AppDirs::resolve(&env(no)).expect("resolves").is_portable(),
            "{no:?}"
        );
    }
}

#[test]
fn verify_portable_marker_must_be_a_file() {
    let exe = tempfile::tempdir().expect("tempdir");
    fs::create_dir(exe.path().join(PORTABLE_MARKER)).expect("mkdir");
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(p("/home/u")),
        &[],
        Some(exe.path().to_path_buf()),
    );
    assert!(
        !AppDirs::resolve(&env).expect("resolves").is_portable(),
        "a directory named like the marker does not enable portable mode"
    );
    assert!(
        !exe.path().join("arma3sync-rs-data").exists(),
        "resolve must not create anything"
    );
}

#[test]
fn verify_xdg_relative_dot_and_tilde_are_ignored_for_rust_dirs() {
    let fallback = (
        p("/home/u/.config/arma3sync-rs"),
        p("/home/u/.local/share/arma3sync-rs"),
        p("/home/u/.cache/arma3sync-rs"),
    );
    for value in [".", "./x", "~/.config", "../x", " /abs-with-leading-space"] {
        let env = PlatformEnv::for_tests(
            Os::Linux,
            Some(p("/home/u")),
            &[
                ("XDG_CONFIG_HOME", value),
                ("XDG_DATA_HOME", value),
                ("XDG_CACHE_HOME", value),
            ],
            None,
        );
        let dirs = AppDirs::resolve(&env).expect("resolves");
        assert_eq!(
            (
                dirs.config_dir().to_path_buf(),
                dirs.data_dir().to_path_buf(),
                dirs.cache_dir().to_path_buf()
            ),
            fallback,
            "{value:?}"
        );
    }
}

#[test]
fn verify_relative_xdg_without_home_is_no_home_dir() {
    let env = PlatformEnv::for_tests(
        Os::Linux,
        None,
        &[
            ("XDG_CONFIG_HOME", "rel"),
            ("XDG_DATA_HOME", "/d"),
            ("XDG_CACHE_HOME", "/k"),
        ],
        None,
    );
    assert!(matches!(AppDirs::resolve(&env), Err(Error::NoHomeDir)));
    // Absolute XDG for all three: the home directory is not needed.
    let env = PlatformEnv::for_tests(
        Os::Linux,
        None,
        &[
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_DATA_HOME", "/d"),
            ("XDG_CACHE_HOME", "/k"),
        ],
        None,
    );
    let dirs = AppDirs::resolve(&env).expect("resolves");
    assert_eq!(dirs.config_dir(), Path::new("/c/arma3sync-rs"));
}

#[test]
fn verify_blank_overrides_fall_through() {
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(p("/home/u")),
        &[
            ("A3S_CONFIG_DIR", " "),
            ("A3S_DATA_DIR", "\t"),
            ("A3S_CACHE_DIR", ""),
        ],
        None,
    );
    let dirs = AppDirs::resolve(&env).expect("resolves");
    assert_eq!(dirs.config_dir(), Path::new("/home/u/.config/arma3sync-rs"));
    assert_eq!(
        dirs.data_dir(),
        Path::new("/home/u/.local/share/arma3sync-rs")
    );
    assert_eq!(dirs.cache_dir(), Path::new("/home/u/.cache/arma3sync-rs"));
}

/// Java `String.isBlank` uses `Character.isWhitespace`: U+001F is blank, U+00A0 is not.
// Mirrors dao/ApplicationPaths.java:207, 232
#[test]
fn verify_blank_follows_java_is_blank() {
    let roots = |value: &str| {
        java_per_user_roots(&PlatformEnv::for_tests(
            Os::Linux,
            Some(p("/home/u")),
            &[("XDG_CONFIG_HOME", value)],
            None,
        ))
        .expect("resolves")
        .config
    };
    assert_eq!(
        roots("\u{1F}"),
        p("/home/u/.config/Arma3Sync"),
        "U+001F is whitespace for Java isBlank"
    );
    assert_eq!(
        roots("\u{A0}"),
        p("\u{A0}").join("Arma3Sync"),
        "U+00A0 is not whitespace for Java isBlank"
    );
}

#[test]
fn verify_windows_legacy_roots_without_home() {
    let env = PlatformEnv::for_tests(
        Os::Windows,
        None,
        &[("APPDATA", "C:\\R"), ("LOCALAPPDATA", "C:\\L")],
        None,
    );
    let roots = java_per_user_roots(&env).expect("no home needed");
    assert_eq!(roots.config, p("C:\\R").join("Arma3Sync"));
    assert_eq!(roots.data, p("C:\\L").join("Arma3Sync"));
    assert_eq!(roots.cache, p("C:\\L").join("Arma3Sync"));
    // Only APPDATA: data/cache need the home fallback, which is missing.
    let env = PlatformEnv::for_tests(Os::Windows, None, &[("APPDATA", "C:\\R")], None);
    assert_eq!(java_per_user_roots(&env), None);
}

#[test]
fn verify_installation_items_must_have_the_right_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("install");
    // `a3s.cfg` is a directory, `profiles` and `resources/ftp` are files.
    fs::create_dir_all(dir.join("a3s.cfg")).expect("mkdir");
    write(&dir.join("profiles"), "not a dir");
    write(&dir.join("resources/ftp"), "not a dir");
    write(&dir.join("resources/configuration/a3s.prefs"), "prefs");
    let data = inspect_java_installation(&dir);
    assert_eq!(
        data.configuration_dir,
        dir.join("resources/configuration"),
        "a directory named a3s.cfg does not select the compact layout"
    );
    assert_eq!(data.configuration_file, None);
    assert_eq!(
        data.preferences_file,
        Some(dir.join("resources/configuration/a3s.prefs"))
    );
    assert_eq!(data.profiles_dir, None);
    assert_eq!(data.repositories_dir, None);
}

#[test]
fn verify_missing_installation_is_empty_and_not_created() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("does/not/exist");
    let data = inspect_java_installation(&dir);
    assert!(data.is_empty());
    assert_eq!(data.source, LegacySource::Installation(dir.clone()));
    assert!(!tmp.path().join("does").exists());
}

#[cfg(unix)]
#[test]
fn verify_symlinks_followed_and_dangling_ignored() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().expect("tempdir");
    let real = tmp.path().join("real");
    write(&real.join("cfg-target"), "cfg");
    fs::create_dir_all(real.join("profiles-target")).expect("mkdir");
    let dir = tmp.path().join("install");
    fs::create_dir_all(&dir).expect("mkdir");
    symlink(real.join("cfg-target"), dir.join("a3s.cfg")).expect("symlink");
    symlink(real.join("profiles-target"), dir.join("profiles")).expect("symlink");
    symlink(tmp.path().join("missing"), dir.join("a3s.prefs")).expect("symlink");
    fs::create_dir_all(dir.join("resources")).expect("mkdir");
    symlink(tmp.path().join("missing-dir"), dir.join("resources/ftp")).expect("symlink");

    let before = snapshot(tmp.path());
    let data = inspect_java_installation(&dir);
    assert_eq!(snapshot(tmp.path()), before, "inspection is read-only");
    assert_eq!(data.configuration_dir, dir);
    assert_eq!(data.configuration_file, Some(dir.join("a3s.cfg")));
    assert_eq!(data.preferences_file, None, "dangling symlink");
    assert_eq!(data.profiles_dir, Some(dir.join("profiles")));
    assert_eq!(data.repositories_dir, None, "dangling symlink");
}

/// L5: an install dir inside the Rust portable data folder is never returned.
#[test]
fn verify_discovery_never_returns_rust_portable_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let exe_dir = tmp.path().join("exe");
    write(&exe_dir.join(PORTABLE_MARKER), "");
    let rust_config = exe_dir.join("arma3sync-rs-data/config");
    write(&rust_config.join("a3s.cfg"), "rust-owned?");
    write(
        &exe_dir.join("arma3sync-rs-data/data/profiles/x.a3s.profile"),
        "p",
    );
    let env = PlatformEnv::for_tests(Os::Linux, None, &[], Some(exe_dir.clone()));
    let before = snapshot(tmp.path());
    let found = discover_legacy_java(
        &env,
        &[rust_config.clone(), exe_dir.join("arma3sync-rs-data/data")],
    );
    assert_eq!(snapshot(tmp.path()), before, "discovery is read-only");
    assert!(found.is_empty(), "{found:#?}");
}

/// L5 through a symlink: an install dir that resolves into the Rust config dir is skipped.
#[cfg(unix)]
#[test]
fn verify_discovery_skips_symlink_into_rust_dirs() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().expect("tempdir");
    let rust_config = tmp.path().join("xdg-config/arma3sync-rs");
    write(&rust_config.join("a3s.cfg"), "cfg");
    let link = tmp.path().join("link");
    symlink(&rust_config, &link).expect("symlink");
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(tmp.path().join("home")),
        &[("XDG_CONFIG_HOME", utf8(&tmp.path().join("xdg-config")))],
        None,
    );
    let found = discover_legacy_java(&env, &[link]);
    assert!(found.is_empty(), "{found:#?}");
}

/// L5: a Java install dir whose `resources/ftp` is a symlink into the Rust data dir keeps its
/// other items but loses that one.
#[cfg(unix)]
#[test]
fn verify_discovery_drops_items_resolving_into_rust_dirs() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().expect("tempdir");
    let rust_data = tmp.path().join("rs-data");
    fs::create_dir_all(rust_data.join("repositories")).expect("mkdir");
    let install = tmp.path().join("install");
    write(&install.join("profiles/Default.a3s.profile"), "p");
    fs::create_dir_all(install.join("resources")).expect("mkdir");
    symlink(
        rust_data.join("repositories"),
        install.join("resources/ftp"),
    )
    .expect("symlink");
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(tmp.path().join("home")),
        &[("A3S_DATA_DIR", utf8(&rust_data))],
        None,
    );
    let found = discover_legacy_java(&env, std::slice::from_ref(&install));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].profiles_dir, Some(install.join("profiles")));
    assert_eq!(found[0].repositories_dir, None);
}

#[test]
fn verify_discovery_without_home_still_inspects_extra_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = tmp.path().join("install");
    write(&install.join("a3s.prefs"), "prefs");
    let env = PlatformEnv::for_tests(Os::Linux, None, &[], None);
    let found = discover_legacy_java(&env, std::slice::from_ref(&install));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, LegacySource::Installation(install.clone()));
    assert_eq!(found[0].preferences_file, Some(install.join("a3s.prefs")));
}

#[test]
fn verify_windows_program_files_dedup_keeps_first_occurrence() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pf = tmp.path().join("PF");
    write(&pf.join("Arma3Sync/a3s.cfg"), "cfg");
    let env = PlatformEnv::for_tests(
        Os::Windows,
        Some(tmp.path().join("home")),
        &[
            ("ProgramFiles", utf8(&pf)),
            ("ProgramW6432", utf8(&pf)),
            ("ProgramFiles(x86)", "   "),
        ],
        None,
    );
    // The same folder as an explicit extra dir comes first and wins.
    let found = discover_legacy_java(&env, &[pf.join("Arma3Sync")]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        found[0].source,
        LegacySource::Installation(pf.join("Arma3Sync"))
    );
    // Non-Windows: ProgramFiles variables are ignored.
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(tmp.path().join("home")),
        &[("ProgramFiles", utf8(&pf))],
        None,
    );
    assert!(discover_legacy_java(&env, &[]).is_empty());
}

/// Rust and Java per-user roots never overlap, even when the XDG variables point at the
/// parent of a Java folder or at Java's own folder name.
#[test]
fn verify_no_collision_with_adversarial_xdg_values() {
    fn lower(path: &Path) -> Vec<String> {
        path.to_string_lossy()
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .map(str::to_lowercase)
            .collect()
    }
    let cases: &[(Os, &[(&str, &str)])] = &[
        (
            Os::Linux,
            &[
                ("XDG_CONFIG_HOME", "/same"),
                ("XDG_DATA_HOME", "/same"),
                ("XDG_CACHE_HOME", "/same"),
            ],
        ),
        (Os::Linux, &[("XDG_CONFIG_HOME", "/home/u/ARMA3SYNC")]),
        (Os::MacOs, &[("XDG_CONFIG_HOME", "/Users/u/Library/Caches")]),
        (
            Os::MacOs,
            &[("XDG_DATA_HOME", "/Users/u/Library/Application Support")],
        ),
        (
            Os::Windows,
            &[
                ("APPDATA", "C:\\Users\\u\\AppData\\Local"),
                ("LOCALAPPDATA", "c:\\users\\u\\appdata\\local"),
            ],
        ),
        (Os::OtherUnix, &[("XDG_CACHE_HOME", "/home/u/.arma3sync")]),
        (Os::Other, &[("XDG_CONFIG_HOME", "/home/u/.arma3sync")]),
    ];
    for &(os, vars) in cases {
        let home = if os == Os::Windows {
            "C:\\Users\\u"
        } else if os == Os::MacOs {
            "/Users/u"
        } else {
            "/home/u"
        };
        let env = PlatformEnv::for_tests(os, Some(p(home)), vars, None);
        let rust = AppDirs::resolve(&env).expect("resolves");
        let java = java_per_user_roots(&env).expect("resolves");
        for rust_dir in [rust.config_dir(), rust.data_dir(), rust.cache_dir()] {
            for java_dir in [&java.config, &java.data, &java.cache] {
                let (r, j) = (lower(rust_dir), lower(java_dir));
                assert!(
                    !r.starts_with(&j) && !j.starts_with(&r),
                    "{os:?} {vars:?}: {} vs {}",
                    rust_dir.display(),
                    java_dir.display()
                );
            }
        }
    }
}

// ---- Real process environment (child processes; no `set_var`) ----

const CHILD_VAR: &str = "A3S_VERIFY_PATHS_CHILD";

/// Runs `verify_child_entry` in a child process with `envs` replacing the environment.
fn run_child(envs: &[(&str, &std::ffi::OsStr)]) -> String {
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .args([
            "verify_child_entry",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env(CHILD_VAR, "1");
    for (name, value) in envs {
        command.env(name, value);
    }
    let output = command.output().expect("spawn child");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
        .lines()
        .find_map(|line| line.split_once("RESULT ").map(|(_, result)| result))
        .expect("child result line")
        .to_owned()
}

#[test]
fn verify_child_entry() {
    if std::env::var_os(CHILD_VAR).is_none() {
        return;
    }
    let env = PlatformEnv::current().expect("current env");
    let result = match AppDirs::resolve(&env) {
        Ok(dirs) => format!(
            "ok portable={} config={:?} data={:?} cache={:?}",
            dirs.is_portable(),
            dirs.config_dir(),
            dirs.data_dir(),
            dirs.cache_dir()
        ),
        Err(err) => format!("err {err:?}"),
    };
    println!("RESULT {result}");
}

#[test]
fn verify_current_env_empty_home_is_no_home_dir() {
    let empty = std::ffi::OsStr::new("");
    let result = run_child(&[("HOME", empty)]);
    assert_eq!(result, "err NoHomeDir");
    let result = run_child(&[]);
    assert_eq!(result, "err NoHomeDir");
}

#[cfg(target_os = "linux")]
#[test]
fn verify_current_env_relative_and_blank_values() {
    use std::ffi::OsStr;

    let result = run_child(&[
        ("HOME", OsStr::new("/h")),
        ("XDG_CONFIG_HOME", OsStr::new("relative")),
        ("XDG_DATA_HOME", OsStr::new("  ")),
        ("XDG_CACHE_HOME", OsStr::new("/abs/cache")),
        ("A3S_PORTABLE", OsStr::new("1")),
    ]);
    assert_eq!(
        result,
        "ok portable=false config=\"/h/.config/arma3sync-rs\" data=\"/h/.local/share/arma3sync-rs\" cache=\"/abs/cache/arma3sync-rs\""
    );
}

#[cfg(target_os = "linux")]
#[test]
fn verify_current_env_non_utf8_values() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    // Non-UTF-8 XDG and override values are valid paths on Unix and must be kept byte-exact.
    let xdg = OsStr::from_bytes(b"/x\xff/cfg");
    let data = OsStr::from_bytes(b"/d\xfe");
    let portable = OsStr::from_bytes(b"true\xe9");
    let result = run_child(&[
        ("HOME", OsStr::new("/h")),
        ("XDG_CONFIG_HOME", xdg),
        ("A3S_DATA_DIR", data),
        ("A3S_PORTABLE", portable),
    ]);
    assert_eq!(
        result,
        "ok portable=false config=\"/x\\xFF/cfg/arma3sync-rs\" data=\"/d\\xFE\" cache=\"/h/.cache/arma3sync-rs\""
    );
}

/// A directory that cannot be created is reported with its absolute path, also when it was
/// configured relative to the working directory (the crate folder under `cargo test`).
#[test]
fn verify_create_dir_error_names_an_absolute_path() {
    let relative = p("Cargo.toml/below-a-file");
    let dirs = AppDirs::from_roots(relative.clone(), relative.clone(), relative.clone());
    match dirs.ensure_created() {
        Err(Error::CreateDir { path, .. }) => {
            assert!(path.is_absolute(), "{}", path.display());
            assert!(path.ends_with(&relative), "{}", path.display());
        }
        other => panic!("expected CreateDir, got {other:?}"),
    }
}

/// Two spellings of one installation, through a symbolic link, are one location, even when its
/// configuration folder does not exist.
#[cfg(unix)]
#[test]
fn verify_discovery_dedups_missing_configuration_dir_through_symlink() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().expect("tempdir");
    let install = tmp.path().join("real/install");
    write(&install.join("profiles/Default.a3s.profile"), "p");
    let alias = tmp.path().join("alias");
    symlink(tmp.path().join("real"), &alias).expect("symlink");
    let env = PlatformEnv::for_tests(Os::Linux, None, &[], None);
    let found = discover_legacy_java(&env, &[install, alias.join("install")]);
    assert_eq!(found.len(), 1, "{found:#?}");
}
