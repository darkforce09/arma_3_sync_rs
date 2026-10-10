//! Defaults (A5), addon-group tree order (A6) and favorite server / external application order
//! (A7) of the local types.

use std::collections::{BTreeMap, BTreeSet};

use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, ModsetType, StartWithOs,
};
use a3s_types::local::{
    Acre2Options, AcreOptions, Addon, BikeyExtractOptions, Configuration, ExternalApplication,
    FileAttributes, LauncherOptions, PROFILE_TREE_ROOT_NAME, Preferences, Profile, Proxy,
    Repository, RptOptions, TfarOptions, TreeDirectory, TreeLeaf, TreeNode,
};
use a3s_types::{FavoriteServer, Protocol, ProtocolType};

fn protocol() -> Protocol {
    Protocol::new(ProtocolType::Ftp, "host/repo", "21", "anonymous", "", false)
}

#[test]
fn a5_repository_defaults() {
    let r = Repository::new("name", protocol());
    let expected = Repository {
        name: "name".to_owned(),
        protocol: protocol(),
        bit_torrent_protocol: None,
        revision: 0,
        notify: false,
        auto: false,
        hidden_folder_paths: BTreeSet::new(),
        no_auto_discover: true,
        exact_match: false,
        default_download_location: None,
        events_download_location: BTreeMap::new(),
        number_of_client_connections: 0,
        number_of_repository_check_connections: 4,
        maximum_client_download_speed: 0.0,
        upload_protocol: None,
        upload_compressed_pbo_files_only: false,
        parallel_upload_connections: 4,
        path: None,
        number_of_connections: 1,
        compressed: false,
        no_partial_file_transfer: false,
        auto_config_url: None,
        favorite_servers_set_to_autoconfig: Vec::new(),
        excluded_files_from_build: BTreeSet::new(),
        excluded_folders_from_sync: BTreeSet::new(),
    };
    assert_eq!(r, expected);
}

#[test]
fn a5_profile_defaults() {
    let p = Profile::new("Default");
    assert_eq!(p.name, "Default");
    assert_eq!(p.tree.name, PROFILE_TREE_ROOT_NAME);
    assert_eq!(PROFILE_TREE_ROOT_NAME, "racine");
    assert!(p.tree.children.is_empty());
    assert!(!p.tree.selected && !p.tree.marked && !p.tree.optional && !p.tree.updated);
    assert_eq!(p.tree.modset_type, None);
    assert_eq!(p.tree.modset_repository_name, None);
    assert_eq!(p.additional_parameters, None);
    assert!(p.addon_names_by_priority.is_empty());
    assert_eq!(p.launcher_options, LauncherOptions::default());
    assert!(p.addon_search_directories.is_empty());
    assert_eq!(p.last_added_addon_search_directory, None);
}

#[test]
fn a5_configuration_defaults() {
    let c = Configuration::default();
    let expected = Configuration {
        height: 0,
        width: 0,
        profile_name: "Default".to_owned(),
        game_version: "Arma 3".to_owned(),
        server_name: None,
        default_modset: None,
        view_mode_tree: true,
        favorite_servers: Vec::new(),
        external_applications: Vec::new(),
        acre_options: AcreOptions {
            ts3_path: None,
            acre_plugin_path: None,
            acre_userconfig_path: None,
        },
        acre2_options: Acre2Options {
            ts3_path: None,
            acre2_plugin_path: None,
        },
        tfar_options: TfarOptions {
            ts3_path: None,
            tfar_plugin_path: None,
            tfar_userconfig_path: None,
        },
        rpt_options: RptOptions { rpt_path: None },
        bikey_extract_options: BikeyExtractOptions {
            source_directory_path: None,
            target_directory_path: None,
        },
        proxy: Proxy {
            enable_proxy: false,
            proxy_protocol: None,
        },
    };
    assert_eq!(c, expected);
}

#[test]
fn a5_preferences_defaults() {
    let p = Preferences::default();
    assert_eq!(p.launch_panel_game_launch, MinimizationType::TaskBar);
    assert_eq!(p.launch_panel_minimized, MinimizationType::TaskBar);
    assert_eq!(p.look_and_feel, LookAndFeel::LafDefault);
    // The initializer is NONE (only the null getter gives AUTO).
    assert_eq!(p.icon_resize_size, IconResize::Disabled);
    assert_eq!(p.start_with_os, StartWithOs::Disabled);
    assert_eq!(
        p.check_repositories_frequency,
        CheckRepositoriesFrequency::Freq3
    );
    assert!(p.prefer_github_updates);
}

