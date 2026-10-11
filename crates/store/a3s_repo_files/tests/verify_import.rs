//! Verifier tests for `legacy::import`: never overwriting Rust data, idempotency
//!, duplicate resolution, failure isolation and store-write failures.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use a3s_jserial::{Graph, Node};
use a3s_paths::legacy::{LegacyJavaData, discover_legacy_java};
use a3s_paths::{Os, PlatformEnv};
use a3s_repo_files::legacy::{
    FILE_CORRUPTED, ImportOptions, ItemOutcome, NamedOutcome, import, read_configuration_file,
};
use a3s_repo_files::local::{CacheKind, LocalDirs, LocalStore};
use a3s_types::local::{Profile, Repository};
use a3s_types::{Protocol, ProtocolType};

const HTTP_FILE: &str = "Sample_HTTP__5ab39964a7ab.a3s.repository";
const FTP_FILE: &str = "Sample_FTP__21a7c6dd6e0b.a3s.repository";
const LEGACY_FTP_FILE: &str = "SampleFTP.a3s.repository";

const WORKSHOP: &str = "#Arma3Sync known Workshop metadata\n\
                        mod.QGFjZQ.key=@ace\n\
                        mod.QGFjZQ.name=ACE\n\
                        mod.QGFjZQ.publishedid=463939057\n";

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_local")
        .join(rel)
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Every file below `dir` with its bytes and mtime.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return out;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.insert(path.clone(), (Vec::new(), SystemTime::UNIX_EPOCH));
            out.extend(snapshot(&path));
        } else {
            let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
            out.insert(path.clone(), (std::fs::read(&path).unwrap(), mtime));
        }
    }
    out
}

fn bytes_of(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    snapshot(dir)
        .into_iter()
        .map(|(path, (bytes, _))| (path, bytes))
        .collect()
}

fn set_mtime(path: &Path, secs: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn store(root: &Path) -> LocalStore {
    LocalStore::with_dirs(LocalDirs {
        config: root.join("config"),
        profiles: root.join("config/profiles"),
        repositories: root.join("data/repositories"),
        sha1_cache: root.join("data/sha1_cache"),
    })
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .unwrap();
    raw
}

fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).unwrap();
    encoder.finish().unwrap()
}

fn rewrite(path: &Path, f: impl FnOnce(&mut Graph)) {
    let mut graph = a3s_jserial::read(&gunzip(&std::fs::read(path).unwrap())).unwrap();
    f(&mut graph);
    std::fs::write(path, gzip(&a3s_jserial::write(&graph).unwrap())).unwrap();
}

fn set_root_string(graph: &mut Graph, field: &str, value: &str) {
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let id = graph.field(root, field).unwrap().as_ref().unwrap();
    *graph.node_mut(id).unwrap() = Node::String(value.to_owned());
}

/// A copy of the Java per-user fixture with `active` as the active profile and a Workshop file.
fn java_layout(root: &Path, active: &str) -> PathBuf {
    let java = root.join("java");
    copy_tree(&fixture("peruser"), &java);
    let configuration = java.join("config/Arma3Sync/configuration");
    rewrite(&configuration.join("a3s.cfg"), |g| {
        set_root_string(g, "profileName", active);
    });
    std::fs::write(
        configuration.join("known-workshop-mods.properties"),
        WORKSHOP,
    )
    .unwrap();
    let repositories = java.join("data/Arma3Sync/repositories");
    set_mtime(&repositories.join(LEGACY_FTP_FILE), 1_000);
    set_mtime(&repositories.join(FTP_FILE), 2_000);
    java
}

fn sources(java: &Path) -> Vec<LegacyJavaData> {
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(java.join("home")),
        &[
            ("XDG_CONFIG_HOME", java.join("config").to_str().unwrap()),
            ("XDG_DATA_HOME", java.join("data").to_str().unwrap()),
            ("XDG_CACHE_HOME", java.join("cache").to_str().unwrap()),
        ],
        None,
    );
    discover_legacy_java(&env, &[])
}

fn java_repos(java: &Path) -> PathBuf {
    java.join("data/Arma3Sync/repositories")
}

fn java_profiles(java: &Path) -> PathBuf {
    java.join("config/Arma3Sync/profiles")
}

fn outcome_of<'a>(list: &'a [NamedOutcome], file: &str) -> &'a ItemOutcome {
    &list
        .iter()
        .find(|o| o.file == file)
        .unwrap_or_else(|| panic!("no outcome for {file}: {list:?}"))
        .outcome
}

