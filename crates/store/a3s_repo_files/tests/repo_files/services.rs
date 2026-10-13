//! `services::ClientData`: start-up load, persistence of every operation (checked by reloading
//! from disk), rollback on failure, and autoconfig export/import.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_repo_files::hash::{FileAttributes, Sha1Cache};
use a3s_repo_files::local::{CacheKind, LocalDirs, LocalStore};
use a3s_repo_files::services::{
    ClientData, FavoriteServerEdit, ProtocolInput, ServiceError, duplicate_profile_name,
};
use a3s_types::constants::{CheckRepositoriesFrequency, LookAndFeel, ModsetType};
use a3s_types::local::{
    Configuration, ExternalApplication, LauncherOptions, Preferences, Profile, ProtocolCheckError,
    Repository, TreeDirectory, TreeLeaf, TreeNode,
};
use a3s_types::{FavoriteServer, Protocol, ProtocolType};

fn dirs(root: &Path) -> LocalDirs {
    LocalDirs {
        config: root.join("config"),
        profiles: root.join("config/profiles"),
        repositories: root.join("data/repositories"),
        sha1_cache: root.join("data/sha1_cache"),
    }
}

fn store(root: &Path) -> Arc<LocalStore> {
    Arc::new(LocalStore::with_dirs(dirs(root)))
}

/// Loads the data, failing on any warning.
fn load(root: &Path) -> ClientData {
    let outcome = ClientData::load(store(root));
    assert_eq!(outcome.warnings, Vec::<String>::new());
    outcome.data
}

fn input(protocol_type: ProtocolType, url: &str, port: &str, login: &str) -> ProtocolInput {
    ProtocolInput {
        protocol_type,
        url: url.to_owned(),
        port: port.to_owned(),
        login: login.to_owned(),
        password: "pw".to_owned(),
        validate_ssl_certificate: true,
    }
}

fn http(url: &str) -> ProtocolInput {
    input(ProtocolType::Http, url, "80", "anonymous")
}

fn server(name: Option<&str>) -> FavoriteServer {
    FavoriteServer {
        name: name.map(str::to_owned),
        ip_address: Some("192.0.2.1".to_owned()),
        port: 2302,
        ..FavoriteServer::default()
    }
}

fn names(servers: &[FavoriteServer]) -> Vec<Option<&str>> {
    servers.iter().map(|s| s.name.as_deref()).collect()
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

/// The bytes and modification time of `path`, to prove that nothing was written.
fn fingerprint(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
    (
        std::fs::read(path).unwrap(),
        std::fs::metadata(path).unwrap().modified().unwrap(),
    )
}

// ---------------------------------------------------------------------------------------------
// Start-up

#[test]
fn empty_store_loads_defaults_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let data = load(tmp.path());
    assert_eq!(data.configuration(), &Configuration::default());
    assert_eq!(data.preferences(), &Preferences::default());
    assert_eq!(data.profile_names(), ["Default"]);
    assert_eq!(data.repositories().count(), 0);
    assert!(!tmp.path().join("config").exists());
}

#[test]
fn unknown_active_profile_is_reset_and_written() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    store
        .write_configuration(&Configuration {
            profile_name: "Ghost".to_owned(),
            ..Configuration::default()
        })
        .unwrap();
    let data = load(tmp.path());
    assert_eq!(data.active_profile_name(), "Default");
    assert_eq!(store.read_configuration().unwrap().profile_name, "Default");
}

#[test]
fn unreadable_files_become_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    std::fs::create_dir_all(tmp.path().join("config/profiles")).unwrap();
    std::fs::write(store.configuration_path(), "not toml [").unwrap();
    store.write_profile(&Profile::new("Good")).unwrap();
    std::fs::write(tmp.path().join("config/profiles/bad.toml"), "[[").unwrap();
    store
        .write_repository(&Repository::new(
            "R",
            Protocol::new(ProtocolType::Http, "h", "80", "anonymous", "", true),
        ))
        .unwrap();
    std::fs::write(tmp.path().join("data/repositories/bad.toml"), "[[").unwrap();

    let outcome = ClientData::load(store);
    assert_eq!(outcome.warnings.len(), 3, "{:?}", outcome.warnings);
    assert_eq!(outcome.data.configuration(), &Configuration::default());
    assert_eq!(outcome.data.profile_names(), ["Default", "Good"]);
    assert_eq!(outcome.data.repository_names(), ["R"]);
}

#[test]
fn preferences_and_sorted_servers_are_loaded() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let preferences = Preferences {
        look_and_feel: LookAndFeel::LafHifi,
        check_repositories_frequency: CheckRepositoriesFrequency::Freq2,
        ..Preferences::default()
    };
    store.write_preferences(&preferences).unwrap();
    store
        .write_configuration(&Configuration {
            favorite_servers: vec![server(Some("b")), server(Some("A")), server(None)],
            ..Configuration::default()
        })
        .unwrap();
    let data = load(tmp.path());
    assert_eq!(data.preferences(), &preferences);
    assert_eq!(names(data.favorite_servers()), [None, Some("A"), Some("b")]);
}

#[test]
fn save_on_exit_writes_configuration_preferences_and_active_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.save_on_exit(600, 800).unwrap();
    let store = data.store().clone();
    let configuration = store.read_configuration().unwrap();
    assert_eq!((configuration.height, configuration.width), (600, 800));
    assert!(store.preferences_path().exists());
    assert!(store.profile_path("Default").exists());
}

// ---------------------------------------------------------------------------------------------
// Configuration and preferences

