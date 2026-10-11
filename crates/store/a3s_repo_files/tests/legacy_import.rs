//! One-time import of the Java client's data, on copies of the real Java-written
//! files of `tests/fixtures/java_local`.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use a3s_foundation::paths::legacy::{
    LegacyJavaData, discover_legacy_java, inspect_java_installation,
};
use a3s_foundation::paths::{Os, PlatformEnv};
use a3s_jserial::{ArrayData, Graph, Node};
use a3s_repo_files::legacy::{
    ImportOptions, ImportReport, ItemOutcome, NamedOutcome, import, read_configuration_file,
    read_profile_file, read_repository_file, sealed, should_offer_import,
};
use a3s_repo_files::local::{CacheKind, LocalDirs, LocalStore, WorkshopEntry};
use a3s_types::local::{LauncherOptions, Profile};

// ---------------------------------------------------------------------------------------------
// Helpers

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

/// Every file below `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return out;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(snapshot(&path));
        } else {
            out.insert(path.clone(), std::fs::read(&path).unwrap());
        }
    }
    out
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

/// Copies a Java `GZIP(stream)` file to `dst`, applying `f` to its graph.
fn rewrite(src: &Path, dst: &Path, f: impl FnOnce(&mut Graph)) {
    let mut graph = a3s_jserial::read(&gunzip(&std::fs::read(src).unwrap())).unwrap();
    f(&mut graph);
    std::fs::write(dst, gzip(&a3s_jserial::write(&graph).unwrap())).unwrap();
}

/// Replaces the string referenced by `field` of the root object.
fn set_root_string(graph: &mut Graph, field: &str, value: &str) {
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let id = graph.field(root, field).unwrap().as_ref().unwrap();
    *graph.node_mut(id).unwrap() = Node::String(value.to_owned());
}

/// Writes an encrypted repository file whose inner stream is the plain copy `plain` after `f`.
fn sealed_repository(plain: &str, dst: &Path, f: impl FnOnce(&mut Graph)) {
    let mut graph = a3s_jserial::read(&gunzip(&std::fs::read(fixture(plain)).unwrap())).unwrap();
    f(&mut graph);
    let inner = a3s_jserial::write(&graph).unwrap();
    std::fs::write(dst, sealed::seal(&inner).unwrap()).unwrap();
}

