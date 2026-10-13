//! Decoding the real Java-written client files of `tests/fixtures/java_local`.
//!
//! Expected values are transcribed from the Java identity dumps in
//! `tests/fixtures/java_local/dumps/` and the harness inputs (`LocalData.java`).

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use a3s_jserial::{FieldType, Node, flags};
use a3s_repo_files::legacy::{
    read_configuration_file, read_object_file, read_preferences_file, read_profile_file,
    read_repository_file, sealed,
};
use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, ModsetType, StartWithOs,
};
use a3s_types::local::{
    Acre2Options, AcreOptions, BikeyExtractOptions, Configuration, ExternalApplication,
    FileAttributes, LauncherOptions, LegacyConfiguration, LegacyDecode as _, LegacyRepository,
    Preferences, Profile, Proxy, Repository, RptOptions, TfarOptions, TreeDirectory, TreeLeaf,
    TreeNode,
};
use a3s_types::{FavoriteServer, Protocol, ProtocolType};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_local")
        .join(rel)
}

const PERUSER_CONFIG: &str = "peruser/config/Arma3Sync";
const PERUSER_REPOS: &str = "peruser/data/Arma3Sync/repositories";
const PORTABLE_CONFIG: &str = "portable/resources/configuration";
const PORTABLE_REPOS: &str = "portable/resources/ftp";

