//! Run parameters: Java's exact order, `-mod=` grouping and splitting, renderings.

use a3s_addons::AddonRegistry;
use a3s_launch::params::{
    ModPathStyle, RunParameterInput, additional_parameters, determine_run_parameters,
    mod_parameters, run_parameters, run_parameters_display, selected_addon_names,
    shortcut_arguments, tokenize_parameters, wine_path,
};
use a3s_types::FavoriteServer;
use a3s_types::local::{Addon, Configuration, Profile, TreeDirectory, TreeLeaf, TreeNode};

const SLASH: ModPathStyle = ModPathStyle::Native { separator: '/' };

/// The Arma 3 folder of the tests, absolute on the host (the install-root test resolves it).
fn game_dir() -> &'static str {
    if cfg!(windows) {
        r"C:\games\Arma 3"
    } else {
        "/games/Arma 3"
    }
}

/// `game_dir` in other letter case.
fn game_dir_upper() -> &'static str {
    if cfg!(windows) {
        r"C:\games\ARMA 3"
    } else {
        "/games/ARMA 3"
    }
}

fn game_exe() -> String {
    format!("{}/arma3_x64.exe", game_dir())
}

fn params(profile: Option<&Profile>, configuration: &Configuration) -> Vec<String> {
    run_parameters(&RunParameterInput {
        profile,
        configuration,
        registry: &AddonRegistry::new(),
        mod_path_style: SLASH,
    })
}

fn server(name: &str, ip: &str, port: i32, password: &str) -> FavoriteServer {
    FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some(ip.to_owned()),
        port,
        password: Some(password.to_owned()),
        ..Default::default()
    }
}

fn leaf(name: &str, selected: bool) -> TreeNode {
    TreeNode::Leaf(TreeLeaf {
        selected,
        ..TreeLeaf::new(name)
    })
}