/// The Java per-user layout of `root` (`XDG_*_HOME` below it).
fn per_user_sources(root: &Path, extra: &[PathBuf]) -> Vec<LegacyJavaData> {
    let config = root.join("config");
    let data = root.join("data");
    let cache = root.join("cache");
    let env = PlatformEnv::for_tests(
        Os::Linux,
        Some(root.join("home")),
        &[
            ("XDG_CONFIG_HOME", config.to_str().unwrap()),
            ("XDG_DATA_HOME", data.to_str().unwrap()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
        None,
    );
    discover_legacy_java(&env, extra)
}

fn outcomes(list: &[NamedOutcome]) -> Vec<(&str, ItemOutcome)> {
    list.iter()
        .map(|o| (o.file.as_str(), o.outcome.clone()))
        .collect()
}

const HTTP_FILE: &str = "Sample_HTTP__5ab39964a7ab.a3s.repository";
const FTP_FILE: &str = "Sample_FTP__21a7c6dd6e0b.a3s.repository";
const SFTP_FILE: &str = "Sample_SFTP__132c722dd1cb.a3s.repository";
const LEGACY_FTP_FILE: &str = "SampleFTP.a3s.repository";

const WORKSHOP: &str = "#Arma3Sync known Workshop metadata\n\
                        #Sat Oct 11 12:00:00 CEST 2026\n\
                        mod.QGFjZQ.key=@ace\n\
                        mod.QGFjZQ.name=ACE\n\
                        mod.QGFjZQ.publishedid=463939057\n";

/// A copy of the per-user fixture with the active profile set to `active`, a `.backup` copy of
/// a repository, a Workshop file and fixed mtimes.
fn java_per_user(root: &Path, active: &str) -> PathBuf {
    let java = root.join("java");
    copy_tree(&fixture("peruser"), &java);
    let configuration = java.join("config/Arma3Sync/configuration");
    let cfg = configuration.join("a3s.cfg");
    rewrite(&cfg, &cfg, |g| {
        set_root_string(g, "profileName", active);
    });
    std::fs::write(
        configuration.join("known-workshop-mods.properties"),
        WORKSHOP,
    )
    .unwrap();
    let repositories = java.join("data/Arma3Sync/repositories");
    std::fs::copy(
        repositories.join(HTTP_FILE),
        repositories.join(format!("{HTTP_FILE}.backup")),
    )
    .unwrap();
    set_mtime(&repositories.join(LEGACY_FTP_FILE), 1_000);
    set_mtime(&repositories.join(FTP_FILE), 2_000);
    java
}

/// The 17 launcher fields Java copies from `a3s.cfg` onto `base`.
fn migrated(base: &LauncherOptions, old: &LauncherOptions) -> LauncherOptions {
    LauncherOptions {
        arma3_exe_path: old.arma3_exe_path.clone(),
        auto_restart: old.auto_restart,
        check_signatures: old.check_signatures,
        file_patching: old.file_patching,
        no_pause: old.no_pause,
        show_script_errors: old.show_script_errors,
        window_mode: old.window_mode,
        cpu_count_selection: old.cpu_count_selection,
        default_world: old.default_world,
        enable_ht: old.enable_ht,
        huge_pages: old.huge_pages,
        ex_threads_selection: old.ex_threads_selection.clone(),
        game_profile: old.game_profile.clone(),
        malloc_selection: old.malloc_selection.clone(),
        max_memory_selection: old.max_memory_selection.clone(),
        nologs: old.nologs,
        no_splash_screen: old.no_splash_screen,
        ..base.clone()
    }
}

// ---------------------------------------------------------------------------------------------

#[test]
fn full_import_of_the_java_per_user_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let before = snapshot(&java);
    let sources = per_user_sources(&java, &[]);
    assert_eq!(sources.len(), 1);
    let store = store(&tmp.path().join("rust"));

    let report = import(&store, &sources, &ImportOptions::default()).unwrap();
    let source = &report.sources[0];
    assert_eq!(
        outcomes(&source.profiles),
        [
            ("Default.a3s.profile", ItemOutcome::Imported),
            ("Event Night.a3s.profile", ItemOutcome::Imported),
        ]
    );
    // The `.backup` file is not a candidate; the legacy-named copy loses to the newer file.
    assert_eq!(
        outcomes(&source.repositories),
        [
            (LEGACY_FTP_FILE, ItemOutcome::SkippedExisting),
            (FTP_FILE, ItemOutcome::Imported),
            (HTTP_FILE, ItemOutcome::Imported),
            (SFTP_FILE, ItemOutcome::Imported),
        ]
    );
    assert_eq!(source.configuration, ItemOutcome::Imported);
    assert_eq!(source.preferences, ItemOutcome::Imported);
    assert_eq!(source.workshop, ItemOutcome::Imported);
    assert_eq!(
        report.warnings,
        [format!(
            "Sample FTP: replaced \"{LEGACY_FTP_FILE}\" with \"{FTP_FILE}\" because it is newer."
        )]
    );
    assert!(report.imported_anything());
    assert_eq!(
        report.summary(),
        [
            "Profiles: 2 imported, 0 skipped, 0 failed",
            "Repositories: 3 imported, 1 skipped, 0 failed",
            "Configuration: 1 imported, 0 skipped, 0 failed",
            "Preferences: 1 imported, 0 skipped, 0 failed",
            "Workshop metadata: 1 imported, 0 skipped, 0 failed",
        ]
    );

    // Repositories with every value and their caches.
    let repositories = java.join("data/Arma3Sync/repositories");
    let load = store.load_repositories().unwrap();
    assert!(load.failures.is_empty());
    assert_eq!(load.repositories.len(), 3);
    for file in [HTTP_FILE, FTP_FILE, SFTP_FILE] {
        let java_repo = read_repository_file(&repositories.join(file))
            .unwrap()
            .value;
        let name = &java_repo.repository.name;
        assert_eq!(load.repositories[name].repository, java_repo.repository);
        for (kind, map) in [
            (CacheKind::Sync, &java_repo.files_for_sync),
            (CacheKind::Build, &java_repo.files_for_build),
        ] {
            let cache = store.read_sha1_cache(name, kind);
            assert_eq!(cache.entries.len(), map.len());
            for (path, attributes) in map {
                let cached = &cache.entries[Path::new(path)];
                assert_eq!(cached.sha1, attributes.sha1);
                assert_eq!(cached.last_modified, attributes.last_modified);
            }
        }
    }

    // Profiles: the active one received the deprecated settings, the other is untouched.
    let java_cfg = read_configuration_file(&java.join("config/Arma3Sync/configuration/a3s.cfg"))
        .unwrap()
        .value;
    let profiles = store.load_profiles().unwrap();
    let event_night =
        read_profile_file(&java.join("config/Arma3Sync/profiles/Event Night.a3s.profile"))
            .unwrap()
            .value;
    assert_eq!(profiles.profiles["Event Night"], event_night);
    let default = &profiles.profiles["Default"];
    let mut expected = Profile::new("Default");
    expected.addon_search_directories = vec![
        "/home/user/arma3/!Workshop".to_owned(),
        "C:\\Games\\Arma 3".to_owned(),
    ];
    let old = java_cfg.launcher_options.clone().unwrap();
    expected.launcher_options = migrated(&LauncherOptions::default(), &old);
    assert_eq!(default, &expected);
    // Not copied, although set in `a3s.cfg`.
    assert!(old.steam_exe_path.is_some());
    assert_eq!(default.launcher_options.steam_exe_path, None);
    assert!(!default.launcher_options.mission_file);
    assert_eq!(default.launcher_options.mission_file_path, None);
    assert!(!default.launcher_options.launch_arma3_as_administrator);
    assert_eq!(
        source.migration.as_deref(),
        Some(
            "Migrated 2 search directories and the launcher options of a3s.cfg into profile \
             \"Default\"."
        )
    );

    // Configuration, preferences, workshop.
    assert_eq!(store.read_configuration().unwrap(), java_cfg.configuration);
    assert_eq!(
        store.read_preferences().unwrap().look_and_feel,
        a3s_types::constants::LookAndFeel::LafHifi
    );
    assert_eq!(
        store.workshop().find("@ace"),
        Some(WorkshopEntry {
            addon_key: "@ace".to_owned(),
            published_id: "463939057".to_owned(),
            name: "ACE".to_owned(),
        })
    );

    // The Java files are untouched.
    assert_eq!(snapshot(&java), before);
}

#[test]
fn search_directories_are_appended_without_deduplication() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Event Night");
    let store = store(&tmp.path().join("rust"));
    import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    let profile = &store.load_profiles().unwrap().profiles["Event Night"];
    assert_eq!(
        profile.addon_search_directories,
        [
            "/srv/arma3/SampleRepo",
            "/home/user/arma3/!Workshop",
            "/home/user/arma3/!Workshop",
            "C:\\Games\\Arma 3",
        ]
    );
}

