//! Verifier tests for `local`: naming edge cases (RepositoryDAO.java:313-363), TOML round
//! trips of hostile values, file modes and atomic-write failures, duplicate resolution.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use a3s_repo_files::hash::{FileAttributes, Sha1Cache};
use a3s_repo_files::legacy::naming::{
    java_legacy_repository_file_name, java_preferred_repository_file_name,
};
use a3s_repo_files::local::{CacheKind, LocalDirs, LocalStore, file_stem, name_hash, sanitize};
use a3s_types::local::{Profile, Repository, TreeDirectory, TreeLeaf, TreeNode};
use a3s_types::{Protocol, ProtocolType};

fn store(root: &Path) -> LocalStore {
    LocalStore::with_dirs(LocalDirs {
        config: root.join("config"),
        profiles: root.join("config/profiles"),
        repositories: root.join("data/repositories"),
        sha1_cache: root.join("data/sha1_cache"),
    })
}

fn set_mtime(path: &Path, secs: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn repository(name: &str) -> Repository {
    Repository::new(
        name,
        Protocol::new(
            ProtocolType::Http,
            "repo.example/x",
            "80",
            "anonymous",
            "",
            true,
        ),
    )
}

/// Strings TOML serializers commonly get wrong.
const HOSTILE: &[&str] = &[
    "",
    " ",
    "\n",
    "\nleading newline",
    "trailing newline\n",
    "crlf\r\nline",
    "lone\rcr",
    "trailing cr\r",
    "'''triple single'''",
    "\"\"\"triple double\"\"\"",
    "back\\slash\\",
    "C:\\Users\\Arma\\@mods",
    "tab\there",
    "nul\u{0}byte",
    "del\u{7F}",
    "bom\u{FEFF}",
    "émoji 🎮 ünïcödé 日本語",
    "\u{2028}line sep\u{2029}",
    "key = \"value\"\n[table]\n",
];

// ---------------------------------------------------------------------------------------------
// Naming

#[test]
fn verify_whitespace_only_names_hash_the_untrimmed_name() {
    for name in ["   ", "\t", " \n ", "\u{1}", ""] {
        let stem = file_stem(name);
        assert_eq!(stem, format!("repository__{}", name_hash(name)), "{name:?}");
        assert_eq!(name_hash(name).len(), 12);
    }
    // Different whitespace-only names do not collide.
    assert_ne!(file_stem("   "), file_stem("  "));
    assert_ne!(file_stem("   "), file_stem(""));
    // Java helper agrees for whitespace only names.
    assert_eq!(
        java_preferred_repository_file_name(Some("   ")),
        format!("repository__{}.a3s.repository", name_hash("   "))
    );
}

#[test]
fn verify_slashes_and_reserved_chars() {
    assert_eq!(sanitize("a/b\\c"), "a_b_c");
    assert_eq!(sanitize("<>:\"/\\|?*"), "_________");
    assert_eq!(sanitize("../../etc/passwd"), ".._.._etc_passwd");
    // No path separator survives in a stem.
    for name in ["a/b", "a\\b", "/", "\\", "..", "../x", "C:\\x", "a:b"] {
        let stem = file_stem(name);
        assert!(
            !stem.contains('/') && !stem.contains('\\') && !stem.contains(':'),
            "{stem}"
        );
        assert!(stem.ends_with(&name_hash(name)));
    }
    // Windows reserved device names never form the whole file name (the hash suffix follows).
    for name in ["CON", "nul", "COM1", "LPT9", "AUX", "PRN"] {
        let stem = file_stem(name);
        assert_eq!(stem, format!("{name}__{}", name_hash(name)));
    }
    // Trailing dots or spaces are not at the end of the file name either.
    assert!(file_stem("name. ").starts_with("name.__"));
}

#[test]
fn verify_java_whitespace_rules() {
    // Java `\s` = [ \t\n\x0B\f\r]; runs collapse to one `_`.
    assert_eq!(sanitize("a\u{0B}\u{0C} \r\nb"), "a_b");
    // Java `trim` strips every char <= U+0020 at both ends, including controls.
    assert_eq!(sanitize("\u{1}\u{1F} a \u{0}"), "a");
    // Non-ASCII whitespace is neither trimmed nor collapsed by Java.
    assert_eq!(sanitize("\u{A0}x\u{A0}"), "\u{A0}x\u{A0}");
    assert_eq!(sanitize("a\u{3000}b"), "a\u{3000}b");
    // Inner control chars: Java keeps them, the Rust store replaces them.
    let name = "a\u{1C}b";
    assert_eq!(
        java_preferred_repository_file_name(Some(name)),
        format!("a\u{1C}b__{}.a3s.repository", name_hash(name))
    );
    assert_eq!(sanitize(name), "a_b");
    // A whitespace run next to a reserved char: Java replaces in two passes.
    assert_eq!(sanitize("a \\ b"), "a___b");
}

#[test]
fn verify_rust_and_java_stems_agree_for_plain_names() {
    for name in [
        "WOG",
        "My Repo",
        "Sample HTTP",
        "  a/b:c*?  ",
        "Événement été",
        "x",
    ] {
        assert_eq!(
            format!("{}.a3s.repository", file_stem(name)),
            java_preferred_repository_file_name(Some(name)),
            "{name:?}"
        );
    }
    assert_eq!(
        java_legacy_repository_file_name(Some("a\tb  c")),
        "a\tbc.a3s.repository"
    );
    assert_eq!(
        java_legacy_repository_file_name(None),
        "repository.a3s.repository"
    );
}

#[test]
fn verify_truncation_on_char_boundary() {
    for unit in ["é", "€", "🎮", "a", "日"] {
        let name = unit.repeat(200);
        let sanitized = sanitize(&name);
        assert!(sanitized.len() <= 64, "{unit}: {}", sanitized.len());
        assert!(
            sanitized.len() > 64 - unit.len(),
            "{unit}: {}",
            sanitized.len()
        );
        assert!(name.starts_with(&sanitized));
        let stem = file_stem(&name);
        assert_eq!(stem, format!("{sanitized}__{}", name_hash(&name)));
        // A file system with 255-byte names accepts it with the longest suffix.
        assert!(stem.len() + ".build.toml".len() <= 255);
    }
    // Names that differ only after the cut point still differ.
    let a = format!("{}A", "x".repeat(100));
    let b = format!("{}B", "x".repeat(100));
    assert_eq!(sanitize(&a), sanitize(&b));
    assert_ne!(file_stem(&a), file_stem(&b));
}

#[test]
fn verify_names_differing_in_case_do_not_collide() {
    let lower_a = file_stem("wog").to_lowercase();
    let lower_b = file_stem("WOG").to_lowercase();
    assert_ne!(lower_a, lower_b);

    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut upper = repository("WOG");
    upper.revision = 1;
    let mut lower = repository("wog");
    lower.revision = 2;
    store.write_repository(&upper).unwrap();
    store.write_repository(&lower).unwrap();
    let load = store.load_repositories().unwrap();
    assert!(load.failures.is_empty());
    assert_eq!(load.repositories["WOG"].repository.revision, 1);
    assert_eq!(load.repositories["wog"].repository.revision, 2);
}

// ---------------------------------------------------------------------------------------------
// TOML round trips

fn deep_tree(depth: usize) -> TreeDirectory {
    let mut node = TreeDirectory::new("leafdir");
    node.children.push(TreeNode::Leaf(TreeLeaf::new("@bottom")));
    for i in 0..depth {
        let mut parent = TreeDirectory::new(format!("level {i}"));
        parent.children.push(TreeNode::Directory(node));
        node = parent;
    }
    node
}

#[test]
fn verify_profile_round_trips_hostile_strings() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    for (i, value) in HOSTILE.iter().enumerate() {
        let name = format!("p{i} {value}");
        let mut profile = Profile::new(name.clone());
        profile.additional_parameters = Some((*value).to_owned());
        profile.addon_names_by_priority = vec![(*value).to_owned(), "@x".to_owned()];
        profile.addon_search_directories = vec![(*value).to_owned()];
        profile.last_added_addon_search_directory = Some((*value).to_owned());
        profile.launcher_options.game_profile = Some((*value).to_owned());
        let mut leaf = TreeLeaf::new(*value);
        leaf.workshop_id = Some((*value).to_owned());
        let mut group = TreeDirectory::new(*value);
        group.modset_repository_name = Some((*value).to_owned());
        group.children.push(TreeNode::Leaf(leaf));
        profile.tree.children.push(TreeNode::Directory(group));
        store.write_profile(&profile).unwrap();
        let load = store.load_profiles().unwrap();
        assert!(
            load.failures.is_empty(),
            "{value:?}: {:?}",
            load.error_message()
        );
        assert_eq!(load.profiles[&name], profile, "{value:?}");
    }
}

