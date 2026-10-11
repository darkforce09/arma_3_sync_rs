//! The local TOML store: envelope, atomic writes, profiles, repositories and SHA-1 caches
//!.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use a3s_error::UserFacing as _;
use a3s_hash::{FileAttributes, Sha1Cache};
use a3s_repo_files::local::{CacheKind, Error, LocalDirs, LocalStore};
use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, StartWithOs,
};
use a3s_types::local::{
    Acre2Options, AcreOptions, BikeyExtractOptions, Configuration, ExternalApplication,
    Preferences, Profile, Proxy, Repository, RptOptions, TfarOptions, TreeDirectory, TreeLeaf,
    TreeNode,
};
use a3s_types::{FavoriteServer, Protocol, ProtocolType};

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

fn full_configuration() -> Configuration {
    Configuration {
        height: 900,
        width: 1400,
        profile_name: "Main".to_owned(),
        game_version: "Arma 3".to_owned(),
        server_name: Some("WOG #1".to_owned()),
        default_modset: Some("Repo".to_owned()),
        view_mode_tree: false,
        favorite_servers: vec![FavoriteServer {
            name: Some("Alpha".to_owned()),
            ip_address: Some("192.0.2.10".to_owned()),
            port: 2302,
            password: Some("secret".to_owned()),
            selected: true,
            modset_name: Some("Event".to_owned()),
            repository_name: Some("Repo".to_owned()),
        }],
        external_applications: vec![ExternalApplication {
            name: Some("TS3".to_owned()),
            executable_path: Some("/opt/ts3".to_owned()),
            parameters: Some("-x".to_owned()),
            enable: true,
        }],
        acre_options: AcreOptions {
            ts3_path: Some("/ts3".to_owned()),
            acre_plugin_path: Some("/acre".to_owned()),
            acre_userconfig_path: Some("/acre/uc".to_owned()),
        },
        acre2_options: Acre2Options {
            ts3_path: Some("/ts3".to_owned()),
            acre2_plugin_path: Some("/acre2".to_owned()),
        },
        tfar_options: TfarOptions {
            ts3_path: Some("/ts3".to_owned()),
            tfar_plugin_path: Some("/tfar".to_owned()),
            tfar_userconfig_path: Some("/tfar/uc".to_owned()),
        },
        rpt_options: RptOptions {
            rpt_path: Some("/rpt".to_owned()),
        },
        bikey_extract_options: BikeyExtractOptions {
            source_directory_path: Some("/src".to_owned()),
            target_directory_path: Some("/keys".to_owned()),
        },
        proxy: Proxy {
            enable_proxy: true,
            proxy_protocol: Some(Protocol::new(
                ProtocolType::Socks5,
                "proxy.example.org",
                "1080",
                "user",
                "pass",
                false,
            )),
        },
    }
}

fn full_preferences() -> Preferences {
    Preferences {
        launch_panel_game_launch: MinimizationType::Tray,
        launch_panel_minimized: MinimizationType::Close,
        look_and_feel: LookAndFeel::LafGraphite,
        icon_resize_size: IconResize::Size3,
        start_with_os: StartWithOs::Enabled,
        check_repositories_frequency: CheckRepositoriesFrequency::Freq1,
        prefer_github_updates: false,
    }
}

fn repository(name: &str) -> Repository {
    let mut r = Repository::new(
        name,
        Protocol::new(
            ProtocolType::Http,
            "repo.example/x",
            "80",
            "anonymous",
            "",
            true,
        ),
    );
    r.revision = 3;
    r.parallel_upload_connections = 0;
    r
}

// ---------------------------------------------------------------------------------------------

#[test]
fn configuration_and_preferences_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let configuration = full_configuration();
    let preferences = full_preferences();
    assert_ne!(configuration, Configuration::default());
    assert_ne!(preferences, Preferences::default());

    store.write_configuration(&configuration).unwrap();
    store.write_preferences(&preferences).unwrap();
    assert_eq!(store.read_configuration().unwrap(), configuration);
    assert_eq!(store.read_preferences().unwrap(), preferences);

    let text = std::fs::read_to_string(store.configuration_path()).unwrap();
    assert!(text.starts_with("format = 1\n"), "{text}");
    assert!(text.contains("[configuration]"), "{text}");
}