#[test]
fn a5_launcher_options_defaults() {
    let l = LauncherOptions::default();
    let expected = LauncherOptions {
        game_profile: None,
        show_script_errors: false,
        no_pause: false,
        file_patching: false,
        window_mode: false,
        check_signatures: false,
        auto_restart: false,
        launch_arma3_as_administrator: false,
        mission_file: false,
        mission_file_path: None,
        max_memory_selection: None,
        cpu_count_selection: 0,
        ex_threads_selection: None,
        malloc_selection: None,
        enable_ht: false,
        huge_pages: false,
        no_splash_screen: false,
        default_world: false,
        nologs: false,
        arma3_exe_path: None,
        steam_exe_path: None,
    };
    assert_eq!(l, expected);
}

#[test]
fn a5_other_types() {
    let addon = Addon::new("@ace", "@ace", "/games/arma3/@ace");
    assert!(!addon.at_arma3_install_root);
    assert_eq!(addon.key, "@ace");
    assert_eq!(FileAttributes::default().last_modified, 0);
    let app = ExternalApplication::default();
    assert!(!app.enable);
    assert_eq!(app.name, None);
}

fn dir(name: &str) -> TreeNode {
    TreeNode::Directory(TreeDirectory::new(name))
}

fn leaf(name: &str) -> TreeNode {
    TreeNode::Leaf(TreeLeaf::new(name))
}

fn names(dir: &TreeDirectory) -> Vec<&str> {
    dir.children.iter().map(TreeNode::name).collect()
}

#[test]
fn a6_add_child_orders_directories_then_leaves() {
    let mut root = TreeDirectory::new_root();
    root.add_child(leaf("b_leaf"));
    root.add_child(dir("b"));
    root.add_child(leaf("A_leaf"));
    root.add_child(dir("a"));
    root.add_child(dir("_x"));
    assert_eq!(names(&root), ["_x", "a", "b", "A_leaf", "b_leaf"]);
    assert!(!root.children[2].is_leaf());
    assert!(root.children[3].is_leaf());
}

#[test]
fn a6_add_child_is_stable_for_equal_names() {
    let mut root = TreeDirectory::new_root();
    let mut first = TreeDirectory::new("abc");
    first.marked = true;
    root.add_child(TreeNode::Directory(first));
    root.add_child(dir("ABC"));
    root.add_child(dir("Abc"));
    assert_eq!(names(&root), ["abc", "ABC", "Abc"]);
    let TreeNode::Directory(d) = &root.children[0] else {
        panic!("expected a directory");
    };
    assert!(d.marked);
}

#[test]
fn a6_remove_child_by_position() {
    let mut root = TreeDirectory::new_root();
    root.add_child(dir("a"));
    root.add_child(leaf("x"));
    root.add_child(leaf("y"));
    let removed = root.remove_child(1).unwrap();
    assert_eq!(removed.name(), "x");
    assert_eq!(names(&root), ["a", "y"]);
    assert!(root.remove_child(5).is_none());
    assert_eq!(root.directories().count(), 1);
    assert_eq!(root.leaves().count(), 1);
}

#[test]
fn a6_tree_node_accessors() {
    let mut node = leaf("x");
    assert!(!node.selected() && !node.optional());
    node.set_selected(true);
    node.set_optional(true);
    assert!(node.selected() && node.optional());
    let mut node = TreeNode::Directory(TreeDirectory {
        modset_type: Some(ModsetType::Event),
        ..TreeDirectory::new("e")
    });
    node.set_selected(true);
    assert!(node.selected() && !node.optional() && !node.is_leaf());
}

fn server(name: Option<&str>, port: i32) -> FavoriteServer {
    FavoriteServer {
        name: name.map(str::to_owned),
        port,
        ..Default::default()
    }
}

#[test]
fn a7_sort_favorite_servers() {
    let mut c = Configuration {
        favorite_servers: vec![
            server(Some("b"), 1),
            server(Some("ABC"), 2),
            server(None, 3),
            server(Some("abc"), 4),
            server(Some("_x"), 5),
            server(Some("A"), 6),
        ],
        ..Default::default()
    };
    c.sort_favorite_servers();
    let ports: Vec<i32> = c.favorite_servers.iter().map(|s| s.port).collect();
    // None sorts as "", equal names keep their order.
    assert_eq!(ports, [3, 5, 6, 2, 4, 1]);
}

#[test]
fn a7_external_application_order() {
    let app = |name: Option<&str>| ExternalApplication {
        name: name.map(str::to_owned),
        ..Default::default()
    };
    let mut apps = [app(Some("b")), app(Some("A")), app(None), app(Some("a"))];
    apps.sort_by(ExternalApplication::java_cmp);
    let names: Vec<Option<&str>> = apps.iter().map(|a| a.name.as_deref()).collect();
    assert_eq!(names, [None, Some("A"), Some("a"), Some("b")]);
}