fn group(name: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode::Directory(TreeDirectory {
        children,
        ..TreeDirectory::new(name)
    })
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn every_option_in_java_order() {
    let mut profile = Profile::new("p");
    let options = &mut profile.launcher_options;
    options.malloc_selection = Some("tbb4malloc_bi_x64".to_owned());
    options.game_profile = Some("Player".to_owned());
    options.show_script_errors = true;
    options.no_pause = true;
    options.file_patching = true;
    options.window_mode = true;
    options.check_signatures = true;
    options.mission_file = true;
    options.mission_file_path = Some("/m/mission.sqm".to_owned());
    options.max_memory_selection = Some("8192".to_owned());
    options.cpu_count_selection = 8;
    options.ex_threads_selection = Some("7".to_owned());
    options.enable_ht = true;
    options.huge_pages = true;
    options.no_splash_screen = true;
    options.default_world = true;
    options.nologs = true;
    let configuration = Configuration {
        server_name: Some("WOG".to_owned()),
        favorite_servers: vec![
            server("Other", "5.5.5.5", 1, ""),
            server("WOG", "1.2.3.4", 2302, "pw"),
            server("WOG", "9.9.9.9", 1, ""),
        ],
        ..Default::default()
    };

    assert_eq!(
        params(Some(&profile), &configuration),
        strings(&[
            "-malloc=tbb4malloc_bi_x64",
            "-name=Player",
            "-showScriptErrors",
            "-noPause",
            "-filePatching",
            "-window",
            "-checkSignatures",
            "/m/mission.sqm",
            "-maxMem=8192",
            "-cpuCount=8",
            "-exThreads=7",
            "-enableHT",
            "-hugepages",
            "-nosplash",
            "-world=empty",
            "-nologs",
            "-connect=1.2.3.4",
            "-port=2302",
            "-password=pw",
        ])
    );
}

#[test]
fn defaults_and_edge_values() {
    let configuration = Configuration::default();
    assert!(params(None, &configuration).is_empty());
    assert!(params(Some(&Profile::new("p")), &configuration).is_empty());

    let mut profile = Profile::new("p");
    profile.launcher_options.mission_file = true;
    assert!(params(Some(&profile), &configuration).is_empty());
    profile.launcher_options.mission_file_path = Some(String::new());
    assert!(params(Some(&profile), &configuration).is_empty());

    let mut profile = Profile::new("p");
    profile.launcher_options.malloc_selection = Some(String::new());
    profile.launcher_options.cpu_count_selection = -1;
    assert_eq!(
        params(Some(&profile), &configuration),
        strings(&["-malloc=", "-cpuCount=-1"])
    );
}

#[test]
fn server_join() {
    let profile = Profile::new("p");
    let mut configuration = Configuration {
        server_name: Some("WOG".to_owned()),
        favorite_servers: vec![server("WOG", "1.2.3.4", 2302, "")],
        ..Default::default()
    };
    assert_eq!(
        params(Some(&profile), &configuration),
        strings(&["-connect=1.2.3.4", "-port=2302"])
    );

    configuration.server_name = Some("wog".to_owned());
    assert!(params(Some(&profile), &configuration).is_empty());

    // A server without a name never matches; one without an address is not joined; a missing
    // password gives no `-password=`.
    configuration.server_name = Some("WOG".to_owned());
    configuration.favorite_servers = vec![
        FavoriteServer {
            name: None,
            ..server("WOG", "7.7.7.7", 1, "")
        },
        FavoriteServer {
            password: None,
            ..server("WOG", "1.2.3.4", 2302, "")
        },
    ];
    assert_eq!(
        params(Some(&profile), &configuration),
        strings(&["-connect=1.2.3.4", "-port=2302"])
    );
    configuration.favorite_servers = vec![FavoriteServer {
        ip_address: None,
        ..server("WOG", "", 2302, "pw")
    }];
    assert!(params(Some(&profile), &configuration).is_empty());
}

#[test]
fn mods_in_priority_then_sorted_order_grouped_by_path() {
    let mut registry = AddonRegistry::new();
    let workshop = format!("{}/!Workshop", game_dir());
    for (key, name, path) in [
        ("@ace", "@ace", "/mods"),
        ("@ace*", "@ace", "/mods2"),
        ("@tfar", "@TFAR", "/mods"),
        ("@cba", "@cba", workshop.as_str()),
        ("gm", "GM", game_dir()),
        ("@rel", "@rel", ""),
        ("vn", "vn", game_dir_upper()),
    ] {
        registry.insert(Addon::new(key, name, path));
    }
    let mut profile = Profile::new("p");
    profile.launcher_options.arma3_exe_path = Some(game_exe());
    profile.addon_names_by_priority = strings(&["gm", "@tfar", "@ace", "@missing"]);
    profile.tree.children = vec![
        group(
            "one",
            vec![
                leaf("@ACE", true),
                leaf("@TFAR", true),
                leaf("GM", true),
                leaf("vn", true),
                leaf("@cba", true),
            ],
        ),
        group(
            "two",
            vec![
                leaf("@cba", true),
                leaf("@rel", true),
                leaf("@ace*", true),
                leaf("@unknown", true),
                leaf("@off", false),
            ],
        ),
    ];

    let mods = run_parameters(&RunParameterInput {
        profile: Some(&profile),
        configuration: &Configuration::default(),
        registry: &registry,
        mod_path_style: SLASH,
    });
    assert_eq!(
        mods,
        vec![
            "-mod=GM;".to_owned(),
            "-mod=/mods/@TFAR;".to_owned(),
            "-mod=/mods/@ace;".to_owned(),
            "-mod=/mods2/@ace;".to_owned(),
            format!("-mod={workshop}/@cba;"),
            "-mod=@rel;".to_owned(),
            "-mod=vn;".to_owned(),
        ]
    );
}

#[test]
fn selected_names_ignore_group_selection() {
    let mut tree = TreeDirectory::new_root();
    tree.children = vec![
        TreeNode::Directory(TreeDirectory {
            selected: true,
            children: vec![leaf("b", false), leaf("c", true)],
            ..TreeDirectory::new("g")
        }),
        leaf("a", true),
    ];
    assert_eq!(selected_addon_names(&tree), strings(&["c", "a"]));
}

fn addon(name: &str, path: &str) -> Addon {
    Addon::new(name.to_lowercase(), name, path)
}

#[test]
fn install_root_groups() {
    let exe = game_exe();
    let exe = Some(exe.as_str());
    let adjacent = [addon("GM", game_dir()), addon("vn", game_dir())];
    assert_eq!(
        mod_parameters(&adjacent, exe, SLASH),
        strings(&["-mod=GM;vn;"])
    );

    let separated = [
        addon("GM", game_dir()),
        addon("@x", "/m"),
        addon("vn", game_dir()),
    ];
    assert_eq!(
        mod_parameters(&separated, exe, SLASH),
        strings(&["-mod=GM;", "-mod=/m/@x;", "-mod=vn;"])
    );

    // Without an executable folder no addon is root-relative.
    for exe in [None, Some(""), Some("arma3.exe")] {
        assert_eq!(
            mod_parameters(&[addon("GM", game_dir())], exe, SLASH),
            vec![format!("-mod={}/GM;", game_dir())]
        );
    }
}

#[test]
fn separator_and_literal_split() {
    let backslash = ModPathStyle::Native { separator: '\\' };
    assert_eq!(
        mod_parameters(&[addon("@a", r"C:\m")], None, backslash),
        strings(&[r"-mod=C:\m\@a;"])
    );
    assert_eq!(
        mod_parameters(&[addon("@a-mod=b", "/m")], None, SLASH),
        strings(&["-mod=/m/@a", "-mod=b;"])
    );
}

#[test]
fn wine_paths() {
    assert_eq!(wine_path("/home/u/mods"), r"Z:\home\u\mods");
    assert_eq!(wine_path("relative"), "relative");

    let exe = game_exe();
    let mods = [addon("@ace", "/mods"), addon("GM", game_dir())];
    assert_eq!(
        mod_parameters(&mods, Some(&exe), ModPathStyle::WineZDrive),
        strings(&[r"-mod=Z:\mods\@ace;", "-mod=GM;"])
    );
}

#[test]
fn additional_parameters_come_first_with_malloc_tokens_in_front() {
    let mut profile = Profile::new("p");
    profile.additional_parameters = Some("-world=empty  -malloc=a -x -MALLOC=b \t-z".to_owned());
    profile.launcher_options.malloc_selection = Some("tbb".to_owned());
    let all = determine_run_parameters(&RunParameterInput {
        profile: Some(&profile),
        configuration: &Configuration::default(),
        registry: &AddonRegistry::new(),
        mod_path_style: SLASH,
    });
    assert_eq!(
        all,
        strings(&[
            "-MALLOC=b",
            "-malloc=a",
            "-world=empty",
            "-x",
            "-z",
            "-malloc=tbb"
        ])
    );

    for text in [None, Some(""), Some("   ")] {
        profile.additional_parameters = text.map(str::to_owned);
        assert!(additional_parameters(Some(&profile)).is_empty());
    }
    assert!(additional_parameters(None).is_empty());

    assert_eq!(tokenize_parameters("  a  b\tc "), strings(&["a", "b\tc"]));
    assert_eq!(tokenize_parameters("a \t b"), strings(&["a", "", "b"]));
}

#[test]
fn renderings() {
    let params = strings(&["-a", "-b"]);
    assert_eq!(run_parameters_display(&params), "-a\n-b\n");
    assert_eq!(shortcut_arguments(&params), " -a -b");
    assert_eq!(run_parameters_display(&[]), "");
    assert_eq!(shortcut_arguments(&[]), "");
}