#[test]
fn missing_files_read_as_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir.path().join("does/not/exist"));
    assert_eq!(
        store.read_configuration().unwrap(),
        Configuration::default()
    );
    assert_eq!(store.read_preferences().unwrap(), Preferences::default());
    // Reading creates nothing.
    assert!(!dir.path().join("does").exists());
}

#[test]
fn newer_format_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    std::fs::write(store.preferences_path(), "format = 2\n[preferences]\n").unwrap();
    match store.read_preferences() {
        Err(Error::UnsupportedFormat {
            found, supported, ..
        }) => assert_eq!((found, supported), (2, 1)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn garbage_is_a_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    std::fs::write(store.configuration_path(), "this is = = not toml").unwrap();
    let err = store.read_configuration().unwrap_err();
    assert!(matches!(err, Error::Parse { .. }), "{err:?}");
}

#[test]
fn unknown_keys_and_missing_format_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    std::fs::write(
        store.preferences_path(),
        "future = true\n[preferences]\nlook_and_feel = \"LafHifi\"\nnew_field = 3\n",
    )
    .unwrap();
    let preferences = store.read_preferences().unwrap();
    assert_eq!(preferences.look_and_feel, LookAndFeel::LafHifi);
    assert_eq!(
        preferences.check_repositories_frequency,
        Preferences::default().check_repositories_frequency
    );
}

#[cfg(unix)]
#[test]
fn files_are_private_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_configuration(&full_configuration()).unwrap();
    store.write_repository(&repository("WOG")).unwrap();
    for path in [store.configuration_path(), store.repository_path("WOG")] {
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{}", path.display());
    }
}

#[cfg(unix)]
#[test]
fn failed_write_leaves_the_old_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_preferences(&Preferences::default()).unwrap();
    let before = std::fs::read(store.preferences_path()).unwrap();

    let config = &store.dirs().config;
    std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = store.write_preferences(&full_preferences());
    std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Root ignores directory permissions; there is nothing to check then.
    if result.is_err() {
        assert_eq!(std::fs::read(store.preferences_path()).unwrap(), before);
        let leftovers: Vec<_> = std::fs::read_dir(config)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "preferences.toml")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}

// ---------------------------------------------------------------------------------------------

fn nested_profile(name: &str) -> Profile {
    let mut group = TreeDirectory::new("Group");
    group.add_child(TreeNode::Leaf(TreeLeaf::new("@cba")));
    let mut profile = Profile::new(name);
    profile.tree.add_child(TreeNode::Directory(group));
    profile.addon_search_directories = vec!["/a".to_owned()];
    profile
}

#[test]
fn profiles_load_with_an_in_memory_default() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let main = nested_profile("Main");
    let other = Profile::new("Other/With:Chars");
    store.write_profile(&main).unwrap();
    store.write_profile(&other).unwrap();

    let load = store.load_profiles().unwrap();
    assert!(load.failures.is_empty());
    assert_eq!(load.error_message(), None);
    assert_eq!(load.profiles.len(), 3);
    assert_eq!(load.profiles["Main"], main);
    assert_eq!(load.profiles["Other/With:Chars"], other);
    assert_eq!(load.profiles["Default"], Profile::new("Default"));
    // The default profile is not written.
    assert!(!store.profile_path("Default").exists());
    assert!(!load.sources.contains_key("Default"));
}