#[expect(clippy::unnecessary_wraps, reason = "shorthand for optional fields")]
fn s(v: &str) -> Option<String> {
    Some(v.to_owned())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

/// `LauncherOptions` of `LocalData.eventNightProfile()` and of the deprecated `a3s.cfg` field.
fn sample_launcher_options() -> LauncherOptions {
    LauncherOptions {
        game_profile: s("Sample Player"),
        show_script_errors: true,
        no_pause: true,
        file_patching: false,
        window_mode: true,
        check_signatures: true,
        auto_restart: false,
        launch_arma3_as_administrator: false,
        mission_file: false,
        mission_file_path: None,
        max_memory_selection: s("8192"),
        cpu_count_selection: 6,
        ex_threads_selection: s("7"),
        malloc_selection: s("system"),
        enable_ht: true,
        huge_pages: false,
        no_splash_screen: true,
        default_world: false,
        nologs: false,
        arma3_exe_path: s("/srv/arma3/SampleRepo/arma3_x64.exe"),
        steam_exe_path: s("/srv/arma3/SampleRepo/steam.exe"),
        game_start: Default::default(),
    }
}

fn favorite(repository_name: &str) -> FavoriteServer {
    FavoriteServer {
        name: s("Alpha Server"),
        ip_address: s("192.0.2.10"),
        port: 2302,
        password: s("alpha-pass"),
        selected: true,
        modset_name: s("Event Night"),
        repository_name: s(repository_name),
    }
}

fn expected_configuration() -> LegacyConfiguration {
    let mut proxy = Protocol::new(
        ProtocolType::Socks5,
        "proxy.example.org",
        "1080",
        "proxy-user",
        "proxy-pass",
        false,
    );
    proxy.connection_timeout = None;
    proxy.read_timeout = None;
    LegacyConfiguration {
        configuration: Configuration {
            height: 900,
            width: 1400,
            profile_name: "Event Night".to_owned(),
            game_version: "Arma 3".to_owned(),
            server_name: s("WOG #1"),
            default_modset: s("Sample HTTP"),
            view_mode_tree: false,
            favorite_servers: vec![
                favorite("Sample HTTP"),
                FavoriteServer {
                    name: s("Bravo Server"),
                    ip_address: s("192.0.2.20"),
                    port: 2402,
                    password: None,
                    selected: false,
                    modset_name: None,
                    repository_name: None,
                },
            ],
            external_applications: vec![ExternalApplication {
                name: s("TeamSpeak 3"),
                executable_path: s("/srv/arma3/SampleRepo/tools/ts3client_linux_amd64"),
                parameters: s("-nosingleinstance"),
                enable: true,
            }],
            acre_options: AcreOptions {
                ts3_path: s("/srv/arma3/SampleRepo/ts3"),
                acre_plugin_path: s("/srv/arma3/SampleRepo/ts3/plugins/acre"),
                acre_userconfig_path: s("/srv/arma3/SampleRepo/userconfig/acre"),
            },
            acre2_options: Acre2Options {
                ts3_path: s("/srv/arma3/SampleRepo/ts3"),
                acre2_plugin_path: s("/srv/arma3/SampleRepo/ts3/plugins/acre2"),
            },
            tfar_options: TfarOptions {
                ts3_path: s("/srv/arma3/SampleRepo/ts3"),
                tfar_plugin_path: s("/srv/arma3/SampleRepo/ts3/plugins/tfar"),
                tfar_userconfig_path: s("/srv/arma3/SampleRepo/userconfig/tfar"),
            },
            rpt_options: RptOptions {
                rpt_path: s("/srv/arma3/SampleRepo/rpt"),
            },
            bikey_extract_options: BikeyExtractOptions {
                source_directory_path: s("/srv/arma3/SampleRepo/@Mod_A"),
                target_directory_path: s("/srv/arma3/SampleRepo/keys"),
            },
            proxy: Proxy {
                enable_proxy: true,
                proxy_protocol: Some(proxy),
            },
        },
        addon_search_directory_paths: Some(strings(&[
            "/home/user/arma3/!Workshop",
            "C:\\Games\\Arma 3",
        ])),
        launcher_options: Some(sample_launcher_options()),
    }
}

fn expected_preferences() -> Preferences {
    Preferences {
        launch_panel_game_launch: MinimizationType::Tray,
        launch_panel_minimized: MinimizationType::Tray,
        look_and_feel: LookAndFeel::LafHifi,
        icon_resize_size: IconResize::Size2,
        start_with_os: StartWithOs::Enabled,
        check_repositories_frequency: CheckRepositoriesFrequency::Freq2,
        prefer_github_updates: true,
    }
}

fn leaf(name: &str, selected: bool, optional: bool) -> TreeLeaf {
    TreeLeaf {
        selected,
        optional,
        ..TreeLeaf::new(name)
    }
}

fn expected_event_night() -> Profile {
    let night_op = TreeDirectory {
        modset_type: Some(ModsetType::Event),
        children: vec![TreeNode::Leaf(TreeLeaf {
            workshop_id: s("3147580028"),
            ..leaf("@gm", true, false)
        })],
        ..TreeDirectory::new("Night Op")
    };
    let nested = TreeDirectory {
        marked: true,
        children: vec![TreeNode::Leaf(leaf("@Nested", true, false))],
        ..TreeDirectory::new("Nested")
    };
    let repository_mods = TreeDirectory {
        selected: true,
        modset_type: Some(ModsetType::Repository),
        modset_repository_name: s("Sample HTTP"),
        children: vec![
            TreeNode::Directory(nested),
            TreeNode::Leaf(leaf("@Mod_A", true, false)),
            TreeNode::Leaf(leaf("@mod_b", false, true)),
        ],
        ..TreeDirectory::new("Sample HTTP mods")
    };
    let mut tree = TreeDirectory::new_root();
    tree.children = vec![
        TreeNode::Directory(night_op),
        TreeNode::Directory(repository_mods),
        TreeNode::Leaf(TreeLeaf {
            dlc_app_id: s("1681170"),
            ..leaf("WS", true, false)
        }),
    ];
    Profile {
        name: "Event Night".to_owned(),
        tree,
        additional_parameters: s("-world=empty -noLogs"),
        addon_names_by_priority: strings(&["@Mod_A", "@mod_b", "@gm"]),
        launcher_options: sample_launcher_options(),
        addon_search_directories: strings(&["/srv/arma3/SampleRepo", "/home/user/arma3/!Workshop"]),
        last_added_addon_search_directory: s("/srv/arma3/SampleRepo"),
    }
}

fn attributes(sha1: &str, last_modified: i64) -> FileAttributes {
    FileAttributes {
        sha1: sha1.to_owned(),
        last_modified,
    }
}

fn expected_http() -> LegacyRepository {
    let mut repository = Repository::new(
        "Sample HTTP",
        Protocol::new(
            ProtocolType::Http,
            "repo.example.org/arma3",
            "80",
            "anonymous",
            "",
            true,
        ),
    );
    repository.auto = true;
    repository.compressed = true;
    repository.exact_match = true;
    repository.maximum_client_download_speed = 1.5;
    repository.no_auto_discover = false;
    repository.no_partial_file_transfer = true;
    repository.notify = true;
    repository.number_of_client_connections = 3;
    repository.number_of_connections = 2;
    repository.number_of_repository_check_connections = 2;
    repository.parallel_upload_connections = 4;
    repository.revision = 7;
    repository.upload_compressed_pbo_files_only = false;
    repository.auto_config_url = s("http://repo.example.org/arma3/.a3s/autoconfig");
    repository.default_download_location = s("/srv/arma3/SampleRepo/downloads");
    repository.excluded_files_from_build = ["/srv/arma3/SampleRepo/excluded.txt".to_owned()].into();
    repository.excluded_folders_from_sync =
        ["/srv/arma3/SampleRepo/docs/hidden_a".to_owned()].into();
    repository.favorite_servers_set_to_autoconfig = vec![favorite("Sample SFTP")];
    repository.hidden_folder_paths =
        ["docs/hidden_a".to_owned(), "docs/hidden_b".to_owned()].into();
    repository.events_download_location = [(
        "Night Op".to_owned(),
        "/srv/arma3/SampleRepo/downloads/night_op".to_owned(),
    )]
    .into();
    repository.path = s("/srv/arma3/SampleRepo");
    LegacyRepository {
        repository,
        files_for_sync: BTreeMap::from([
            (
                "/srv/arma3/SampleRepo/downloads/@Mod_A/addons/empty.txt".to_owned(),
                attributes("0", 1_704_024_000_000),
            ),
            (
                "/srv/arma3/SampleRepo/downloads/@Mod_A/addons/mod_a.pbo".to_owned(),
                attributes(
                    "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12",
                    1_704_024_060_000,
                ),
            ),
        ]),
        files_for_build: BTreeMap::from([
            (
                "/srv/arma3/SampleRepo/@Mod_A/addons/empty.txt".to_owned(),
                attributes("0", 1_704_024_120_000),
            ),
            (
                "/srv/arma3/SampleRepo/@Mod_A/addons/mod_a.pbo".to_owned(),
                attributes(
                    "de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3",
                    1_704_024_180_000,
                ),
            ),
        ]),
    }
}

/// Decodes a plain (unencrypted) `GZIP(ObjectOutputStream(Repository))` copy.
fn read_plain_repository(rel: &str) -> LegacyRepository {
    let graph = read_object_file(&fixture(rel)).unwrap();
    let mut warnings = Vec::new();
    let value = LegacyRepository::decode_legacy(&graph, graph.root_value().unwrap(), &mut warnings)
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    value
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&std::fs::read(path).unwrap()[..])
        .read_to_end(&mut raw)
        .unwrap();
    raw
}