#[test]
fn configuration_setters_persist() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    let s = |v: &str| Some(v.to_owned());
    data.set_server_name(s("server")).unwrap();
    data.set_default_modset(s("modset")).unwrap();
    data.set_game_version("Arma 3 Test".to_owned()).unwrap();
    data.set_view_mode_tree(false).unwrap();
    data.set_acre2_ts3_path(s("a")).unwrap();
    data.set_acre2_plugin_path(s("b")).unwrap();
    data.set_tfar_ts3_path(s("c")).unwrap();
    data.set_tfar_plugin_path(s("d")).unwrap();
    data.set_tfar_userconfig_path(s("")).unwrap();
    data.set_rpt_path(s("f")).unwrap();
    data.set_bikey_source_directory(s("g")).unwrap();
    data.set_bikey_target_directory(s("h")).unwrap();
    data.set_window_size(10, 20).unwrap();
    data.create_profile("Other").unwrap();
    data.set_active_profile("Other").unwrap();

    let c = data.configuration();
    assert_eq!(c.server_name.as_deref(), Some("server"));
    assert_eq!(c.default_modset.as_deref(), Some("modset"));
    assert_eq!(c.game_version, "Arma 3 Test");
    assert!(!c.view_mode_tree);
    assert_eq!(c.acre2_options.ts3_path.as_deref(), Some("a"));
    assert_eq!(c.acre2_options.acre2_plugin_path.as_deref(), Some("b"));
    assert_eq!(c.tfar_options.ts3_path.as_deref(), Some("c"));
    assert_eq!(c.tfar_options.tfar_plugin_path.as_deref(), Some("d"));
    assert_eq!(c.tfar_options.tfar_userconfig_path.as_deref(), Some(""));
    assert_eq!(c.rpt_options.rpt_path.as_deref(), Some("f"));
    assert_eq!(
        c.bikey_extract_options.source_directory_path.as_deref(),
        Some("g")
    );
    assert_eq!(
        c.bikey_extract_options.target_directory_path.as_deref(),
        Some("h")
    );
    assert_eq!((c.height, c.width), (10, 20));
    assert_eq!(c.profile_name, "Other");
    assert_eq!(load(tmp.path()).configuration(), data.configuration());
}

#[test]
fn active_profile_must_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.set_server_name(None).unwrap();
    let before = fingerprint(&data.store().configuration_path());
    assert!(matches!(
        data.set_active_profile("Nope"),
        Err(ServiceError::ProfileNotFound(_))
    ));
    assert!(matches!(
        data.update_configuration(|c| {
            c.profile_name = "Nope".to_owned();
            true
        }),
        Err(ServiceError::ProfileNotFound(_))
    ));
    assert_eq!(data.active_profile_name(), "Default");
    assert_eq!(fingerprint(&data.store().configuration_path()), before);
}

#[test]
fn favorite_servers_stay_sorted() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    let mut selected = server(Some("b"));
    selected.selected = true;
    data.add_favorite_server(selected.clone()).unwrap();
    data.add_favorite_server(server(Some("a"))).unwrap();
    data.add_favorite_server(selected).unwrap();
    assert_eq!(
        names(data.favorite_servers()),
        [Some("a"), Some("b"), Some("b")]
    );
    assert!(data.favorite_servers().iter().all(|s| !s.selected));

    let before = fingerprint(&data.store().configuration_path());
    assert!(!data.delete_favorite_server(3).unwrap());
    assert!(
        !data
            .edit_favorite_server(3, FavoriteServerEdit::default())
            .unwrap()
    );
    assert_eq!(fingerprint(&data.store().configuration_path()), before);

    assert!(data.delete_favorite_server(1).unwrap());
    data.update_configuration(|c| {
        c.favorite_servers[0].selected = true;
        c.favorite_servers[0].repository_name = Some("R".to_owned());
        true
    })
    .unwrap();
    let edit = FavoriteServerEdit {
        name: Some("z".to_owned()),
        ip_address: Some("ip".to_owned()),
        port: 1,
        password: Some("p".to_owned()),
        modset_name: Some("m".to_owned()),
    };
    assert!(data.edit_favorite_server(0, edit).unwrap());
    let edited = &data.favorite_servers()[1];
    assert_eq!(edited.name.as_deref(), Some("z"));
    assert_eq!(
        (
            edited.ip_address.as_deref(),
            edited.port,
            edited.password.as_deref()
        ),
        (Some("ip"), 1, Some("p"))
    );
    assert_eq!(edited.modset_name.as_deref(), Some("m"));
    assert!(edited.selected);
    assert_eq!(edited.repository_name.as_deref(), Some("R"));
    assert_eq!(load(tmp.path()).favorite_servers(), data.favorite_servers());
}

#[test]
fn external_applications_are_stably_sorted() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    let app = |name: Option<&str>| ExternalApplication {
        name: name.map(str::to_owned),
        ..ExternalApplication::default()
    };
    data.save_external_applications(vec![
        app(Some("b")),
        app(None),
        app(Some("B")),
        app(Some("a")),
    ])
    .unwrap();
    let names: Vec<_> = load(tmp.path())
        .configuration()
        .external_applications
        .iter()
        .map(|a| a.name.clone())
        .collect();
    let expected = [None, Some("a"), Some("b"), Some("B")].map(|n| n.map(str::to_owned));
    assert_eq!(names, expected);
}