fn repository(name: &str) -> Repository {
    Repository::new(
        name,
        Protocol::new(ProtocolType::Http, "mine.example/x", "80", "me", "", true),
    )
}

// ---------------------------------------------------------------------------------------------
// Never overwrite Rust data

#[test]
fn verify_newer_format_workshop_file_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    // Written by a newer Rust client: this one cannot read it, but it is user data.
    let newer = "format = 2\n[workshop.mods.\"@mine\"]\npublished_id = \"42\"\nname = \"Mine\"\n";
    std::fs::write(store.workshop_path(), newer).unwrap();

    import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    assert_eq!(
        std::fs::read_to_string(store.workshop_path()).unwrap(),
        newer,
        "import replaced an existing (unreadable) Rust workshop file"
    );
}

#[test]
fn verify_corrupt_workshop_file_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    let corrupt = "format = 1\n[workshop.mods.\"@mine\"\npublished_id = \"42\"\n";
    std::fs::write(store.workshop_path(), corrupt).unwrap();

    import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    assert_eq!(
        std::fs::read_to_string(store.workshop_path()).unwrap(),
        corrupt,
        "import replaced an existing (corrupt) Rust workshop file"
    );
}

#[test]
fn verify_unreadable_or_renamed_rust_files_are_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Event Night");
    let rust = tmp.path().join("rust");
    let store = store(&rust);
    std::fs::create_dir_all(&store.dirs().repositories).unwrap();
    std::fs::create_dir_all(&store.dirs().profiles).unwrap();
    std::fs::create_dir_all(&store.dirs().config).unwrap();

    // "Sample HTTP" at its canonical path, written by a newer version.
    let newer = "format = 2\n[repository]\nname = \"Sample HTTP\"\n";
    std::fs::write(store.repository_path("Sample HTTP"), newer).unwrap();
    // "Sample SFTP" under a file name the user chose.
    let mut mine = repository("Sample SFTP");
    mine.revision = 1234;
    store.write_repository(&mine).unwrap();
    let custom = store.dirs().repositories.join("custom.toml");
    std::fs::rename(store.repository_path("Sample SFTP"), &custom).unwrap();
    // A corrupt profile at "Event Night"'s path.
    std::fs::write(store.profile_path("Event Night"), "not toml [[[").unwrap();
    // Corrupt configuration and preferences.
    std::fs::write(store.configuration_path(), "garbage = [").unwrap();
    std::fs::write(store.preferences_path(), "format = 9\n").unwrap();
    let before = bytes_of(&rust);

    let report = import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    let source = &report.sources[0];
    assert_eq!(
        outcome_of(&source.repositories, HTTP_FILE),
        &ItemOutcome::SkippedExisting
    );
    assert_eq!(
        outcome_of(
            &source.repositories,
            "Sample_SFTP__132c722dd1cb.a3s.repository"
        ),
        &ItemOutcome::SkippedExisting
    );
    assert_eq!(
        outcome_of(&source.profiles, "Event Night.a3s.profile"),
        &ItemOutcome::SkippedExisting
    );
    assert_eq!(source.configuration, ItemOutcome::SkippedExisting);
    assert_eq!(source.preferences, ItemOutcome::SkippedExisting);

    let after = bytes_of(&rust);
    for (path, bytes) in &before {
        assert_eq!(after.get(path), Some(bytes), "{} changed", path.display());
    }
    // No stray copy of the renamed repository either.
    assert!(!store.repository_path("Sample SFTP").exists());
    assert!(
        store
            .read_sha1_cache("Sample HTTP", CacheKind::Build)
            .entries
            .is_empty()
    );
}

// ---------------------------------------------------------------------------------------------
// Idempotency