#[test]
fn configuration_and_preferences() {
    for dir in [
        format!("{PERUSER_CONFIG}/configuration"),
        PORTABLE_CONFIG.to_owned(),
    ] {
        let cfg = read_configuration_file(&fixture(&format!("{dir}/a3s.cfg"))).unwrap();
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        assert_eq!(cfg.value, expected_configuration(), "{dir}");

        let prefs = read_preferences_file(&fixture(&format!("{dir}/a3s.prefs"))).unwrap();
        assert!(prefs.warnings.is_empty(), "{:?}", prefs.warnings);
        assert_eq!(prefs.value, expected_preferences(), "{dir}");
    }
}

#[test]
fn profiles() {
    for dir in [
        format!("{PERUSER_CONFIG}/profiles"),
        "portable/profiles".to_owned(),
    ] {
        let night = read_profile_file(&fixture(&format!("{dir}/Event Night.a3s.profile"))).unwrap();
        assert!(night.warnings.is_empty(), "{:?}", night.warnings);
        assert_eq!(night.value, expected_event_night(), "{dir}");

        let default = read_profile_file(&fixture(&format!("{dir}/Default.a3s.profile"))).unwrap();
        assert!(default.warnings.is_empty(), "{:?}", default.warnings);
        assert_eq!(default.value, Profile::new("Default"), "{dir}");
    }
}