#[test]
fn proxy_is_checked_before_it_is_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.set_proxy(
        Some(&input(ProtocolType::Http, "10.0.0.1", "3128", "u")),
        true,
    )
    .unwrap();
    let stored = data.proxy().clone();
    let protocol = stored.proxy_protocol.as_ref().unwrap();
    assert!(stored.enable_proxy);
    assert_eq!(protocol.connection_timeout.as_deref(), Some("0"));
    assert_eq!(protocol.read_timeout.as_deref(), Some("0"));
    assert_eq!(load(tmp.path()).proxy(), &stored);

    let failures = [
        (input(ProtocolType::A3s, "h", "1", "u"), None),
        (
            input(ProtocolType::Http, "h", "1", ""),
            Some(ProtocolCheckError::LoginEmpty),
        ),
        (
            input(ProtocolType::Http, "h", "x", "u"),
            Some(ProtocolCheckError::PortInvalid),
        ),
    ];
    for (bad, check) in failures {
        let result = data.set_proxy(Some(&bad), false);
        match check {
            None => assert!(matches!(result, Err(ServiceError::UnsupportedProtocol(_)))),
            Some(expected) => {
                assert!(matches!(result, Err(ServiceError::ProtocolCheck(e)) if e == expected));
            }
        }
        assert_eq!(data.proxy(), &stored);
        assert_eq!(load(tmp.path()).proxy(), &stored);
    }

    data.set_proxy(None, true).unwrap();
    assert!(!data.proxy().enable_proxy);
    assert_eq!(data.proxy().proxy_protocol, None);
    assert_eq!(load(tmp.path()).proxy(), data.proxy());
}

#[test]
fn preferences_persist() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    let preferences = Preferences {
        look_and_feel: LookAndFeel::LafHifi,
        ..Preferences::default()
    };
    data.set_preferences(preferences.clone()).unwrap();
    assert_eq!(load(tmp.path()).preferences(), &preferences);
    assert!(
        data.update_preferences(|p| {
            p.prefer_github_updates = false;
            true
        })
        .unwrap()
    );
    assert!(!load(tmp.path()).preferences().prefer_github_updates);
    assert!(!data.update_preferences(|_| false).unwrap());
}

// ---------------------------------------------------------------------------------------------
// Profiles

#[test]
fn profiles_are_created_once_and_listed_in_java_order() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_profile("A").unwrap();
    assert!(matches!(
        data.create_profile("A"),
        Err(ServiceError::ProfileExists(_))
    ));
    assert!(matches!(
        data.create_profile("  "),
        Err(ServiceError::EmptyProfileName)
    ));
    data.create_profile("\u{FF5E}").unwrap();
    data.create_profile("\u{1F600}").unwrap();
    // UTF-16 order puts the surrogate pair (U+D83D …) before U+FF5E; `str` order would not.
    assert_eq!(
        load(tmp.path()).profile_names(),
        ["A", "Default", "\u{1F600}", "\u{FF5E}"]
    );
}

fn rich_profile(name: &str) -> Profile {
    let mut profile = Profile::new(name);
    let mut dir1 = TreeDirectory::new("dir1");
    dir1.optional = true;
    dir1.marked = true;
    dir1.selected = true;
    dir1.modset_type = Some(ModsetType::Repository);
    dir1.modset_repository_name = Some("R".to_owned());
    let mut dir2 = TreeDirectory::new("dir2");
    dir2.add_child(TreeNode::Leaf(TreeLeaf {
        name: "@leaf".to_owned(),
        selected: true,
        optional: true,
        workshop_id: Some("1".to_owned()),
        dlc_app_id: Some("2".to_owned()),
        missing: false,
    }));
    // Unsorted on purpose: the copy re-sorts through `add_child`.
    profile.tree.children = vec![
        TreeNode::Leaf(TreeLeaf::new("@b")),
        TreeNode::Directory(dir2),
        TreeNode::Leaf(TreeLeaf::new("@a")),
        TreeNode::Directory(dir1),
    ];
    profile.addon_search_directories = vec!["/mods".to_owned()];
    profile.additional_parameters = Some("-x".to_owned());
    profile.addon_names_by_priority = vec!["@a".to_owned()];
    profile.last_added_addon_search_directory = Some("/mods".to_owned());
    let s = |v: &str| Some(v.to_owned());
    profile.launcher_options = LauncherOptions {
        game_profile: s("gp"),
        show_script_errors: true,
        no_pause: true,
        file_patching: true,
        window_mode: true,
        check_signatures: true,
        auto_restart: true,
        launch_arma3_as_administrator: true,
        mission_file: true,
        mission_file_path: s("m"),
        max_memory_selection: s("4096"),
        cpu_count_selection: 8,
        ex_threads_selection: s("7"),
        malloc_selection: s("tbb"),
        enable_ht: true,
        huge_pages: true,
        no_splash_screen: false,
        default_world: true,
        nologs: true,
        arma3_exe_path: s("arma3.exe"),
        steam_exe_path: s("steam.exe"),
        game_start: Default::default(),
    };
    profile
}

