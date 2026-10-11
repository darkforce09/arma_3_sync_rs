//! serde / TOML round trips of the local types.

use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, ModsetType, StartWithOs,
};
use a3s_types::local::{
    Configuration, ExternalApplication, Preferences, Profile, Proxy, Repository, TreeDirectory,
    TreeLeaf, TreeNode,
};
use a3s_types::{EncryptionMode, FavoriteServer, Protocol, ProtocolType};
use serde::Serialize;
use serde::de::DeserializeOwned;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
    let text = toml::to_string(value).unwrap();
    let back: T = toml::from_str(&text).unwrap_or_else(|err| panic!("{err}\n{text}"));
    assert_eq!(&back, value, "{text}");
    text
}

fn server(name: &str) -> FavoriteServer {
    FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some("127.0.0.1".to_owned()),
        port: 2302,
        password: None,
        selected: true,
        modset_name: Some("Event".to_owned()),
        repository_name: Some("Repo".to_owned()),
    }
}

#[test]
fn repository_round_trip() {
    let mut upload = Protocol::new(ProtocolType::Sftp, "upload.example/r", "22", "u", "p", true);
    upload.encryption_mode = Some(EncryptionMode::ExplicitSsl);
    upload.connection_timeout = None;
    let mut r = Repository::new(
        "My Repo",
        Protocol::new(
            ProtocolType::Https,
            "repo.example/a3s",
            "443",
            "anonymous",
            "",
            true,
        ),
    );
    r.upload_protocol = Some(upload);
    r.revision = 42;
    r.hidden_folder_paths = ["@a/x".to_owned(), "b".to_owned()].into();
    r.events_download_location = [("My Event".to_owned(), "/games/x".to_owned())].into();
    r.maximum_client_download_speed = 1.5;
    r.parallel_upload_connections = 0;
    r.number_of_connections = 0;
    r.favorite_servers_set_to_autoconfig = vec![server("s1"), server("s2")];
    r.excluded_files_from_build = ["*.bak".to_owned()].into();
    round_trip(&r);
}

#[test]
fn profile_round_trip() {
    let mut group = TreeDirectory::new("Group");
    group.modset_type = Some(ModsetType::Repository);
    group.modset_repository_name = Some("Repo".to_owned());
    let mut nested = TreeDirectory::new("Nested");
    nested.add_child(TreeNode::Leaf(TreeLeaf {
        workshop_id: Some("123".to_owned()),
        selected: true,
        ..TreeLeaf::new("@cba")
    }));
    group.add_child(TreeNode::Directory(nested));
    group.add_child(TreeNode::Leaf(TreeLeaf {
        dlc_app_id: Some("1042220".to_owned()),
        ..TreeLeaf::new("GM")
    }));
    let mut p = Profile::new("Main");
    p.tree.add_child(TreeNode::Directory(group));
    p.tree.add_child(TreeNode::Leaf(TreeLeaf::new("@ace")));
    p.additional_parameters = Some("-world=empty".to_owned());
    p.addon_names_by_priority = vec!["@cba".to_owned(), "@ace".to_owned()];
    p.launcher_options.cpu_count_selection = 4;
    p.launcher_options.arma3_exe_path = Some("C:\\arma3.exe".to_owned());
    p.addon_search_directories = vec!["/games/arma3".to_owned()];
    round_trip(&p);
}

#[test]
fn configuration_round_trip() {
    let mut c = Configuration {
        height: 600,
        width: 800,
        server_name: Some("srv".to_owned()),
        view_mode_tree: false,
        favorite_servers: vec![server("a"), server("b")],
        external_applications: vec![ExternalApplication {
            name: Some("TS".to_owned()),
            executable_path: Some("/usr/bin/ts3".to_owned()),
            parameters: None,
            enable: true,
        }],
        proxy: Proxy {
            enable_proxy: true,
            proxy_protocol: Some(Protocol::new(
                ProtocolType::Socks5,
                "proxy",
                "1080",
                "",
                "",
                false,
            )),
        },
        ..Default::default()
    };
    c.rpt_options.rpt_path = Some("/rpt".to_owned());
    c.tfar_options.ts3_path = Some("/ts3".to_owned());
    round_trip(&c);
}

#[test]
fn preferences_round_trip() {
    let p = Preferences {
        launch_panel_game_launch: MinimizationType::Tray,
        launch_panel_minimized: MinimizationType::Close,
        look_and_feel: LookAndFeel::LafNoire,
        icon_resize_size: IconResize::Size2,
        start_with_os: StartWithOs::Enabled,
        check_repositories_frequency: CheckRepositoriesFrequency::Disabled,
        prefer_github_updates: false,
    };
    let text = round_trip(&p);
    // Enums use the Rust variant names.
    assert!(text.contains("look_and_feel = \"LafNoire\""), "{text}");
    round_trip(&Preferences::default());
}

#[test]
fn missing_keys_take_java_initializers() {
    assert_eq!(
        toml::from_str::<Configuration>("").unwrap(),
        Configuration::default()
    );
    assert_eq!(
        toml::from_str::<Preferences>("").unwrap(),
        Preferences::default()
    );

    let text = r#"
        name = "Repo"

        [protocol]
        protocol_type = "Http"
        url = "repo.example/path"
        port = "80"
        validate_ssl_certificate = false
    "#;
    let r: Repository = toml::from_str(text).unwrap();
    assert!(r.no_auto_discover);
    assert_eq!(r.parallel_upload_connections, 4);
    assert_eq!(r.number_of_connections, 1);
    assert_eq!(r.number_of_repository_check_connections, 4);
    assert_eq!(r, Repository::new("Repo", r.protocol.clone()));

    // A repository needs a name and a protocol.
    assert!(toml::from_str::<Repository>("name = \"Repo\"").is_err());

    let p: Profile = toml::from_str("name = \"P\"").unwrap();
    assert_eq!(p, Profile::new("P"));
}