#[test]
fn corrupt_profile_is_collected() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_profile(&Profile::new("Main")).unwrap();
    std::fs::write(store.dirs().profiles.join("x.toml"), "not = [valid").unwrap();
    std::fs::write(store.dirs().profiles.join("ignored.txt"), "garbage").unwrap();

    let load = store.load_profiles().unwrap();
    assert!(load.profiles.contains_key("Main"));
    assert_eq!(load.failures.len(), 1);
    let detail = load.failures["x.toml"].detail();
    assert_eq!(
        load.error_message().unwrap(),
        format!("Failed to load profiles:\n - x.toml: {detail}")
    );
    assert!(detail.starts_with("Failed to read file: "), "{detail}");
}

#[test]
fn duplicate_profile_names_newest_wins() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let mut old = Profile::new("Main");
    old.additional_parameters = Some("old".to_owned());
    let mut new = Profile::new("Main");
    new.additional_parameters = Some("new".to_owned());
    std::fs::create_dir_all(&store.dirs().profiles).unwrap();
    let a = store.dirs().profiles.join("a.toml");
    let b = store.dirs().profiles.join("b.toml");
    let text = |p: &Profile| {
        format!(
            "format = 1\n[profile]\nname = \"Main\"\nadditional_parameters = \"{}\"\n",
            p.additional_parameters.as_deref().unwrap()
        )
    };
    std::fs::write(&a, text(&new)).unwrap();
    std::fs::write(&b, text(&old)).unwrap();
    set_mtime(&a, 2_000);
    set_mtime(&b, 1_000);

    let load = store.load_profiles().unwrap();
    assert_eq!(load.profiles["Main"], new);
    assert_eq!(load.sources["Main"], a);
    assert_eq!(
        load.warnings,
        vec!["Main: ignored \"b.toml\" because \"a.toml\" is newer.".to_owned()]
    );

    set_mtime(&b, 3_000);
    let load = store.load_profiles().unwrap();
    assert_eq!(load.profiles["Main"], old);
    assert_eq!(
        load.warnings,
        vec!["Main: replaced \"a.toml\" with \"b.toml\" because it is newer.".to_owned()]
    );
}

#[test]
fn delete_profile() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_profile(&Profile::new("Main")).unwrap();
    assert!(store.delete_profile("Main").unwrap());
    assert!(!store.delete_profile("Main").unwrap());
}

// ---------------------------------------------------------------------------------------------

fn write_repository_file(store: &LocalStore, file: &str, name: &str, revision: i32) -> PathBuf {
    std::fs::create_dir_all(&store.dirs().repositories).unwrap();
    let path = store.dirs().repositories.join(file);
    std::fs::write(
        &path,
        format!(
            "format = 1\n[repository]\nname = \"{name}\"\nrevision = {revision}\n\
             [repository.protocol]\nprotocol_type = \"Http\"\nurl = \"x\"\nport = \"80\"\n\
             validate_ssl_certificate = true\n"
        ),
    )
    .unwrap();
    path
}

#[test]
fn repositories_load_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let wog = repository("WOG");
    let other = repository("Other Repo");
    store.write_repository(&wog).unwrap();
    store.write_repository(&other).unwrap();
    assert!(
        store
            .repository_path("WOG")
            .ends_with("WOG__fe52247a11ac.toml")
    );

    let load = store.load_repositories().unwrap();
    assert!(load.failures.is_empty());
    assert_eq!(load.error_message(), None);
    assert_eq!(load.repositories["WOG"].repository, wog);
    assert_eq!(
        load.repositories["WOG"].source,
        store.repository_path("WOG")
    );
    assert_eq!(load.repositories["Other Repo"].repository, other);
    // Raw values stay raw.
    assert_eq!(
        load.repositories["WOG"]
            .repository
            .parallel_upload_connections,
        0
    );
}

#[test]
fn blank_name_and_corrupt_files_are_failures_and_successes_survive() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_repository(&repository("WOG")).unwrap();
    write_repository_file(&store, "blank.toml", "  ", 1);
    std::fs::write(store.dirs().repositories.join("broken.toml"), "[[[").unwrap();

    let load = store.load_repositories().unwrap();
    assert_eq!(load.repositories.len(), 1);
    assert!(load.repositories.contains_key("WOG"));
    let broken = load.failures["broken.toml"].detail();
    assert_eq!(
        load.error_message().unwrap(),
        format!(
            "Failed to read repository files:\n - blank.toml: Repository name is missing.\n - \
             broken.toml: {broken}"
        )
    );
}