#[test]
fn verify_idempotent_with_a_created_default_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    std::fs::remove_file(java_profiles(&java).join("Default.a3s.profile")).unwrap();
    let rust = tmp.path().join("rust");
    let store = store(&rust);
    let java_before = snapshot(&java);

    let first = import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    let migration = first.sources[0].migration.clone().unwrap();
    assert!(migration.contains("(created)"), "{migration}");
    let default = store.load_profiles().unwrap().profiles["Default"].clone();
    assert_eq!(default.addon_search_directories.len(), 2);
    assert!(store.profile_path("Default").is_file());
    let mut once = bytes_of(&rust);
    once.remove(&store.legacy_import_marker_path());

    for _ in 0..2 {
        let again = import(
            &store,
            &sources(&java),
            &ImportOptions {
                ignore_marker: true,
            },
        )
        .unwrap();
        assert!(!again.imported_anything(), "{:?}", again.summary());
        assert_eq!(again.sources[0].migration, None);
        let mut now = bytes_of(&rust);
        now.remove(&store.legacy_import_marker_path());
        assert_eq!(now, once);
    }
    // Search directories were not appended a second time.
    assert_eq!(
        store.load_profiles().unwrap().profiles["Default"]
            .addon_search_directories
            .len(),
        2
    );
    assert_eq!(snapshot(&java), java_before, "the Java tree changed");
}

// ---------------------------------------------------------------------------------------------
// Duplicates

#[test]
fn verify_repository_duplicates_equal_mtime_first_file_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let repos = java_repos(&java);
    // Make the legacy-named copy distinguishable, then give both the same mtime.
    let legacy_plain = gunzip(&std::fs::read(fixture("plain/Sample_FTP.repository.gz")).unwrap());
    let mut graph = a3s_jserial::read(&legacy_plain).unwrap();
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let path_id = graph.field(root, "path").unwrap().as_ref().unwrap();
    *graph.node_mut(path_id).unwrap() = Node::String("/legacy/copy".to_owned());
    std::fs::write(
        repos.join(LEGACY_FTP_FILE),
        a3s_repo_files::legacy::sealed::seal(&a3s_jserial::write(&graph).unwrap()).unwrap(),
    )
    .unwrap();
    set_mtime(&repos.join(LEGACY_FTP_FILE), 7_000);
    set_mtime(&repos.join(FTP_FILE), 7_000);

    let store = store(&tmp.path().join("rust"));
    let report = import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    let source = &report.sources[0];
    assert_eq!(
        outcome_of(&source.repositories, LEGACY_FTP_FILE),
        &ItemOutcome::Imported
    );
    assert_eq!(
        outcome_of(&source.repositories, FTP_FILE),
        &ItemOutcome::SkippedExisting
    );
    assert!(report.warnings.contains(&format!(
        "Sample FTP: ignored \"{FTP_FILE}\" because \"{LEGACY_FTP_FILE}\" is newer."
    )));
    let load = store.load_repositories().unwrap();
    assert_eq!(
        load.repositories["Sample FTP"].repository.path.as_deref(),
        Some("/legacy/copy")
    );
}

#[test]
fn verify_primary_profile_beats_a_newer_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let profiles = java_profiles(&java);
    let primary = profiles.join("Event Night.a3s.profile");
    let backup = profiles.join("Event Night.a3s.profile.backup");
    std::fs::copy(&primary, &backup).unwrap();
    rewrite(&backup, |g| {
        set_root_string(g, "additionalParameters", "-from-backup");
    });
    set_mtime(&primary, 1_000);
    set_mtime(&backup, 9_000);

    let store = store(&tmp.path().join("rust"));
    let report = import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    let source = &report.sources[0];
    assert_eq!(
        outcome_of(&source.profiles, "Event Night.a3s.profile"),
        &ItemOutcome::Imported
    );
    assert_eq!(
        outcome_of(&source.profiles, "Event Night.a3s.profile.backup"),
        &ItemOutcome::SkippedExisting
    );
    let loaded = &store.load_profiles().unwrap().profiles["Event Night"];
    assert_ne!(
        loaded.additional_parameters.as_deref(),
        Some("-from-backup")
    );
}

// ---------------------------------------------------------------------------------------------
// Failure isolation and store failures