// ---------------------------------------------------------------------------------------------

fn without_marker(store: &LocalStore, root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = snapshot(root);
    files.remove(&store.legacy_import_marker_path());
    files
}

fn all_skipped(report: &ImportReport) {
    for source in &report.sources {
        for outcome in source.profiles.iter().chain(&source.repositories) {
            assert_eq!(outcome.outcome, ItemOutcome::SkippedExisting, "{outcome:?}");
        }
        assert_eq!(source.configuration, ItemOutcome::SkippedExisting);
        assert_eq!(source.preferences, ItemOutcome::SkippedExisting);
        assert_eq!(source.workshop, ItemOutcome::SkippedExisting);
        assert_eq!(source.migration, None);
    }
    assert!(!report.imported_anything());
}

#[test]
fn import_is_idempotent_and_marked() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let sources = per_user_sources(&java, &[]);
    let rust = tmp.path().join("rust");
    let store = store(&rust);

    assert!(should_offer_import(&store, &sources));
    assert!(!should_offer_import(&store, &[]));
    import(&store, &sources, &ImportOptions::default()).unwrap();
    assert!(!should_offer_import(&store, &sources));
    let marker = store.legacy_import_marker().unwrap();
    assert_eq!(
        marker.sources,
        [java
            .join("config/Arma3Sync/configuration")
            .display()
            .to_string()]
    );
    assert_eq!(marker.counts["repositories.imported"], 3);
    assert_eq!(marker.counts["repositories.skipped"], 1);
    assert_eq!(marker.counts["profiles.failed"], 0);
    assert!(marker.imported_at_ms > 0);
    let first = without_marker(&store, &rust);

    // Without `ignore_marker` nothing happens.
    let report = import(&store, &sources, &ImportOptions::default()).unwrap();
    assert!(report.sources.is_empty());

    let report = import(
        &store,
        &sources,
        &ImportOptions {
            ignore_marker: true,
        },
    )
    .unwrap();
    all_skipped(&report);
    assert_eq!(without_marker(&store, &rust), first);
}