#[test]
fn duplicate_copies_groups_paths_and_launch_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let source = rich_profile("Src");
    let mut data = ClientData::from_parts(
        store(tmp.path()),
        Configuration::default(),
        Preferences::default(),
        vec![source.clone()],
        Vec::new(),
    );
    data.duplicate_profile("Src", "Copy").unwrap();
    let copy = load(tmp.path()).profile("Copy").unwrap().clone();

    let children: Vec<&str> = copy.tree.children.iter().map(TreeNode::name).collect();
    assert_eq!(children, ["dir1", "dir2", "@a", "@b"]);
    let TreeNode::Directory(dir1) = &copy.tree.children[0] else {
        panic!()
    };
    assert!(dir1.selected && !dir1.optional && !dir1.marked);
    assert_eq!(dir1.modset_type, Some(ModsetType::Repository));
    assert_eq!(dir1.modset_repository_name.as_deref(), Some("R"));
    let TreeNode::Directory(dir2) = &copy.tree.children[1] else {
        panic!()
    };
    let TreeNode::Directory(source_dir2) = &source.tree.children[1] else {
        panic!()
    };
    assert_eq!(dir2.children, source_dir2.children);

    assert_eq!(
        copy.addon_search_directories,
        source.addon_search_directories
    );
    assert_eq!(copy.additional_parameters, source.additional_parameters);
    assert!(copy.addon_names_by_priority.is_empty());
    assert_eq!(copy.last_added_addon_search_directory, None);
    let expected = LauncherOptions {
        steam_exe_path: None,
        mission_file: false,
        mission_file_path: None,
        ..source.launcher_options.clone()
    };
    assert_eq!(copy.launcher_options, expected);

    data.set_active_profile("Copy").unwrap();
    data.add_addon_search_directory("/more".to_owned()).unwrap();
    assert_eq!(
        data.profile("Src").unwrap().addon_search_directories,
        ["/mods"]
    );

    assert!(matches!(
        data.duplicate_profile("Src", "Copy"),
        Err(ServiceError::ProfileExists(_))
    ));
    let files = toml_files(&tmp.path().join("config/profiles"));
    data.duplicate_profile("Missing", "Other").unwrap();
    assert_eq!(toml_files(&tmp.path().join("config/profiles")), files);
    assert!(data.profile("Other").is_none());

    assert_eq!(
        duplicate_profile_name("A").as_deref(),
        Some("A - Duplicate")
    );
    assert_eq!(duplicate_profile_name("A - Duplicate"), None);
}

#[test]
fn rename_replaces_the_file_and_keeps_the_profile_active() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_profile("A").unwrap();
    data.set_active_profile("A").unwrap();
    data.rename_profile("A", "B").unwrap();
    let store = data.store().clone();
    assert_eq!(
        toml_files(&tmp.path().join("config/profiles")),
        [store.profile_path("B")]
    );
    let reloaded = load(tmp.path());
    assert_eq!(reloaded.active_profile_name(), "B");
    assert_eq!(reloaded.profile_names(), ["B", "Default"]);

    assert!(matches!(
        data.rename_profile("X", "Y"),
        Err(ServiceError::ProfileNotFound(_))
    ));
    assert!(matches!(
        data.rename_profile("Default", "Y"),
        Err(ServiceError::DefaultProfileRename)
    ));
    assert!(matches!(
        data.rename_profile("B", "Default"),
        Err(ServiceError::ProfileExists(_))
    ));
    assert!(matches!(
        data.rename_profile("B", " "),
        Err(ServiceError::EmptyProfileName)
    ));
}

#[test]
fn rename_removes_a_non_canonical_source_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    store.write_profile(&Profile::new("A")).unwrap();
    let odd = tmp.path().join("config/profiles/odd.toml");
    std::fs::rename(store.profile_path("A"), &odd).unwrap();
    let mut data = load(tmp.path());
    data.rename_profile("A", "B").unwrap();
    assert_eq!(
        toml_files(&tmp.path().join("config/profiles")),
        [store.profile_path("B")]
    );
}

#[test]
fn remove_falls_back_to_default() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_profile("A").unwrap();
    data.set_active_profile("A").unwrap();
    data.remove_profile("A").unwrap();
    assert!(!data.store().profile_path("A").exists());
    let reloaded = load(tmp.path());
    assert_eq!(reloaded.active_profile_name(), "Default");
    assert_eq!(reloaded.profile_names(), ["Default"]);
    assert!(matches!(
        data.remove_profile("Default"),
        Err(ServiceError::DefaultProfileRemoval)
    ));
    assert!(matches!(
        data.remove_profile("A"),
        Err(ServiceError::ProfileNotFound(_))
    ));
}

#[test]
fn active_profile_setters_persist() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.add_addon_search_directory("/a".to_owned()).unwrap();
    data.add_addon_search_directory("/b".to_owned()).unwrap();
    data.add_addon_search_directory("/a".to_owned()).unwrap();
    assert!(data.remove_addon_search_directory("/a").unwrap());
    assert_eq!(data.addon_search_directories(), ["/b", "/a"]);
    let path = data.store().profile_path("Default");
    let before = fingerprint(&path);
    assert!(!data.remove_addon_search_directory("/c").unwrap());
    assert!(!data.update_active_profile(|_| false).unwrap());
    assert_eq!(fingerprint(&path), before);

    data.set_additional_parameters(Some("-p".to_owned()))
        .unwrap();
    data.set_last_added_addon_search_directory(Some("/b".to_owned()))
        .unwrap();
    let options = rich_profile("x").launcher_options;
    data.update_launcher_options(|o| *o = options.clone())
        .unwrap();
    assert!(matches!(
        data.set_cpu_count(Some("abc")),
        Err(ServiceError::InvalidNumber(_))
    ));
    assert_eq!(data.launcher_options().unwrap().cpu_count_selection, 8);
    let reloaded = load(tmp.path());
    assert_eq!(reloaded.additional_parameters(), Some("-p"));
    assert_eq!(reloaded.last_added_addon_search_directory(), Some("/b"));
    assert_eq!(reloaded.addon_search_directories(), ["/b", "/a"]);
    assert_eq!(reloaded.launcher_options(), Some(&options));

    data.set_cpu_count(Some("+4")).unwrap();
    assert_eq!(
        load(tmp.path())
            .launcher_options()
            .unwrap()
            .cpu_count_selection,
        4
    );
    data.set_cpu_count(None).unwrap();
    assert_eq!(
        load(tmp.path())
            .launcher_options()
            .unwrap()
            .cpu_count_selection,
        0
    );

    assert!(matches!(
        data.update_active_profile(|p| {
            p.name = "Renamed".to_owned();
            p.additional_parameters = None;
            true
        }),
        Err(ServiceError::NameChange)
    ));
    assert_eq!(data.additional_parameters(), Some("-p"));
    assert!(data.profile("Renamed").is_none());
}