#[test]
fn encrypted_repositories() {
    let http = read_repository_file(&fixture(&format!(
        "{PERUSER_REPOS}/Sample_HTTP__5ab39964a7ab.a3s.repository"
    )))
    .unwrap();
    assert!(http.warnings.is_empty(), "{:?}", http.warnings);
    assert_eq!(http.value, expected_http());

    let ftp = read_repository_file(&fixture(&format!(
        "{PERUSER_REPOS}/Sample_FTP__21a7c6dd6e0b.a3s.repository"
    )))
    .unwrap()
    .value;
    let r = &ftp.repository;
    assert_eq!(r.name, "Sample FTP");
    assert_eq!(r.protocol.protocol_type, ProtocolType::Ftp);
    assert_eq!(
        (r.protocol.url.as_str(), r.protocol.port.as_str()),
        ("ftp.example.org/arma3", "21")
    );
    assert_eq!(r.protocol.login.as_deref(), Some("player"));
    assert_eq!(r.protocol.password.as_deref(), Some("ftp-pass"));
    let upload = r.upload_protocol.as_ref().unwrap();
    assert_eq!(upload.protocol_type, ProtocolType::Ftps);
    assert_eq!(upload.port, "990");
    assert_eq!(upload.connection_timeout.as_deref(), Some("30000"));
    assert_eq!(upload.read_timeout.as_deref(), Some("60000"));
    assert_eq!(r.parallel_upload_connections, 6);
    assert!(r.upload_compressed_pbo_files_only);
    assert_eq!(ftp.files_for_sync, expected_http().files_for_sync);

    let sftp = read_repository_file(&fixture(&format!(
        "{PERUSER_REPOS}/Sample_SFTP__132c722dd1cb.a3s.repository"
    )))
    .unwrap()
    .value;
    assert_eq!(sftp.repository.name, "Sample SFTP");
    assert_eq!(sftp.repository.protocol.protocol_type, ProtocolType::Sftp);
    assert_eq!(sftp.repository.protocol.port, "22");

    // The encrypted files hold exactly what the plain copies hold.
    for (name, file) in [
        ("HTTP", "Sample_HTTP__5ab39964a7ab"),
        ("FTP", "Sample_FTP__21a7c6dd6e0b"),
        ("SFTP", "Sample_SFTP__132c722dd1cb"),
    ] {
        let plain = read_plain_repository(&format!("plain/Sample_{name}.repository.gz"));
        for dir in [PERUSER_REPOS, PORTABLE_REPOS] {
            let encrypted =
                read_repository_file(&fixture(&format!("{dir}/{file}.a3s.repository"))).unwrap();
            assert_eq!(encrypted.value, plain, "{dir}/{file}");
        }
    }
    // The legacy-named byte copy.
    assert_eq!(
        read_repository_file(&fixture(&format!(
            "{PERUSER_REPOS}/SampleFTP.a3s.repository"
        )))
        .unwrap()
        .value,
        ftp
    );
}