#[test]
fn existing_rust_data_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    let mut mine = read_repository_file(&java.join("data/Arma3Sync/repositories").join(HTTP_FILE))
        .unwrap()
        .value
        .repository;
    mine.revision = 99;
    store.write_repository(&mine).unwrap();
    let mut my_profile = Profile::new("Event Night");
    my_profile.additional_parameters = Some("mine".to_owned());
    store.write_profile(&my_profile).unwrap();
    store.workshop().save("@ace", "1", "My ACE").unwrap();
    let rust_before = std::fs::read(store.repository_path("Sample HTTP")).unwrap();

    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    let source = &report.sources[0];
    assert!(
        source
            .repositories
            .iter()
            .any(|o| o.file == HTTP_FILE && o.outcome == ItemOutcome::SkippedExisting)
    );
    assert!(
        source
            .profiles
            .iter()
            .any(|o| o.file == "Event Night.a3s.profile"
                && o.outcome == ItemOutcome::SkippedExisting)
    );
    assert_eq!(source.workshop, ItemOutcome::SkippedExisting);
    assert_eq!(
        std::fs::read(store.repository_path("Sample HTTP")).unwrap(),
        rust_before
    );
    assert_eq!(
        store
            .read_sha1_cache("Sample HTTP", CacheKind::Sync)
            .entries
            .len(),
        0
    );
    assert_eq!(
        store.load_profiles().unwrap().profiles["Event Night"],
        my_profile
    );
    assert_eq!(store.workshop().find("@ace").unwrap().name, "My ACE");
}

#[test]
fn two_sources_first_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let install = tmp.path().join("install");
    copy_tree(&fixture("portable"), &install);
    sealed_repository(
        "plain/Sample_SFTP.repository.gz",
        &install.join("resources/ftp/Portable_Only.a3s.repository"),
        |g| set_root_string(g, "name", "Portable Only"),
    );
    let sources = per_user_sources(&java, std::slice::from_ref(&install));
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[1], inspect_java_installation(&install));
    let store = store(&tmp.path().join("rust"));

    let report = import(&store, &sources, &ImportOptions::default()).unwrap();
    assert_eq!(report.sources[0].configuration, ItemOutcome::Imported);
    assert_eq!(
        report.sources[1].configuration,
        ItemOutcome::SkippedExisting
    );
    assert_eq!(report.sources[1].preferences, ItemOutcome::SkippedExisting);
    assert_eq!(
        outcomes(&report.sources[1].repositories),
        [
            ("Portable_Only.a3s.repository", ItemOutcome::Imported),
            (FTP_FILE, ItemOutcome::SkippedExisting),
            (HTTP_FILE, ItemOutcome::SkippedExisting),
            (SFTP_FILE, ItemOutcome::SkippedExisting),
        ]
    );
    let names: Vec<_> = store
        .load_repositories()
        .unwrap()
        .repositories
        .into_keys()
        .collect();
    assert_eq!(
        names,
        ["Portable Only", "Sample FTP", "Sample HTTP", "Sample SFTP"]
    );
    // The configuration of the first source (active profile "Default").
    assert_eq!(store.read_configuration().unwrap().profile_name, "Default");
}

// ---------------------------------------------------------------------------------------------