#[test]
fn verify_profile_round_trips_a_deep_tree() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut profile = Profile::new("Deep");
    profile
        .tree
        .children
        .push(TreeNode::Directory(deep_tree(40)));
    store.write_profile(&profile).unwrap();
    let load = store.load_profiles().unwrap();
    assert!(load.failures.is_empty(), "{:?}", load.error_message());
    assert_eq!(load.profiles["Deep"], profile);
}

#[test]
fn verify_repository_round_trips_extreme_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut r = repository("Extreme");
    r.revision = i32::MAX;
    r.number_of_client_connections = i32::MIN;
    r.number_of_repository_check_connections = -1;
    r.parallel_upload_connections = 0;
    r.number_of_connections = i32::MAX;
    r.maximum_client_download_speed = 1e300;
    r.default_download_location = Some(String::new());
    r.auto_config_url = Some("ftp://h/x?a=1&b=\"2\"#frag".to_owned());
    r.path = Some("C:\\Arma 3\\@mods\\".to_owned());
    for value in HOSTILE {
        r.events_download_location
            .insert((*value).to_owned(), format!("loc {value}"));
        r.hidden_folder_paths.insert((*value).to_owned());
        r.excluded_files_from_build.insert((*value).to_owned());
    }
    r.events_download_location
        .insert("a.b.c".to_owned(), "dotted key".to_owned());
    r.events_download_location
        .insert("[x]".to_owned(), "bracket key".to_owned());
    store.write_repository(&r).unwrap();
    let load = store.load_repositories().unwrap();
    assert!(load.failures.is_empty(), "{:?}", load.error_message());
    assert_eq!(load.repositories["Extreme"].repository, r);

    // A huge value (1 MiB string).
    let mut big = repository("Big");
    big.path = Some("p".repeat(1 << 20));
    store.write_repository(&big).unwrap();
    let load = store.load_repositories().unwrap();
    assert_eq!(load.repositories["Big"].repository, big);
}