#[test]
fn duplicates_follow_java() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let a = write_repository_file(&store, "a.toml", "WOG", 1);
    let b = write_repository_file(&store, "b.toml", "WOG", 2);

    // Equal mtimes: the first file wins, with an "ignored" warning.
    set_mtime(&a, 1_000);
    set_mtime(&b, 1_000);
    let load = store.load_repositories().unwrap();
    assert_eq!(load.repositories["WOG"].repository.revision, 1);
    assert_eq!(
        load.duplicate_warnings,
        vec!["WOG: ignored \"b.toml\" because \"a.toml\" is newer.".to_owned()]
    );

    set_mtime(&b, 2_000);
    let load = store.load_repositories().unwrap();
    assert_eq!(load.repositories["WOG"].repository.revision, 2);
    assert_eq!(load.repositories["WOG"].source, b);
    assert_eq!(
        load.duplicate_warnings,
        vec!["WOG: replaced \"a.toml\" with \"b.toml\" because it is newer.".to_owned()]
    );
}

#[test]
fn write_replacing_deletes_the_previous_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let previous = write_repository_file(&store, "old-name.toml", "WOG", 1);
    let wog = repository("WOG");
    store
        .write_repository_replacing(&wog, Some(&previous))
        .unwrap();
    assert!(!previous.exists());
    assert!(store.repository_path("WOG").exists());

    // The target itself is never deleted.
    let target = store.repository_path("WOG");
    store
        .write_repository_replacing(&wog, Some(&target))
        .unwrap();
    assert!(target.exists());
}

#[test]
fn remove_repository_removes_the_caches() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    store.write_repository(&repository("WOG")).unwrap();
    let cache = cache_of(3);
    store
        .write_sha1_cache("WOG", CacheKind::Sync, &cache)
        .unwrap();
    store
        .write_sha1_cache("WOG", CacheKind::Build, &cache)
        .unwrap();

    assert!(store.remove_repository("WOG").unwrap());
    assert!(!store.repository_path("WOG").exists());
    assert!(!store.sha1_cache_path("WOG", CacheKind::Sync).exists());
    assert!(!store.sha1_cache_path("WOG", CacheKind::Build).exists());
    assert!(!store.remove_repository("WOG").unwrap());
}

// ---------------------------------------------------------------------------------------------

fn cache_of(n: i64) -> Sha1Cache {
    let mut cache = Sha1Cache::default();
    for i in 0..n {
        cache.entries.insert(
            PathBuf::from(format!("/games/@mod/addons/file {i}.pbo")),
            FileAttributes {
                sha1: format!("{i:040x}"),
                last_modified: 1_704_024_000_000 + i,
            },
        );
    }
    cache
}

#[test]
fn sha1_cache_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let cache = cache_of(3);
    store
        .write_sha1_cache("WOG", CacheKind::Sync, &cache)
        .unwrap();
    assert_eq!(store.read_sha1_cache("WOG", CacheKind::Sync), cache);
    assert_eq!(
        store.read_sha1_cache("WOG", CacheKind::Build),
        Sha1Cache::default()
    );
    let path = store.sha1_cache_path("WOG", CacheKind::Sync);
    assert!(path.ends_with("sha1_cache/WOG__fe52247a11ac.sync.toml"));
    // Deterministic output.
    let first = std::fs::read(&path).unwrap();
    store
        .write_sha1_cache("WOG", CacheKind::Sync, &cache)
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
}

#[test]
fn missing_or_corrupt_cache_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    assert_eq!(
        store.read_sha1_cache("Nope", CacheKind::Build),
        Sha1Cache::default()
    );
    let path = store.sha1_cache_path("WOG", CacheKind::Build);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "format = 1\n[cache.entries]\n\"/x\" = 3\n").unwrap();
    assert_eq!(
        store.read_sha1_cache("WOG", CacheKind::Build),
        Sha1Cache::default()
    );
}