/// A per-user layout without a Java `Default` profile, with broken files.
fn broken_layout(root: &Path, active: &str) -> PathBuf {
    let java = root.join("java");
    let configuration = java.join("config/Arma3Sync/configuration");
    let profiles = java.join("config/Arma3Sync/profiles");
    let repositories = java.join("data/Arma3Sync/repositories");
    for dir in [&configuration, &profiles, &repositories] {
        std::fs::create_dir_all(dir).unwrap();
    }
    rewrite(
        &fixture("peruser/config/Arma3Sync/configuration/a3s.cfg"),
        &configuration.join("a3s.cfg"),
        |g| set_root_string(g, "profileName", active),
    );
    std::fs::copy(
        fixture("peruser/config/Arma3Sync/profiles/Event Night.a3s.profile"),
        profiles.join("Event Night.a3s.profile"),
    )
    .unwrap();
    std::fs::write(profiles.join("Broken.a3s.profile"), b"not gzip at all").unwrap();

    let good = fixture(&format!("peruser/data/Arma3Sync/repositories/{HTTP_FILE}"));
    std::fs::copy(&good, repositories.join(HTTP_FILE)).unwrap();
    rewrite(
        &good,
        &repositories.join("BadPadding.a3s.repository"),
        |g| {
            let root = g.root_value().unwrap().as_ref().unwrap();
            let content = g.field(root, "encryptedContent").unwrap().as_ref().unwrap();
            if let Node::Array(array) = g.node_mut(content).unwrap()
                && let ArrayData::Byte(bytes) = &mut array.data
            {
                let last = bytes.len() - 1;
                bytes[last] ^= 0x5A;
            }
        },
    );
    sealed_repository(
        "plain/Sample_FTP.repository.gz",
        &repositories.join("Blank.a3s.repository"),
        |g| set_root_string(g, "name", "  "),
    );
    let mut b = a3s_jserial::jdk::Builder::new();
    let root = b.string("not a sealed object");
    let graph = b.finish(root);
    std::fs::write(
        repositories.join("WrongRoot.a3s.repository"),
        gzip(&a3s_jserial::write(&graph).unwrap()),
    )
    .unwrap();
    java
}

#[test]
fn failures_are_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let java = broken_layout(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    let source = &report.sources[0];

    let failed = |list: &[NamedOutcome], file: &str| -> String {
        match &list.iter().find(|o| o.file == file).unwrap().outcome {
            ItemOutcome::Failed(message) => message.clone(),
            other => panic!("{file}: {other:?}"),
        }
    };
    assert!(
        failed(&source.profiles, "Broken.a3s.profile")
            .starts_with("Failed to read file: Broken.a3s.profile\n")
    );
    assert!(
        failed(&source.repositories, "BadPadding.a3s.repository")
            .starts_with("Failed to decrypt serialized file: BadPadding.a3s.repository")
    );
    assert_eq!(
        failed(&source.repositories, "Blank.a3s.repository"),
        "Repository name is missing."
    );
    assert!(
        failed(&source.repositories, "WrongRoot.a3s.repository")
            .starts_with("Failed to decrypt serialized file: WrongRoot.a3s.repository")
    );
    assert_eq!(
        outcomes(&source.repositories)
            .into_iter()
            .filter(|(_, o)| *o == ItemOutcome::Imported)
            .collect::<Vec<_>>(),
        [(HTTP_FILE, ItemOutcome::Imported)]
    );
    assert_eq!(source.configuration, ItemOutcome::Imported);
    assert!(store.legacy_import_marker().is_some());
    assert_eq!(
        store.legacy_import_marker().unwrap().counts["repositories.failed"],
        3
    );

    // No Java `Default` profile: one is created and carries the migrated settings.
    let profiles = store.load_profiles().unwrap();
    assert!(store.profile_path("Default").exists());
    let default = &profiles.profiles["Default"];
    assert_eq!(default.addon_search_directories.len(), 2);
    assert_eq!(default.launcher_options.cpu_count_selection, 6);
    assert_eq!(default.launcher_options.steam_exe_path, None);
    assert!(
        source.migration.as_deref().unwrap().contains("(created)"),
        "{:?}",
        source.migration
    );
}