#[test]
fn verify_sha1_cache_round_trips_hostile_keys_and_extremes() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut cache = Sha1Cache::default();
    let entries: Vec<(&str, &str, i64)> = vec![
        ("/srv/arma3/@Mod A/addons/empty.txt", "0", 0),
        (
            "C:\\Arma 3\\@mod\\addons\\x.pbo",
            "de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3",
            i64::MAX,
        ),
        ("/x/\"quoted\" 'single' [br].pbo", "a", i64::MIN),
        ("/x/ünïcödé/日本/🎮.pbo", "b", -1),
        ("/x/new\nline.pbo", "c", 1_704_024_000_000),
        ("", "d", 5),
    ];
    for (path, sha1, mtime) in &entries {
        cache.entries.insert(
            PathBuf::from(path),
            FileAttributes {
                sha1: (*sha1).to_owned(),
                last_modified: *mtime,
            },
        );
    }
    store
        .write_sha1_cache("Repo", CacheKind::Build, &cache)
        .unwrap();
    assert_eq!(store.read_sha1_cache("Repo", CacheKind::Build), cache);
    // The other kind is independent.
    assert!(
        store
            .read_sha1_cache("Repo", CacheKind::Sync)
            .entries
            .is_empty()
    );
}

#[test]
fn verify_unknown_fields_and_format_edge_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    // A non-integer format is a parse error, not a panic.
    std::fs::write(store.preferences_path(), "format = \"1\"\n[preferences]\n").unwrap();
    assert!(store.read_preferences().is_err());
    // A huge format is UnsupportedFormat.
    std::fs::write(
        store.preferences_path(),
        format!("format = {}\n[preferences]\n", i64::MAX),
    )
    .unwrap();
    let err = store.read_preferences().unwrap_err();
    assert!(
        matches!(err, a3s_repo_files::local::Error::UnsupportedFormat { found, .. } if found == i64::MAX),
        "{err:?}"
    );
    // A payload that is not a table is a parse error.
    std::fs::write(store.preferences_path(), "format = 1\npreferences = 3\n").unwrap();
    assert!(store.read_preferences().is_err());
}