#[test]
fn verify_bad_entries_are_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let repos = java_repos(&java);
    let profiles = java_profiles(&java);
    std::fs::write(repos.join("zero.a3s.repository"), b"").unwrap();
    std::fs::write(repos.join("text.a3s.repository"), b"hello").unwrap();
    std::fs::create_dir(repos.join("dir.a3s.repository")).unwrap();
    std::fs::create_dir(profiles.join("dir.a3s.profile")).unwrap();
    std::fs::write(profiles.join("zero.a3s.profile"), b"").unwrap();
    // A profile file that is really an encrypted repository (wrong root class).
    std::fs::copy(repos.join(HTTP_FILE), profiles.join("wrong.a3s.profile")).unwrap();

    let store = store(&tmp.path().join("rust"));
    let report = import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    let source = &report.sources[0];

    for file in ["zero.a3s.repository", "text.a3s.repository"] {
        match outcome_of(&source.repositories, file) {
            ItemOutcome::Failed(message) => {
                assert!(message.ends_with(FILE_CORRUPTED), "{message}");
            }
            other => panic!("{file}: {other:?}"),
        }
    }
    match outcome_of(&source.profiles, "zero.a3s.profile") {
        ItemOutcome::Failed(message) => assert!(message.ends_with(FILE_CORRUPTED), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        outcome_of(&source.profiles, "wrong.a3s.profile"),
        ItemOutcome::Failed(_)
    ));
    // Directories are not candidates at all.
    assert!(
        source
            .repositories
            .iter()
            .all(|o| !o.file.starts_with("dir."))
    );
    assert!(source.profiles.iter().all(|o| !o.file.starts_with("dir.")));
    // The good items are still imported and the marker is written.
    assert_eq!(store.load_repositories().unwrap().repositories.len(), 3);
    assert!(
        store
            .load_profiles()
            .unwrap()
            .sources
            .contains_key("Event Night")
    );
    let marker = store.legacy_import_marker().unwrap();
    assert_eq!(marker.counts["repositories.failed"], 2);
    assert_eq!(marker.counts["profiles.failed"], 2);
}

#[test]
fn verify_java_cache_quirks_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    // SHA-1 "0" of an empty file and ms mtimes are kept as Java wrote them.
    let build = store.read_sha1_cache("Sample FTP", CacheKind::Build);
    let empty = &build.entries[Path::new("/srv/arma3/SampleRepo/@Mod_A/addons/empty.txt")];
    assert_eq!(empty.sha1, "0");
    assert_eq!(empty.last_modified, 1_704_024_120_000);
    let sync = store.read_sha1_cache("Sample FTP", CacheKind::Sync);
    assert_eq!(
        sync.entries[Path::new("/srv/arma3/SampleRepo/downloads/@Mod_A/addons/empty.txt")].sha1,
        "0"
    );
    // The configuration's credentials survive (plain TOML, 0600).
    let cfg = read_configuration_file(&java.join("config/Arma3Sync/configuration/a3s.cfg"))
        .unwrap()
        .value;
    assert_eq!(store.read_configuration().unwrap(), cfg.configuration);
}

#[cfg(unix)]
#[test]
fn verify_store_write_failure_aborts_without_marker() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    std::fs::create_dir_all(&store.dirs().repositories).unwrap();
    std::fs::set_permissions(
        &store.dirs().repositories,
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    let result = import(&store, &sources(&java), &ImportOptions::default());
    std::fs::set_permissions(
        &store.dirs().repositories,
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let Err(err) = result else {
        // Running as root: permissions are not enforced.
        return;
    };
    assert!(
        a3s_error::UserFacing::user_message(&err).starts_with("Failed to write file: "),
        "{}",
        a3s_error::UserFacing::user_message(&err)
    );
    assert!(store.legacy_import_marker().is_none());
    assert!(!store.legacy_import_marker_path().exists());

    // After the problem is fixed, a new run completes what is missing.
    let report = import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    assert!(report.imported_anything());
    assert_eq!(store.load_repositories().unwrap().repositories.len(), 3);
    assert!(store.legacy_import_marker().is_some());
}

#[test]
fn verify_profile_with_unsafe_name_is_imported_under_a_safe_stem() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_layout(tmp.path(), "Default");
    let profiles = java_profiles(&java);
    let path = profiles.join("evil.a3s.profile");
    std::fs::copy(profiles.join("Event Night.a3s.profile"), &path).unwrap();
    rewrite(&path, |g| set_root_string(g, "name", "../../escape\\CON:*"));

    let rust = tmp.path().join("rust");
    let store = store(&rust);
    import(&store, &sources(&java), &ImportOptions::default()).unwrap();
    let load = store.load_profiles().unwrap();
    assert!(load.profiles.contains_key("../../escape\\CON:*"));
    // Nothing was written outside the profiles directory.
    let profile_path = store.profile_path("../../escape\\CON:*");
    assert_eq!(profile_path.parent().unwrap(), store.dirs().profiles);
    assert!(profile_path.is_file());
    assert!(!tmp.path().join("escape").exists());
    let p: Profile = load.profiles["../../escape\\CON:*"].clone();
    assert_eq!(p.name, "../../escape\\CON:*");
}