#[test]
fn getters_fall_back_without_an_active_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let data = ClientData::from_parts(
        store(tmp.path()),
        Configuration {
            profile_name: "Ghost".to_owned(),
            ..Configuration::default()
        },
        Preferences::default(),
        Vec::new(),
        Vec::new(),
    );
    assert!(data.active_profile().is_none());
    assert_eq!(data.additional_parameters(), None);
    assert!(data.addon_search_directories().is_empty());
    assert_eq!(data.last_added_addon_search_directory(), None);
    assert_eq!(data.launcher_options(), None);
}

// ---------------------------------------------------------------------------------------------
// Repositories

#[test]
fn repositories_are_created_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    for blank in ["", "  "] {
        assert!(matches!(
            data.create_repository(blank, &http("h/r")),
            Err(ServiceError::EmptyRepositoryName)
        ));
    }
    assert!(matches!(
        data.create_repository("R", &input(ProtocolType::A3s, "h", "1", "a")),
        Err(ServiceError::UnsupportedProtocol(_))
    ));
    assert!(matches!(
        data.create_repository("R", &http("")),
        Err(ServiceError::ProtocolCheck(ProtocolCheckError::UrlEmpty))
    ));
    assert!(matches!(
        data.create_repository("R", &input(ProtocolType::Http, "h", "70000", "a")),
        Err(ServiceError::ProtocolCheck(
            ProtocolCheckError::PortOutOfRange
        ))
    ));
    assert_eq!(data.repositories().count(), 0);

    data.create_repository("R", &http("h/r")).unwrap();
    let protocol = Protocol::new(ProtocolType::Http, "h/r", "80", "anonymous", "pw", true);
    assert_eq!(
        load(tmp.path()).repository("R").unwrap(),
        &Repository::new("R", protocol)
    );
    data.set_notify("R", true).unwrap();
    assert!(matches!(
        data.create_repository("R", &http("other")),
        Err(ServiceError::RepositoryExists(_))
    ));
    assert!(load(tmp.path()).repository("R").unwrap().notify);
}

#[test]
fn protocol_edit_resets_custom_timeouts() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_repository("R", &http("h/r")).unwrap();
    data.set_connection_timeout("R", Some("5000".to_owned()))
        .unwrap();
    data.set_repository_protocol("R", &http("h2/r")).unwrap();
    let protocol = load(tmp.path()).repository("R").unwrap().protocol.clone();
    assert_eq!(protocol.url, "h2/r");
    assert_eq!(protocol.connection_timeout_ms(), 300_000);
    assert!(matches!(
        data.set_repository_protocol("X", &http("h")),
        Err(ServiceError::RepositoryNotFound(_))
    ));
    assert!(matches!(
        data.set_repository_protocol("R", &input(ProtocolType::A3s, "h", "1", "a")),
        Err(ServiceError::UnsupportedProtocol(_))
    ));
}

fn cache(entry: &str) -> Sha1Cache {
    let mut cache = Sha1Cache::default();
    cache.entries.insert(
        PathBuf::from(entry),
        FileAttributes {
            sha1: "abc".to_owned(),
            last_modified: 1,
        },
    );
    cache
}

#[test]
fn rename_moves_the_file_and_both_caches() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_repository("Old", &http("h/r")).unwrap();
    data.set_exact_match("Old", true).unwrap();
    let store = data.store().clone();
    store
        .write_sha1_cache("Old", CacheKind::Sync, &cache("/sync"))
        .unwrap();
    store
        .write_sha1_cache("Old", CacheKind::Build, &cache("/build"))
        .unwrap();
    data.rename_repository("Old", "New").unwrap();

    assert_eq!(
        toml_files(&tmp.path().join("data/repositories")),
        [store.repository_path("New")]
    );
    assert_eq!(
        store.read_sha1_cache("New", CacheKind::Sync).entries,
        cache("/sync").entries
    );
    assert_eq!(
        store.read_sha1_cache("New", CacheKind::Build).entries,
        cache("/build").entries
    );
    assert!(!store.sha1_cache_path("Old", CacheKind::Sync).exists());
    assert!(!store.sha1_cache_path("Old", CacheKind::Build).exists());
    let reloaded = load(tmp.path());
    assert_eq!(reloaded.repository_names(), ["New"]);
    assert!(reloaded.repository("New").unwrap().exact_match);

    data.create_repository("Other", &http("h")).unwrap();
    assert!(matches!(
        data.rename_repository("New", "Other"),
        Err(ServiceError::RepositoryExists(_))
    ));
    assert!(matches!(
        data.rename_repository("Missing", "X"),
        Err(ServiceError::RepositoryNotFound(_))
    ));
    assert!(matches!(
        data.rename_repository("New", " "),
        Err(ServiceError::EmptyRepositoryName)
    ));
}

#[test]
fn rename_and_save_replace_a_non_canonical_source_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let protocol = Protocol::new(ProtocolType::Http, "h", "80", "anonymous", "", true);
    for name in ["A", "B"] {
        store
            .write_repository(&Repository::new(name, protocol.clone()))
            .unwrap();
        std::fs::rename(
            store.repository_path(name),
            tmp.path().join(format!("data/repositories/odd{name}.toml")),
        )
        .unwrap();
    }
    let mut data = load(tmp.path());
    data.rename_repository("A", "C").unwrap();
    data.set_notify("B", true).unwrap();
    assert_eq!(
        toml_files(&tmp.path().join("data/repositories")),
        [store.repository_path("B"), store.repository_path("C")]
    );
}