#[test]
fn local_store_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<LocalStore>();
}

// ---------------------------------------------------------------------------------------------

#[test]
fn errors_never_quote_file_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().repositories).unwrap();
    std::fs::create_dir_all(&store.dirs().profiles).unwrap();
    let repositories = &store.dirs().repositories;
    // An unterminated string: the parser's `Display` would quote the line.
    std::fs::write(
        repositories.join("broken.toml"),
        "format = 1\n[repository]\nname = \"WOG\"\n[repository.protocol]\npassword = \"secret1\n",
    )
    .unwrap();
    // A wrong type and an unknown variant: serde would quote the values.
    std::fs::write(
        repositories.join("typed.toml"),
        "format = 1\n[repository]\nname = \"X\"\nrevision = \"secret2\"\n",
    )
    .unwrap();
    std::fs::write(
        repositories.join("variant.toml"),
        "format = 1\n[repository]\nname = \"Y\"\n[repository.protocol]\nprotocol_type = \
         \"secret3\"\nurl = \"u\"\nport = \"1\"\nvalidate_ssl_certificate = true\n",
    )
    .unwrap();
    std::fs::write(
        store.dirs().profiles.join("p.toml"),
        "format = 1\n[profile]\nname = 'secret4\n",
    )
    .unwrap();
    std::fs::write(
        store.configuration_path(),
        "format = 1\n[configuration]\nheight = \"secret5\"\n",
    )
    .unwrap();

    let load = store.load_repositories().unwrap();
    assert_eq!(load.failures.len(), 3);
    for failure in load.failures.values() {
        for text in [
            failure.detail(),
            failure.user_message(),
            format!("{failure:?}"),
        ] {
            assert!(!text.contains("secret"), "{text}");
        }
    }
    let message = load.error_message().unwrap();
    assert!(!message.contains("secret"), "{message}");
    // The location is still reported.
    assert!(message.contains("line 5"), "{message}");

    let message = store.load_profiles().unwrap().error_message().unwrap();
    assert!(!message.contains("secret"), "{message}");

    let err = store.read_configuration().unwrap_err();
    assert!(!err.detail().contains("secret"), "{}", err.detail());
    assert!(err.detail().contains("invalid type"), "{}", err.detail());
}

#[test]
fn format_other_than_one_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    std::fs::create_dir_all(&store.dirs().config).unwrap();
    for found in [0, -1] {
        std::fs::write(
            store.preferences_path(),
            format!("format = {found}\n[preferences]\n"),
        )
        .unwrap();
        match store.read_preferences() {
            Err(Error::UnsupportedFormat { found: f, .. }) => assert_eq!(f, found),
            other => panic!("{found}: {other:?}"),
        }
    }
}

#[test]
fn directory_creation_failure_reports_the_target_file() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let store = store(&blocker);
    assert!(store.write_configuration(&full_configuration()).is_err());
    assert!(store.write_profile(&Profile::new("Main")).is_err());
}

#[test]
fn write_replacing_never_deletes_outside_the_repositories_dir() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let outside = dir.path().join("precious.toml");
    std::fs::write(&outside, "x").unwrap();
    let wog = repository("WOG");
    store
        .write_repository_replacing(&wog, Some(&outside))
        .unwrap();
    assert!(outside.exists());

    let not_toml = store.dirs().repositories.join("notes.txt");
    std::fs::write(&not_toml, "x").unwrap();
    store
        .write_repository_replacing(&wog, Some(&not_toml))
        .unwrap();
    assert!(not_toml.exists());

    // A path that leaves the directory through `..` is resolved first.
    let sneaky = store.dirs().repositories.join("../../precious.toml");
    store
        .write_repository_replacing(&wog, Some(&sneaky))
        .unwrap();
    assert!(outside.exists());
}