// ---------------------------------------------------------------------------------------------
// File modes and atomic writes

#[cfg(unix)]
#[test]
fn verify_every_store_file_is_0600() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    // A pre-existing world-readable target ends up private after the replace.
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    std::fs::write(store.configuration_path(), "format = 1\n").unwrap();
    std::fs::set_permissions(
        store.configuration_path(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    store
        .write_configuration(&a3s_types::local::Configuration::default())
        .unwrap();
    store
        .write_preferences(&a3s_types::local::Preferences::default())
        .unwrap();
    store.write_profile(&Profile::new("P")).unwrap();
    store.write_repository(&repository("R")).unwrap();
    let mut cache = Sha1Cache::default();
    cache.entries.insert(
        PathBuf::from("/a"),
        FileAttributes {
            sha1: "0".to_owned(),
            last_modified: 1,
        },
    );
    store
        .write_sha1_cache("R", CacheKind::Sync, &cache)
        .unwrap();
    store
        .write_sha1_cache("R", CacheKind::Build, &cache)
        .unwrap();
    store.workshop().save("@ace", "463939057", "ACE").unwrap();
    store
        .write_legacy_import_marker(&a3s_repo_files::local::ImportMarker::default())
        .unwrap();
    for path in [
        store.configuration_path(),
        store.preferences_path(),
        store.profile_path("P"),
        store.repository_path("R"),
        store.sha1_cache_path("R", CacheKind::Sync),
        store.sha1_cache_path("R", CacheKind::Build),
        store.workshop_path(),
        store.legacy_import_marker_path(),
    ] {
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{}", path.display());
    }
}

#[cfg(unix)]
#[test]
fn verify_failed_repository_write_keeps_old_file_and_no_temp() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut r = repository("WOG");
    r.revision = 1;
    store.write_repository(&r).unwrap();
    let before = std::fs::read(store.repository_path("WOG")).unwrap();
    let repos = store.dirs().repositories.clone();
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o555)).unwrap();
    r.revision = 2;
    let result = store.write_repository(&r);
    let removed = store.remove_repository("WOG");
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o755)).unwrap();
    if result.is_ok() {
        // Running as root: permissions are not enforced, nothing to check.
        return;
    }
    assert!(removed.is_err(), "removal in a read-only dir must fail");
    assert_eq!(std::fs::read(store.repository_path("WOG")).unwrap(), before);
    let names: Vec<_> = std::fs::read_dir(&repos)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    // The load still sees the old version.
    let load = store.load_repositories().unwrap();
    assert_eq!(load.repositories["WOG"].repository.revision, 1);
}

#[test]
fn verify_write_into_a_file_instead_of_a_directory_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let store = LocalStore::with_dirs(LocalDirs {
        config: blocker.join("config"),
        profiles: blocker.join("profiles"),
        repositories: blocker.join("repositories"),
        sha1_cache: blocker.join("sha1"),
    });
    assert!(store.write_repository(&repository("X")).is_err());
    assert!(store.write_profile(&Profile::new("X")).is_err());
    assert!(
        store
            .write_sha1_cache("X", CacheKind::Sync, &Sha1Cache::default())
            .is_err()
    );
    assert!(store.workshop().save("@a", "1", "A").is_err());
    // Reads of a path below a regular file are errors or empty, never panics.
    assert!(store.load_repositories().is_err());
    let _profiles: Result<_, _> = store.load_profiles();
    let _configuration: Result<_, _> = store.read_configuration();
    assert!(
        store
            .read_sha1_cache("X", CacheKind::Sync)
            .entries
            .is_empty()
    );
    assert!(store.legacy_import_marker().is_none());
}