#[test]
fn sealed_object_descriptor_and_byte_identical_seal() {
    let path = fixture(&format!(
        "{PERUSER_REPOS}/Sample_HTTP__5ab39964a7ab.a3s.repository"
    ));
    let graph = read_object_file(&path).unwrap();
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let Node::Object(object) = graph.node(root).unwrap() else {
        panic!("root is not an object");
    };
    let desc = graph.class_desc(object.class).unwrap();
    assert_eq!(desc.name, sealed::SEALED_OBJECT);
    assert_eq!(desc.serial_version_uid, sealed::SEALED_OBJECT_UID);
    assert_eq!(desc.flags, flags::SERIALIZABLE);
    assert_eq!(desc.super_class, None);
    let fields: Vec<_> = desc
        .fields
        .iter()
        .map(|f| (f.field_type, f.name.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            (FieldType::Array, "encodedParams"),
            (FieldType::Array, "encryptedContent"),
            (FieldType::Object, "paramsAlg"),
            (FieldType::Object, "sealAlg"),
        ]
    );
    let content = graph
        .field(root, "encryptedContent")
        .unwrap()
        .as_ref()
        .unwrap();
    let Node::Array(array) = graph.node(content).unwrap() else {
        panic!("encryptedContent is not an array");
    };
    let byte_array = graph.class_desc(array.class).unwrap();
    assert_eq!(byte_array.name, sealed::BYTE_ARRAY);
    assert_eq!(byte_array.serial_version_uid, sealed::BYTE_ARRAY_UID);
    assert_eq!(byte_array.flags, flags::SERIALIZABLE);

    // ECB is deterministic: sealing the decrypted content again reproduces Java's stream.
    let inner = sealed::open(&graph, &mut Vec::new()).unwrap();
    let ours = sealed::seal(&inner).unwrap();
    let mut ours_raw = Vec::new();
    flate2::read::GzDecoder::new(&ours[..])
        .read_to_end(&mut ours_raw)
        .unwrap();
    assert_eq!(ours_raw, gunzip(&path));
    // And the inner stream is Java's plain serialization of the repository.
    assert_eq!(inner, gunzip(&fixture("plain/Sample_HTTP.repository.gz")));
}

#[test]
fn historical_variants() {
    // Written by an older `Repository` class without the two connection fields.
    let old = read_repository_file(&fixture(
        "variants/repository_pre_connections.a3s.repository",
    ))
    .unwrap()
    .value;
    assert_eq!(old.repository.parallel_upload_connections, 0);
    assert_eq!(old.repository.number_of_repository_check_connections, 0);
    assert_eq!(old.repository.parallel_upload_connections(), 4);
    assert_eq!(old.repository.repository_check_connections(), 4);
    assert_eq!(
        old,
        read_plain_repository("variants/repository_pre_connections.repository.gz")
    );

    let prefs = read_preferences_file(&fixture("variants/prefs_pre_github.a3s.prefs")).unwrap();
    assert!(prefs.value.prefer_github_updates);
    assert_eq!(prefs.value, expected_preferences());

    let cfg = read_configuration_file(&fixture("variants/cfg_with_aia.a3s.cfg")).unwrap();
    assert_eq!(cfg.value, expected_configuration());

    let profile =
        read_profile_file(&fixture("variants/treeleaf_pre_workshop.a3s.profile")).unwrap();
    let mut expected = expected_event_night();
    // The older `TreeLeaf` has neither `workshopId` nor `dlcAppId`.
    clear_leaf_ids(&mut expected.tree);
    assert_eq!(profile.value, expected);
}

fn clear_leaf_ids(dir: &mut TreeDirectory) {
    for child in &mut dir.children {
        match child {
            TreeNode::Directory(d) => clear_leaf_ids(d),
            TreeNode::Leaf(l) => {
                l.workshop_id = None;
                l.dlc_app_id = None;
            }
        }
    }
}