#[test]
fn remove_drops_the_repository_even_without_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let protocol = Protocol::new(ProtocolType::Http, "h", "80", "anonymous", "", true);
    let mut data = ClientData::from_parts(
        store(tmp.path()),
        Configuration::default(),
        Preferences::default(),
        Vec::new(),
        vec![Repository::new("Unsaved", protocol)],
    );
    assert!(!data.remove_repository("Unsaved").unwrap());
    assert_eq!(data.repositories().count(), 0);

    data.create_repository("Saved", &http("h")).unwrap();
    let store = data.store().clone();
    store
        .write_sha1_cache("Saved", CacheKind::Sync, &cache("/x"))
        .unwrap();
    assert!(data.remove_repository("Saved").unwrap());
    assert!(!store.repository_path("Saved").exists());
    assert!(!store.sha1_cache_path("Saved", CacheKind::Sync).exists());
    assert!(matches!(
        data.remove_repository("Saved"),
        Err(ServiceError::RepositoryNotFound(_))
    ));
}

#[test]
fn repository_setters_persist() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_repository("R", &input(ProtocolType::Https, "h/r", "443", "a"))
        .unwrap();
    let s = |v: &str| v.to_owned();
    data.set_repository_path("R", Some(s("/repo"))).unwrap();
    data.add_hidden_folder("R", s("a\\b")).unwrap();
    data.add_hidden_folder("R", s("c")).unwrap();
    data.set_default_download_location("R", None, Some(s("/dl")))
        .unwrap();
    data.set_default_download_location("R", Some("E"), Some(s("/e")))
        .unwrap();
    data.set_default_download_location("R", Some("F"), Some(s("/f")))
        .unwrap();
    data.set_default_download_location("R", Some("F"), None)
        .unwrap();
    data.set_notify("R", true).unwrap();
    data.set_auto_update("R", true).unwrap();
    data.set_no_auto_discover("R", false).unwrap();
    data.set_exact_match("R", true).unwrap();
    data.add_excluded_file_from_build("R", s("x")).unwrap();
    data.add_excluded_file_from_build("R", s("x")).unwrap();
    data.set_excluded_folders_from_sync("R", vec![s("f1"), s("f2"), s("f1")])
        .unwrap();
    data.set_number_of_connections("R", 0).unwrap();
    data.set_compressed("R", true).unwrap();
    data.set_upload_compressed_pbo_files_only("R", true)
        .unwrap();
    data.set_use_partial_file_transfer("R", false).unwrap();
    data.set_number_of_client_connections("R", 3).unwrap();
    data.set_parallel_upload_connections("R", 50).unwrap();
    data.set_repository_check_connections("R", 10).unwrap();
    data.set_maximum_client_download_speed("R", 1.5e6).unwrap();
    data.set_connection_timeout("R", Some(s("1"))).unwrap();
    data.set_read_timeout("R", Some(s("2"))).unwrap();

    assert!(data.remove_hidden_folder("R", "a/b").unwrap());
    let before = fingerprint(&data.store().repository_path("R"));
    assert!(!data.remove_hidden_folder("R", "zz").unwrap());
    assert_eq!(fingerprint(&data.store().repository_path("R")), before);
    assert!(matches!(
        data.set_parallel_upload_connections("R", 0),
        Err(ServiceError::InvalidSetting(_))
    ));
    assert!(matches!(
        data.set_repository_check_connections("R", 0),
        Err(ServiceError::InvalidSetting(_))
    ));
    assert!(matches!(
        data.set_notify("Missing", true),
        Err(ServiceError::RepositoryNotFound(_))
    ));

    let reloaded = load(tmp.path());
    let r = reloaded.repository("R").unwrap();
    assert_eq!(r, data.repository("R").unwrap());
    assert_eq!(r.path.as_deref(), Some("/repo"));
    assert_eq!(r.hidden_folder_paths.iter().collect::<Vec<_>>(), ["c"]);
    assert_eq!(r.default_download_location.as_deref(), Some("/dl"));
    assert_eq!(
        r.events_download_location.iter().collect::<Vec<_>>(),
        [(&s("E"), &s("/e"))]
    );
    assert!(r.notify && r.auto && !r.no_auto_discover && r.exact_match);
    assert_eq!(r.excluded_files_from_build.len(), 1);
    assert_eq!(r.excluded_folders_from_sync.len(), 2);
    assert_eq!(r.number_of_connections, 0);
    assert!(r.compressed && r.upload_compressed_pbo_files_only);
    assert!(r.no_partial_file_transfer);
    assert_eq!(r.number_of_client_connections, 3);
    assert_eq!(r.parallel_upload_connections, 10);
    assert_eq!(r.number_of_repository_check_connections, 4);
    assert_eq!(r.maximum_client_download_speed, 1.5e6);
    assert_eq!(r.protocol.connection_timeout.as_deref(), Some("1"));
    assert_eq!(r.protocol.read_timeout.as_deref(), Some("2"));
    assert_eq!(
        reloaded.repository_display_url("R").unwrap(),
        "https://h:443/r"
    );

    data.clear_excluded_files_from_build("R").unwrap();
    data.clear_excluded_folders_from_sync("R").unwrap();
    data.set_excluded_files_from_build("R", vec![s("y")])
        .unwrap();
    data.add_excluded_folder_from_sync("R", s("z")).unwrap();
    let reloaded = load(tmp.path());
    let r = reloaded.repository("R").unwrap();
    assert_eq!(
        r.excluded_files_from_build.iter().collect::<Vec<_>>(),
        ["y"]
    );
    assert_eq!(
        r.excluded_folders_from_sync.iter().collect::<Vec<_>>(),
        ["z"]
    );
}