#[test]
fn unknown_active_profile_falls_back_to_default() {
    let tmp = tempfile::tempdir().unwrap();
    let java = broken_layout(tmp.path(), "Ghost");
    let store = store(&tmp.path().join("rust"));
    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    // Java resets an unknown active profile to "Default" first (MainPanel.java:481-490).
    let migration = report.sources[0].migration.clone().unwrap();
    assert_eq!(
        migration,
        "Migrated 2 search directories and the launcher options of a3s.cfg into profile \
         \"Default\" (created) because the active profile \"Ghost\" does not exist."
    );
    assert!(!store.profile_path("Ghost").exists());
    assert_eq!(store.read_configuration().unwrap().profile_name, "Default");
    let profiles = store.load_profiles().unwrap();
    assert_eq!(
        profiles.profiles["Default"].addon_search_directories.len(),
        2
    );
    assert_eq!(
        profiles.profiles["Default"]
            .launcher_options
            .cpu_count_selection,
        6
    );
    // The imported profile is untouched.
    let java_night =
        read_profile_file(&java.join("config/Arma3Sync/profiles/Event Night.a3s.profile"))
            .unwrap()
            .value;
    assert_eq!(profiles.profiles["Event Night"], java_night);
}

#[test]
fn unreadable_rust_files_still_block_their_names() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let rust = tmp.path().join("rust");
    let store = store(&rust);
    // Files of a newer store format under non-canonical names, and one that is not TOML.
    let profiles = rust.join("config/profiles");
    let repositories = rust.join("data/repositories");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::create_dir_all(&repositories).unwrap();
    std::fs::write(
        profiles.join("newer.toml"),
        "format = 2\n[profile]\nname = \"Event Night\"\n",
    )
    .unwrap();
    std::fs::write(
        repositories.join("newer.toml"),
        "format = 2\n[repository]\nname = \"Sample HTTP\"\n",
    )
    .unwrap();
    std::fs::write(profiles.join("broken.toml"), "[[").unwrap();

    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    let source = &report.sources[0];
    assert!(
        source
            .profiles
            .iter()
            .any(|o| o.file == "Event Night.a3s.profile"
                && o.outcome == ItemOutcome::SkippedExisting)
    );
    assert!(
        source
            .repositories
            .iter()
            .any(|o| o.file == HTTP_FILE && o.outcome == ItemOutcome::SkippedExisting)
    );
    assert!(!store.profile_path("Event Night").exists());
    assert!(!store.repository_path("Sample HTTP").exists());
    // The file whose name cannot be read is reported.
    assert!(
        report.warnings.iter().any(|w| w.contains("broken.toml")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn existing_rust_default_is_not_overwritten_by_the_migration() {
    let tmp = tempfile::tempdir().unwrap();
    let java = broken_layout(tmp.path(), "Ghost");
    let store = store(&tmp.path().join("rust"));
    let mut mine = Profile::new("Default");
    mine.additional_parameters = Some("mine".to_owned());
    store.write_profile(&mine).unwrap();
    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    let migration = report.sources[0].migration.clone().unwrap();
    assert!(migration.contains("not migrated"), "{migration}");
    assert!(migration.contains("\"Default\""), "{migration}");
    assert_eq!(store.load_profiles().unwrap().profiles["Default"], mine);
}

// ---------------------------------------------------------------------------------------------

#[test]
fn historical_variants_through_the_importer() {
    let tmp = tempfile::tempdir().unwrap();
    let java = tmp.path().join("java");
    let configuration = java.join("config/Arma3Sync/configuration");
    let profiles = java.join("config/Arma3Sync/profiles");
    let repositories = java.join("data/Arma3Sync/repositories");
    for dir in [&configuration, &profiles, &repositories] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::copy(
        fixture("variants/cfg_with_aia.a3s.cfg"),
        configuration.join("a3s.cfg"),
    )
    .unwrap();
    std::fs::copy(
        fixture("variants/prefs_pre_github.a3s.prefs"),
        configuration.join("a3s.prefs"),
    )
    .unwrap();
    std::fs::copy(
        fixture("variants/repository_pre_connections.a3s.repository"),
        repositories.join("Old.a3s.repository"),
    )
    .unwrap();
    // A profile with an enum constant this client does not know: decoded with a warning.
    rewrite(
        &fixture("variants/treeleaf_pre_workshop.a3s.profile"),
        &profiles.join("Event Night.a3s.profile"),
        |g| {
            let constant = g
                .nodes
                .iter()
                .find_map(|n| match n {
                    Node::Enum(e) if g.string(e.constant).ok() == Some("EVENT") => Some(e.constant),
                    _ => None,
                })
                .unwrap();
            *g.node_mut(constant).unwrap() = Node::String("FUTURE".to_owned());
        },
    );

    let store = store(&tmp.path().join("rust"));
    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    let source = &report.sources[0];
    assert_eq!(source.configuration, ItemOutcome::Imported);
    assert_eq!(source.preferences, ItemOutcome::Imported);
    assert_eq!(
        outcomes(&source.repositories),
        [("Old.a3s.repository", ItemOutcome::Imported)]
    );
    assert_eq!(
        outcomes(&source.profiles),
        [("Event Night.a3s.profile", ItemOutcome::Imported)]
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("Event Night.a3s.profile: ") && w.contains("FUTURE")),
        "{:?}",
        report.warnings
    );

    let repository = &store.load_repositories().unwrap().repositories["Sample FTP"].repository;
    assert_eq!(repository.parallel_upload_connections, 0);
    assert_eq!(repository.number_of_repository_check_connections, 0);
    assert_eq!(repository.parallel_upload_connections(), 4);
    assert_eq!(repository.repository_check_connections(), 4);
    assert!(store.read_preferences().unwrap().prefer_github_updates);
    assert_eq!(store.read_configuration().unwrap().height, 900);
}

// ---------------------------------------------------------------------------------------------

#[test]
fn unreadable_marker_is_not_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let sources = per_user_sources(&java, &[]);
    for (i, marker) in ["format = 2\n[import]\n", "[import\n"]
        .into_iter()
        .enumerate()
    {
        let store = store(&tmp.path().join(format!("rust{i}")));
        std::fs::create_dir_all(&store.dirs().config).unwrap();
        std::fs::write(store.legacy_import_marker_path(), marker).unwrap();
        assert!(store.read_legacy_import_marker().is_err());
        assert!(!should_offer_import(&store, &sources));

        // Not offered, nothing done.
        let report = import(&store, &sources, &ImportOptions::default()).unwrap();
        assert!(report.sources.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert!(!store.repository_path("Sample HTTP").exists());

        // An explicit re-import runs but leaves the marker alone.
        let report = import(
            &store,
            &sources,
            &ImportOptions {
                ignore_marker: true,
            },
        )
        .unwrap();
        assert!(!report.sources.is_empty());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.starts_with("The import marker cannot be read")),
            "{:?}",
            report.warnings
        );
        assert_eq!(
            std::fs::read_to_string(store.legacy_import_marker_path()).unwrap(),
            marker
        );
    }
}