#[test]
fn verify_temp_leftovers_are_ignored_by_listings() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_repository(&repository("A")).unwrap();
    store.write_profile(&Profile::new("P")).unwrap();
    // Simulated crash leftovers.
    std::fs::write(store.dirs().repositories.join(".tmpAbC123"), b"garbage").unwrap();
    std::fs::write(store.dirs().profiles.join(".tmpXyZ789"), b"garbage").unwrap();
    // A directory named like a store file is not a candidate.
    std::fs::create_dir_all(store.dirs().repositories.join("dir.toml")).unwrap();
    let repos = store.load_repositories().unwrap();
    assert!(repos.failures.is_empty(), "{:?}", repos.error_message());
    assert_eq!(repos.repositories.len(), 1);
    let profiles = store.load_profiles().unwrap();
    assert!(
        profiles.failures.is_empty(),
        "{:?}",
        profiles.error_message()
    );
}

// ---------------------------------------------------------------------------------------------
// Duplicates

#[test]
fn verify_duplicate_profiles_tie_keeps_first_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut a = Profile::new("Same");
    a.additional_parameters = Some("first".to_owned());
    let mut b = Profile::new("Same");
    b.additional_parameters = Some("second".to_owned());
    std::fs::create_dir_all(&store.dirs().profiles).unwrap();
    let pa = store.dirs().profiles.join("a.toml");
    let pb = store.dirs().profiles.join("b.toml");
    std::fs::write(&pa, toml_profile(&a)).unwrap();
    std::fs::write(&pb, toml_profile(&b)).unwrap();
    set_mtime(&pa, 5_000);
    set_mtime(&pb, 5_000);
    let load = store.load_profiles().unwrap();
    assert_eq!(
        load.profiles["Same"].additional_parameters.as_deref(),
        Some("first")
    );
    assert_eq!(
        load.warnings,
        ["Same: ignored \"b.toml\" because \"a.toml\" is newer."]
    );
    // An older later file is ignored too; a newer later file replaces.
    set_mtime(&pb, 6_000);
    let load = store.load_profiles().unwrap();
    assert_eq!(
        load.profiles["Same"].additional_parameters.as_deref(),
        Some("second")
    );
    assert_eq!(
        load.warnings,
        ["Same: replaced \"a.toml\" with \"b.toml\" because it is newer."]
    );
}

#[test]
fn verify_three_way_repository_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().repositories).unwrap();
    let mut files = BTreeMap::new();
    for (file, revision, mtime) in [
        ("a.toml", 1, 3_000),
        ("b.toml", 2, 1_000),
        ("c.toml", 3, 9_000),
    ] {
        let mut r = repository("Dup");
        r.revision = revision;
        let path = store.dirs().repositories.join(file);
        std::fs::write(&path, toml_repository(&r)).unwrap();
        set_mtime(&path, mtime);
        files.insert(file, path);
    }
    let load = store.load_repositories().unwrap();
    assert_eq!(load.repositories["Dup"].repository.revision, 3);
    assert_eq!(load.repositories["Dup"].source, files["c.toml"]);
    assert_eq!(
        load.duplicate_warnings,
        [
            "Dup: ignored \"b.toml\" because \"a.toml\" is newer.",
            "Dup: replaced \"a.toml\" with \"c.toml\" because it is newer.",
        ]
    );
}

/// The TOML the store writes for `profile`, via a scratch store.
fn toml_profile(profile: &Profile) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    s.write_profile(profile).unwrap();
    std::fs::read(s.profile_path(&profile.name)).unwrap()
}

fn toml_repository(repository: &Repository) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    s.write_repository(repository).unwrap();
    std::fs::read(s.repository_path(&repository.name)).unwrap()
}