#[test]
fn upload_protocol_is_checked_and_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    assert!(matches!(
        data.set_upload_protocol(
            "Missing",
            &input(ProtocolType::A3s, "h", "1", "a"),
            None,
            None
        ),
        Err(ServiceError::RepositoryNotFound(_))
    ));
    data.create_repository("R", &http("h")).unwrap();
    assert!(matches!(
        data.set_upload_protocol("R", &input(ProtocolType::A3s, "h", "1", "a"), None, None),
        Err(ServiceError::UnsupportedProtocol(_))
    ));
    assert!(matches!(
        data.set_upload_protocol(
            "R",
            &input(ProtocolType::Ftp, "h", "21", "a"),
            Some(String::new()),
            None
        ),
        Err(ServiceError::ProtocolCheck(
            ProtocolCheckError::ConnectionTimeoutEmpty
        ))
    ));
    data.set_upload_protocol(
        "R",
        &input(ProtocolType::Ftp, "h", "21", "a"),
        Some("10".to_owned()),
        Some("20".to_owned()),
    )
    .unwrap();
    let reloaded = load(tmp.path());
    let upload = reloaded
        .repository("R")
        .unwrap()
        .upload_protocol
        .clone()
        .unwrap();
    assert_eq!(upload.protocol_type, ProtocolType::Ftp);
    assert_eq!(upload.connection_timeout.as_deref(), Some("10"));
    assert_eq!(upload.read_timeout.as_deref(), Some("20"));
    data.reset_upload_protocol("R").unwrap();
    assert_eq!(
        load(tmp.path()).repository("R").unwrap().upload_protocol,
        None
    );
}

#[test]
fn favorite_servers_are_set_to_autoconfig_by_index() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.add_favorite_server(server(Some("s0"))).unwrap();
    data.add_favorite_server(server(Some("s1"))).unwrap();
    data.create_repository("R", &http("h")).unwrap();
    data.set_favorite_servers_to_autoconfig("R", &[1, 0])
        .unwrap();

    let reloaded = load(tmp.path());
    let selected = &reloaded
        .repository("R")
        .unwrap()
        .favorite_servers_set_to_autoconfig;
    assert_eq!(names(selected), [Some("s1"), Some("s0")]);
    assert!(
        reloaded
            .favorite_servers()
            .iter()
            .all(|s| s.repository_name.as_deref() == Some("R"))
    );

    assert!(matches!(
        data.set_favorite_servers_to_autoconfig("R", &[0, 5]),
        Err(ServiceError::FavoriteServerIndex { index: 5, len: 2 })
    ));
    assert!(matches!(
        data.set_favorite_servers_to_autoconfig("X", &[0]),
        Err(ServiceError::RepositoryNotFound(_))
    ));
    assert_eq!(
        names(
            &load(tmp.path())
                .repository("R")
                .unwrap()
                .favorite_servers_set_to_autoconfig
        ),
        [Some("s1"), Some("s0")]
    );
}

#[test]
fn update_closures_roll_back() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_repository("R", &http("h")).unwrap();
    let before = data.repository("R").unwrap().clone();
    assert!(matches!(
        data.update_repository("R", |r| {
            r.notify = true;
            Err(ServiceError::EmptyRepositoryName)
        }),
        Err(ServiceError::EmptyRepositoryName)
    ));
    assert!(matches!(
        data.update_repository("R", |r| {
            r.name = "Other".to_owned();
            Ok(true)
        }),
        Err(ServiceError::NameChange)
    ));
    assert_eq!(data.repository("R").unwrap(), &before);
    assert_eq!(load(tmp.path()).repository("R").unwrap(), &before);
}

/// Write failures leave memory unchanged. Skipped when the directory stays writable (root).
#[cfg(unix)]
#[test]
fn failed_writes_leave_memory_unchanged() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_repository("R", &http("h")).unwrap();
    data.create_profile("P").unwrap();
    data.set_active_profile("P").unwrap();
    let read_only = |dir: &Path, mode: u32| {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    let repositories = tmp.path().join("data/repositories");
    let profiles = tmp.path().join("config/profiles");
    read_only(&repositories, 0o500);
    read_only(&profiles, 0o500);
    if std::fs::write(repositories.join("probe"), b"").is_ok() {
        return;
    }
    let notify = data.set_notify("R", true);
    let parameters = data.set_additional_parameters(Some("-x".to_owned()));
    read_only(&repositories, 0o700);
    read_only(&profiles, 0o700);

    assert!(matches!(notify, Err(ServiceError::Store(_))));
    assert!(!data.repository("R").unwrap().notify);
    assert!(matches!(parameters, Err(ServiceError::Store(_))));
    assert_eq!(data.additional_parameters(), None);
}

// ---------------------------------------------------------------------------------------------
// Names held by unreadable files

#[test]
fn unreadable_files_keep_their_names() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    std::fs::create_dir_all(tmp.path().join("config/profiles")).unwrap();
    std::fs::create_dir_all(tmp.path().join("data/repositories")).unwrap();
    // Files of a newer store format, under names that are not the canonical ones.
    std::fs::write(
        tmp.path().join("config/profiles/newer.toml"),
        "format = 2\n[profile]\nname = \"P\"\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("data/repositories/newer.toml"),
        "format = 2\n[repository]\nname = \"R\"\n",
    )
    .unwrap();
    let mut data = ClientData::load(store).data;
    assert!(matches!(
        data.create_profile("P"),
        Err(ServiceError::ProfileExists(_))
    ));
    assert!(matches!(
        data.create_repository("R", &http("h")),
        Err(ServiceError::RepositoryExists(_))
    ));
    data.create_profile("Q").unwrap();
    assert!(matches!(
        data.rename_profile("Q", "P"),
        Err(ServiceError::ProfileExists(_))
    ));
}

// ---------------------------------------------------------------------------------------------
// Autoconfig export and import