#[test]
fn unreadable_rust_workshop_file_is_reported_not_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    let mine = "format = 2\n[workshop.mods.\"@mine\"]\npublished_id = \"42\"\nname = \"Mine\"\n";
    std::fs::write(store.workshop_path(), mine).unwrap();
    let report = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    )
    .unwrap();
    assert!(matches!(
        &report.sources[0].workshop,
        ItemOutcome::Failed(message) if message.starts_with("Not merged")
    ));
    assert_eq!(
        std::fs::read_to_string(store.workshop_path()).unwrap(),
        mine
    );
    // The rest is imported and the marker written.
    assert!(store.legacy_import_marker().is_some());
}

#[cfg(unix)]
#[test]
fn caches_are_written_before_the_repository() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let java = java_per_user(tmp.path(), "Default");
    let store = store(&tmp.path().join("rust"));
    let caches = &store.dirs().sha1_cache;
    std::fs::create_dir_all(caches).unwrap();
    std::fs::set_permissions(caches, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = import(
        &store,
        &per_user_sources(&java, &[]),
        &ImportOptions::default(),
    );
    std::fs::set_permissions(caches, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root ignores directory permissions; there is nothing to check then.
    if result.is_err() {
        // The first repository's cache failed: no repository file without its caches.
        assert!(store.load_repositories().unwrap().repositories.is_empty());
        assert!(store.legacy_import_marker().is_none());
    }
}