#[test]
fn export_selects_profiles_servers_and_reduced_repositories() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    let p1 = rich_profile("P1");
    data.update_configuration(|c| {
        c.favorite_servers = vec![server(Some("a")), server(Some("b"))];
        true
    })
    .unwrap();
    data.create_profile("P1").unwrap();
    data.set_active_profile("P1").unwrap();
    data.update_active_profile(|p| {
        *p = p1.clone();
        true
    })
    .unwrap();
    data.create_profile("P2").unwrap();
    data.create_repository("R", &http("h/r")).unwrap();
    data.set_notify("R", true).unwrap();

    let out = tmp.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let path = data
        .export_autoconfig(
            &["P2".to_owned(), "missing".to_owned(), "P1".to_owned()],
            &[1],
            &["R".to_owned(), "missing".to_owned()],
            &out,
        )
        .unwrap();
    let exported = a3s_repo_files::autoconfig_export::import_auto_config(&path)
        .unwrap()
        .value;
    let profile_names: Vec<&str> = exported.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(profile_names, ["P2", "P1"]);
    assert!(
        exported
            .profiles
            .iter()
            .all(|p| p.launcher_options == LauncherOptions::default())
    );
    assert_eq!(names(&exported.favorite_servers), [Some("b")]);
    let protocol = data.repository("R").unwrap().protocol.clone();
    assert_eq!(exported.repositories, [Repository::new("R", protocol)]);
    assert_eq!(
        data.profile("P1").unwrap().launcher_options,
        p1.launcher_options
    );

    let other = tmp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    assert!(matches!(
        data.export_autoconfig(&[], &[9], &[], &other),
        Err(ServiceError::FavoriteServerIndex { index: 9, len: 2 })
    ));
    assert_eq!(toml_files(&other), Vec::<PathBuf>::new());
    assert!(matches!(
        data.export_autoconfig(&[], &[], &[], &tmp.path().join("absent")),
        Err(ServiceError::AutoConfigWrite(_))
    ));
}

#[test]
fn import_merges_without_overwriting_repositories() {
    let tmp = tempfile::tempdir().unwrap();
    // The file: profiles P1 and P3, two servers (one equal to the existing one), repositories
    // R and S.
    let exporter_root = tmp.path().join("exporter");
    let mut exporter = load(&exporter_root);
    exporter.create_profile("P1").unwrap();
    exporter.create_profile("P3").unwrap();
    exporter.add_favorite_server(server(Some("same"))).unwrap();
    exporter.add_favorite_server(server(Some("new"))).unwrap();
    exporter
        .create_repository("R", &http("exported/r"))
        .unwrap();
    exporter
        .create_repository("S", &http("exported/s"))
        .unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let file = exporter
        .export_autoconfig(
            &["P1".to_owned(), "P3".to_owned()],
            &[0, 1],
            &["R".to_owned(), "S".to_owned()],
            &out,
        )
        .unwrap();

    let root = tmp.path().join("importer");
    let mut data = load(&root);
    data.create_profile("P1").unwrap();
    data.set_active_profile("P1").unwrap();
    data.update_launcher_options(|o| o.nologs = true).unwrap();
    data.add_favorite_server(server(Some("same"))).unwrap();
    data.create_repository("R", &http("mine/r")).unwrap();
    data.set_notify("R", true).unwrap();
    let store = data.store().clone();
    store
        .write_sha1_cache("R", CacheKind::Sync, &cache("/r"))
        .unwrap();
    let repository_before = fingerprint(&store.repository_path("R"));
    let cache_before = fingerprint(&store.sha1_cache_path("R", CacheKind::Sync));

    let summary = data.import_autoconfig(&file).unwrap();
    assert_eq!(summary.profiles_imported, ["P1", "P3"]);
    assert_eq!(summary.profiles_replaced, ["P1"]);
    assert_eq!(summary.favorite_servers_added, 2);
    assert_eq!(summary.repositories_added, ["S"]);
    assert_eq!(summary.repositories_skipped, ["R"]);

    let reloaded = load(&root);
    assert_eq!(
        reloaded.profile("P1").unwrap().launcher_options,
        LauncherOptions::default()
    );
    assert!(reloaded.profile("P3").is_some());
    assert_eq!(
        names(reloaded.favorite_servers()),
        [Some("new"), Some("same"), Some("same")]
    );
    assert_eq!(reloaded.repository("S").unwrap().protocol.url, "exported/s");
    assert_eq!(reloaded.repository("R").unwrap().protocol.url, "mine/r");
    assert!(reloaded.repository("R").unwrap().notify);
    assert_eq!(fingerprint(&store.repository_path("R")), repository_before);
    assert_eq!(
        fingerprint(&store.sha1_cache_path("R", CacheKind::Sync)),
        cache_before
    );

    assert!(matches!(
        data.import_autoconfig(&tmp.path().join("missing.a3s.autoconfig")),
        Err(ServiceError::AutoConfigRead(_))
    ));
}

#[test]
fn update_profile_writes_a_profile_that_is_not_active() {
    let tmp = tempfile::tempdir().unwrap();
    let mut data = load(tmp.path());
    data.create_profile("Other").unwrap();
    assert!(
        data.update_profile("Other", |p| {
            p.additional_parameters = Some("-x".to_owned());
            true
        })
        .unwrap()
    );
    assert_eq!(data.active_profile_name(), "Default");
    let reloaded = load(tmp.path());
    let other = reloaded.profile("Other").unwrap();
    assert_eq!(other.additional_parameters.as_deref(), Some("-x"));

    assert!(matches!(
        data.update_profile("Other", |p| {
            p.name = "Renamed".to_owned();
            true
        }),
        Err(ServiceError::NameChange)
    ));
    assert!(matches!(
        data.update_profile("Missing", |_| true),
        Err(ServiceError::ProfileNotFound(_))
    ));
}
