//! Lenient decoding of the Java client's local files.
//!
//! No Java-written fixtures exist yet for these files, so the streams are built with
//! `a3s_jserial::jdk::Builder` from the class descriptors of the Java sources (names, UIDs and
//! fields in wire order), including historical variants.

use std::collections::{BTreeMap, BTreeSet};

use a3s_jserial::jdk::{Builder, FieldSpec, block_data};
use a3s_jserial::{ClassData, Content, FieldType, NodeId, Value, flags};
use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, ModsetType, StartWithOs,
};
use a3s_types::local::{
    Acre2Options, AcreOptions, BikeyExtractOptions, Configuration, ExternalApplication,
    FileAttributes, LauncherOptions, LegacyConfiguration, LegacyDecode, LegacyRepository,
    Preferences, Profile, Proxy, Repository, RptOptions, TfarOptions, TreeDirectory, TreeLeaf,
    TreeNode, decode_legacy_stream,
};
use a3s_types::{
    EncryptionMode, Error, FavoriteServer, JavaCodec as _, Protocol, ProtocolType, wire,
};

// ---------------------------------------------------------------------------------------------
// Java-like graph building

const S: u8 = flags::SERIALIZABLE;
const STR: &str = "Ljava/lang/String;";
const LIST: &str = "Ljava/util/List;";
const SET: &str = "Ljava/util/Set;";
const MAP: &str = "Ljava/util/Map;";
const AP: &str = "Lfr/soe/a3s/domain/AbstractProtocole;";
const TREE_DIRECTORY_SIG: &str = "Lfr/soe/a3s/domain/TreeDirectory;";
const LAUNCHER_OPTIONS_SIG: &str = "Lfr/soe/a3s/domain/configration/LauncherOptions;";

const CONFIGURATION: &str = "fr.soe.a3s.domain.configration.Configuration";
const LAUNCHER_OPTIONS: &str = "fr.soe.a3s.domain.configration.LauncherOptions";
const PREFERENCES: &str = "fr.soe.a3s.domain.Preferences";
const PROFILE: &str = "fr.soe.a3s.domain.Profile";
const TREE_DIRECTORY: &str = "fr.soe.a3s.domain.TreeDirectory";
const TREE_LEAF: &str = "fr.soe.a3s.domain.TreeLeaf";
const REPOSITORY: &str = "fr.soe.a3s.domain.repository.Repository";
const FILE_ATTRIBUTES: &str = "fr.soe.a3s.domain.repository.FileAttributes";
const MODSET_TYPE: &str = "fr.soe.a3s.constant.ModsetType";

/// A field descriptor and its value.
type F = (FieldSpec<'static>, Value);

fn z(name: &'static str, v: bool) -> F {
    (FieldSpec::prim(FieldType::Boolean, name), Value::Boolean(v))
}

fn int(name: &'static str, v: i32) -> F {
    (FieldSpec::prim(FieldType::Int, name), Value::Int(v))
}

fn long(name: &'static str, v: i64) -> F {
    (FieldSpec::prim(FieldType::Long, name), Value::Long(v))
}

fn double(name: &'static str, v: f64) -> F {
    (FieldSpec::prim(FieldType::Double, name), Value::Double(v))
}

fn obj(name: &'static str, signature: &'static str, v: Value) -> F {
    (FieldSpec::object(name, signature), v)
}

fn s(b: &mut Builder, name: &'static str, v: Option<&str>) -> F {
    obj(name, STR, b.opt_string(v))
}

fn en(b: &mut Builder, name: &'static str, class: &str, constant: &str) -> F {
    let signature: &'static str =
        Box::leak(format!("L{};", class.replace('.', "/")).into_boxed_str());
    obj(name, signature, b.enum_value(class, constant))
}

/// An object of a class without serializable superclass; fields named in `omit` are left out
/// of the descriptor (older class versions).
fn object(b: &mut Builder, class: &str, uid: i64, fields: Vec<F>, omit: &[&str]) -> Value {
    let fields: Vec<F> = fields
        .into_iter()
        .filter(|(f, _)| !omit.contains(&f.name))
        .collect();
    let specs: Vec<FieldSpec<'_>> = fields.iter().map(|(f, _)| *f).collect();
    let desc = b.class_desc(class, uid, S, &specs, None);
    b.simple_object(desc, fields.into_iter().map(|(_, v)| v).collect())
}

fn string_list(b: &mut Builder, strings: &[String]) -> Value {
    let values = strings.iter().map(|s| b.string(s)).collect();
    b.array_list(values)
}

/// `new TreeSet<String>()` with `elements` added (null comparator).
fn tree_set(b: &mut Builder, elements: &[String]) -> Value {
    let class = b.class_desc(
        "java.util.TreeSet",
        -2_479_143_000_061_671_589,
        S | flags::WRITE_METHOD,
        &[],
        None,
    );
    let id = b.reserve_object(class);
    let mut annotations = vec![Content::Value(Value::Null)];
    let len = i32::try_from(elements.len()).unwrap();
    annotations.extend(block_data(&len.to_be_bytes()));
    for e in elements {
        let v = b.string(e);
        annotations.push(Content::Value(v));
    }
    b.set_object_data(
        id,
        vec![ClassData {
            values: Vec::new(),
            annotations,
        }],
    );
    Value::Ref(id)
}

fn finish(b: Builder, root: Value) -> Vec<u8> {
    a3s_jserial::write(&b.finish(root)).unwrap()
}

fn decode<T: LegacyDecode>(bytes: &[u8]) -> (T, Vec<String>) {
    let legacy = decode_legacy_stream::<T>(bytes).unwrap();
    (legacy.value, legacy.warnings)
}

// ---------------------------------------------------------------------------------------------
// Encoders mirroring the Java classes

fn enc_launcher_options(b: &mut Builder, l: &LauncherOptions, omit: &[&str]) -> Value {
    let fields = vec![
        z("autoRestart", l.auto_restart),
        z("checkSignatures", l.check_signatures),
        int("cpuCountSelection", l.cpu_count_selection),
        z("defaultWorld", l.default_world),
        z("enableHT", l.enable_ht),
        z("filePatching", l.file_patching),
        z("hugePages", l.huge_pages),
        z(
            "launchArma3AsAdministrator",
            l.launch_arma3_as_administrator,
        ),
        z("missionFile", l.mission_file),
        z("noFilePatching", false),
        z("noPause", l.no_pause),
        z("noSplashScreen", l.no_splash_screen),
        z("nologs", l.nologs),
        z("runBeta", false),
        z("showScriptErrors", l.show_script_errors),
        z("windowMode", l.window_mode),
        z("xpCompatibilityMode", false),
        s(b, "arma3ExePath", l.arma3_exe_path.as_deref()),
        s(b, "exThreadsSelection", l.ex_threads_selection.as_deref()),
        s(b, "gameProfile", l.game_profile.as_deref()),
        s(b, "mallocSelection", l.malloc_selection.as_deref()),
        s(b, "maxMemorySelection", l.max_memory_selection.as_deref()),
        s(b, "missionFilePath", l.mission_file_path.as_deref()),
        s(b, "steamExePath", l.steam_exe_path.as_deref()),
    ];
    object(b, LAUNCHER_OPTIONS, 7_112_913_210_279_158_291, fields, omit)
}

fn enc_tree_leaf(b: &mut Builder, l: &TreeLeaf, parent: NodeId, omit: &[&str]) -> Value {
    let fields = vec![
        z("duplicate", false),
        z("optional", l.optional),
        z("selected", l.selected),
        s(b, "dlcAppId", l.dlc_app_id.as_deref()),
        s(b, "name", Some(&l.name)),
        obj("parent", TREE_DIRECTORY_SIG, Value::Ref(parent)),
        s(b, "workshopId", l.workshop_id.as_deref()),
    ];
    object(b, TREE_LEAF, 8_681_722_025_933_532_378, fields, omit)
}

fn enc_tree_dir(
    b: &mut Builder,
    d: &TreeDirectory,
    parent: Option<NodeId>,
    leaf_omit: &[&str],
) -> Value {
    let class = b.class_desc(
        TREE_DIRECTORY,
        -3_917_102_856_123_611_985,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "marked"),
            FieldSpec::prim(FieldType::Boolean, "optional"),
            FieldSpec::prim(FieldType::Boolean, "selected"),
            FieldSpec::object("list", LIST),
            FieldSpec::object("modsetRepositoryName", STR),
            FieldSpec::object("modsetType", "Lfr/soe/a3s/constant/ModsetType;"),
            FieldSpec::object("name", STR),
            FieldSpec::object("parent", TREE_DIRECTORY_SIG),
        ],
        None,
    );
    let id = b.reserve_object(class);
    let children = d
        .children
        .iter()
        .map(|c| match c {
            TreeNode::Directory(child) => enc_tree_dir(b, child, Some(id), leaf_omit),
            TreeNode::Leaf(leaf) => enc_tree_leaf(b, leaf, id, leaf_omit),
        })
        .collect();
    let list = b.array_list(children);
    let repo = b.opt_string(d.modset_repository_name.as_deref());
    let modset = d
        .modset_type
        .map_or(Value::Null, |t| b.enum_value(MODSET_TYPE, t.java_name()));
    let name = b.string(&d.name);
    b.set_object_data(
        id,
        vec![ClassData {
            values: vec![
                Value::Boolean(d.marked),
                Value::Boolean(d.optional),
                Value::Boolean(d.selected),
                list,
                repo,
                modset,
                name,
                parent.map_or(Value::Null, Value::Ref),
            ],
            annotations: Vec::new(),
        }],
    );
    Value::Ref(id)
}

fn enc_profile(b: &mut Builder, p: &Profile, leaf_omit: &[&str]) -> Value {
    let fields = vec![
        s(
            b,
            "additionalParameters",
            p.additional_parameters.as_deref(),
        ),
        obj(
            "addonNamesByPriority",
            LIST,
            string_list(b, &p.addon_names_by_priority),
        ),
        obj(
            "addonSearchDirectories",
            LIST,
            string_list(b, &p.addon_search_directories),
        ),
        s(
            b,
            "lastAddedAddonSearchDirectory",
            p.last_added_addon_search_directory.as_deref(),
        ),
        obj(
            "launcherOptions",
            LAUNCHER_OPTIONS_SIG,
            enc_launcher_options(b, &p.launcher_options, &[]),
        ),
        s(b, "name", Some(&p.name)),
        obj(
            "tree",
            TREE_DIRECTORY_SIG,
            enc_tree_dir(b, &p.tree, None, leaf_omit),
        ),
    ];
    object(b, PROFILE, -4_395_099_790_449_721_021, fields, &[])
}

fn enc_preferences(b: &mut Builder, p: &Preferences, omit: &[&str]) -> Value {
    let prefer = b.boolean(p.prefer_github_updates);
    let fields = vec![
        en(
            b,
            "checkRepositoriesFrequency",
            "fr.soe.a3s.constant.CheckRepositoriesFrequency",
            p.check_repositories_frequency.java_name(),
        ),
        en(
            b,
            "iconResizeSize",
            "fr.soe.a3s.constant.IconResize",
            p.icon_resize_size.java_name(),
        ),
        en(
            b,
            "launchPanelGameLaunch",
            "fr.soe.a3s.constant.MinimizationType",
            p.launch_panel_game_launch.java_name(),
        ),
        en(
            b,
            "launchPanelMinimized",
            "fr.soe.a3s.constant.MinimizationType",
            p.launch_panel_minimized.java_name(),
        ),
        en(
            b,
            "lookAndFeel",
            "fr.soe.a3s.constant.LookAndFeel",
            p.look_and_feel.java_name(),
        ),
        obj("preferGitHubUpdates", "Ljava/lang/Boolean;", prefer),
        en(
            b,
            "startWithOS",
            "fr.soe.a3s.constant.StartWithOS",
            p.start_with_os.java_name(),
        ),
    ];
    object(b, PREFERENCES, 7_034_433_806_261_372_240, fields, omit)
}

fn enc_external_application(b: &mut Builder, a: &ExternalApplication) -> Value {
    let fields = vec![
        z("enable", a.enable),
        s(b, "executablePath", a.executable_path.as_deref()),
        s(b, "name", a.name.as_deref()),
        s(b, "parameters", a.parameters.as_deref()),
    ];
    object(
        b,
        "fr.soe.a3s.domain.configration.ExternalApplication",
        6_695_449_091_678_057_038,
        fields,
        &[],
    )
}

/// The configuration class; `extra` fields are appended (they are looked up by name).
fn enc_configuration(
    b: &mut Builder,
    c: &LegacyConfiguration,
    extra: Vec<F>,
    omit: &[&str],
) -> Value {
    let cfg = &c.configuration;
    let acre2 = vec![
        s(
            b,
            "acre2PluginPath",
            cfg.acre2_options.acre2_plugin_path.as_deref(),
        ),
        s(b, "ts3Path", cfg.acre2_options.ts3_path.as_deref()),
    ];
    let acre2 = object(
        b,
        "fr.soe.a3s.domain.configration.Acre2Options",
        -8_995_842_831_822_253_145,
        acre2,
        &[],
    );
    let acre = vec![
        s(
            b,
            "acrePluginPath",
            cfg.acre_options.acre_plugin_path.as_deref(),
        ),
        s(
            b,
            "acreUserconfigPath",
            cfg.acre_options.acre_userconfig_path.as_deref(),
        ),
        s(b, "ts3Path", cfg.acre_options.ts3_path.as_deref()),
    ];
    let acre = object(
        b,
        "fr.soe.a3s.domain.configration.AcreOptions",
        -1_596_307_340_587_244_621,
        acre,
        &[],
    );
    let search = c
        .addon_search_directory_paths
        .as_ref()
        .map_or(Value::Null, |p| tree_set(b, p));
    let bikey = vec![
        s(
            b,
            "sourceDirectoryPath",
            cfg.bikey_extract_options.source_directory_path.as_deref(),
        ),
        s(
            b,
            "targetDirectoryPath",
            cfg.bikey_extract_options.target_directory_path.as_deref(),
        ),
    ];
    let bikey = object(
        b,
        "fr.soe.a3s.domain.configration.BikeyExtractOptions",
        12345,
        bikey,
        &[],
    );
    let apps = cfg
        .external_applications
        .iter()
        .map(|a| enc_external_application(b, a))
        .collect();
    let apps = b.array_list(apps);
    let servers = cfg.favorite_servers.iter().map(|s| s.encode(b)).collect();
    let servers = b.array_list(servers);
    let launcher = c
        .launcher_options
        .as_ref()
        .map_or(Value::Null, |l| enc_launcher_options(b, l, &[]));
    let proxy_protocol = cfg
        .proxy
        .proxy_protocol
        .as_ref()
        .map_or(Value::Null, |p| p.encode(b));
    let proxy = vec![
        z("enableProxy", cfg.proxy.enable_proxy),
        obj("proxyProtocol", AP, proxy_protocol),
    ];
    let proxy = object(
        b,
        "fr.soe.a3s.domain.configration.Proxy",
        -6_891_013_124_542_313_138,
        proxy,
        &[],
    );
    let rpt = vec![s(b, "rptPath", cfg.rpt_options.rpt_path.as_deref())];
    let rpt = object(
        b,
        "fr.soe.a3s.domain.configration.RptOptions",
        7_740_252_023_809_740_200,
        rpt,
        &[],
    );
    let tfar = vec![
        s(
            b,
            "tfarPluginPath",
            cfg.tfar_options.tfar_plugin_path.as_deref(),
        ),
        s(
            b,
            "tfarUserconfigPath",
            cfg.tfar_options.tfar_userconfig_path.as_deref(),
        ),
        s(b, "ts3Path", cfg.tfar_options.ts3_path.as_deref()),
    ];
    let tfar = object(
        b,
        "fr.soe.a3s.domain.configration.TfarOptions",
        -9_099_384_809_608_223_793,
        tfar,
        &[],
    );
    let p = "Lfr/soe/a3s/domain/configration/";
    let sig = |class: &str| -> &'static str { Box::leak(format!("{p}{class};").into_boxed_str()) };
    let mut fields = vec![
        int("height", cfg.height),
        z("viewModeTree", cfg.view_mode_tree),
        int("width", cfg.width),
        obj("acre2Options", sig("Acre2Options"), acre2),
        obj("acreOptions", sig("AcreOptions"), acre),
        obj("addonSearchDirectoryPaths", SET, search),
        obj("bikeyExtractOptions", sig("BikeyExtractOptions"), bikey),
        s(b, "defaultModset", cfg.default_modset.as_deref()),
        obj("externalApplications", LIST, apps),
        obj("favoriteServers", LIST, servers),
        s(b, "gameVersion", Some(&cfg.game_version)),
        obj("launcherOptions", sig("LauncherOptions"), launcher),
        s(b, "profileName", Some(&cfg.profile_name)),
        obj("proxy", sig("Proxy"), proxy),
        obj("rptOptions", sig("RptOptions"), rpt),
        s(b, "serverName", cfg.server_name.as_deref()),
        obj("tfarOptions", sig("TfarOptions"), tfar),
    ];
    fields.extend(extra);
    object(b, CONFIGURATION, -8_310_476_744_472_497_506, fields, omit)
}

fn enc_file_cache(b: &mut Builder, cache: &BTreeMap<String, FileAttributes>) -> Value {
    let entries: Vec<(String, Value)> = cache
        .iter()
        .map(|(k, v)| {
            let fields = vec![
                long("lastModified", v.last_modified),
                s(b, "sha1", Some(&v.sha1)),
            ];
            let value = object(b, FILE_ATTRIBUTES, -2_500_913_557_576_506_383, fields, &[]);
            (k.clone(), value)
        })
        .collect();
    b.hash_map_str(&entries)
}

fn hash_set(b: &mut Builder, set: &BTreeSet<String>) -> Value {
    let elements: Vec<String> = set.iter().cloned().collect();
    b.hash_set_str(&elements)
}

fn enc_repository(b: &mut Builder, l: &LegacyRepository, omit: &[&str]) -> Value {
    let r = &l.repository;
    let opt_protocol =
        |b: &mut Builder, p: &Option<Protocol>| p.as_ref().map_or(Value::Null, |p| p.encode(b));
    let bit_torrent = opt_protocol(b, &r.bit_torrent_protocol);
    let upload = opt_protocol(b, &r.upload_protocol);
    let protocol = r.protocol.encode(b);
    let servers = r
        .favorite_servers_set_to_autoconfig
        .iter()
        .map(|s| s.encode(b))
        .collect();
    let servers = b.array_list(servers);
    let events: Vec<(String, Value)> = r
        .events_download_location
        .iter()
        .map(|(k, v)| (k.clone(), b.string(v)))
        .collect();
    let fields = vec![
        z("auto", r.auto),
        z("compressed", r.compressed),
        z("exactMatch", r.exact_match),
        double(
            "maximumClientDownloadSpeed",
            r.maximum_client_download_speed,
        ),
        z("noAutoDiscover", r.no_auto_discover),
        z("noPartialFileTransfer", r.no_partial_file_transfer),
        z("notify", r.notify),
        int("numberOfClientConnections", r.number_of_client_connections),
        int("numberOfConnections", r.number_of_connections),
        int(
            "numberOfRepositoryCheckConnections",
            r.number_of_repository_check_connections,
        ),
        z("outOfSynk", false),
        int("parallelUploadConnections", r.parallel_upload_connections),
        int("revision", r.revision),
        z(
            "uploadCompressedPboFilesOnly",
            r.upload_compressed_pbo_files_only,
        ),
        s(b, "autoConfigURL", r.auto_config_url.as_deref()),
        obj("bitTorrentProtocole", AP, bit_torrent),
        s(
            b,
            "defaultDownloadLocation",
            r.default_download_location.as_deref(),
        ),
        obj(
            "excludedFilesFromBuild",
            SET,
            hash_set(b, &r.excluded_files_from_build),
        ),
        obj(
            "excludedFoldersFromSync",
            SET,
            hash_set(b, &r.excluded_folders_from_sync),
        ),
        obj("favoriteServersSetToAutoconfig", LIST, servers),
        obj(
            "hidddenFolderPaths",
            SET,
            hash_set(b, &r.hidden_folder_paths),
        ),
        obj("mapEventsDownloadLocation", MAP, b.hash_map_str(&events)),
        obj(
            "mapFilesForBuild",
            MAP,
            enc_file_cache(b, &l.files_for_build),
        ),
        obj("mapFilesForSync", MAP, enc_file_cache(b, &l.files_for_sync)),
        s(b, "name", Some(&r.name)),
        s(b, "path", r.path.as_deref()),
        obj("protocole", AP, protocol),
        obj("uploadProtocole", AP, upload),
    ];
    object(b, REPOSITORY, -8_142_021_113_361_619_970, fields, omit)
}

// ---------------------------------------------------------------------------------------------
// Sample values

fn launcher_options() -> LauncherOptions {
    LauncherOptions {
        game_profile: Some("Player".to_owned()),
        show_script_errors: true,
        no_pause: true,
        file_patching: false,
        window_mode: true,
        check_signatures: true,
        auto_restart: false,
        launch_arma3_as_administrator: true,
        mission_file: true,
        mission_file_path: Some("/m.pbo".to_owned()),
        max_memory_selection: Some("2047".to_owned()),
        cpu_count_selection: 8,
        ex_threads_selection: Some("7".to_owned()),
        malloc_selection: Some("tbb4malloc_bi".to_owned()),
        enable_ht: true,
        huge_pages: true,
        no_splash_screen: true,
        default_world: true,
        nologs: true,
        arma3_exe_path: Some("C:\\Arma 3\\arma3_x64.exe".to_owned()),
        steam_exe_path: Some("C:\\Steam\\steam.exe".to_owned()),
    }
}

fn server(name: &str, port: i32) -> FavoriteServer {
    FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some("10.0.0.1".to_owned()),
        port,
        password: Some("pw".to_owned()),
        selected: port % 2 == 0,
        modset_name: Some("Modset".to_owned()),
        repository_name: Some("Repo".to_owned()),
    }
}

fn legacy_configuration() -> LegacyConfiguration {
    let mut socks = Protocol::new(ProtocolType::Socks5, "proxy.local", "1080", "u", "p", false);
    socks.encryption_mode = Some(EncryptionMode::NoEncryption);
    LegacyConfiguration {
        configuration: Configuration {
            height: 600,
            width: 800,
            profile_name: "Main".to_owned(),
            game_version: "Arma 3".to_owned(),
            server_name: Some("Server".to_owned()),
            default_modset: Some("Mod".to_owned()),
            view_mode_tree: false,
            // Sorted, as Java's getter observes them.
            favorite_servers: vec![server("alpha", 2302), server("Bravo", 2303)],
            external_applications: vec![ExternalApplication {
                name: Some("TeamSpeak".to_owned()),
                executable_path: Some("/opt/ts3".to_owned()),
                parameters: Some("-nosplash".to_owned()),
                enable: true,
            }],
            acre_options: AcreOptions {
                ts3_path: Some("/ts3".to_owned()),
                acre_plugin_path: Some("/acre".to_owned()),
                acre_userconfig_path: Some("/acre/cfg".to_owned()),
            },
            acre2_options: Acre2Options {
                ts3_path: Some("/ts3b".to_owned()),
                acre2_plugin_path: Some("/acre2".to_owned()),
            },
            tfar_options: TfarOptions {
                ts3_path: Some("/ts3c".to_owned()),
                tfar_plugin_path: Some("/tfar".to_owned()),
                tfar_userconfig_path: None,
            },
            rpt_options: RptOptions {
                rpt_path: Some("/rpt".to_owned()),
            },
            bikey_extract_options: BikeyExtractOptions {
                source_directory_path: Some("/src".to_owned()),
                target_directory_path: Some("/dst".to_owned()),
            },
            proxy: Proxy {
                enable_proxy: true,
                proxy_protocol: Some(socks),
            },
        },
        addon_search_directory_paths: Some(vec!["/addons/a".to_owned(), "/addons/b".to_owned()]),
        launcher_options: Some(launcher_options()),
    }
}

fn preferences() -> Preferences {
    Preferences {
        launch_panel_game_launch: MinimizationType::Tray,
        launch_panel_minimized: MinimizationType::Nothing,
        look_and_feel: LookAndFeel::LafMetal,
        icon_resize_size: IconResize::Size3,
        start_with_os: StartWithOs::Enabled,
        check_repositories_frequency: CheckRepositoriesFrequency::Freq1,
        prefer_github_updates: false,
    }
}

fn profile() -> Profile {
    let leaf = |name: &str, workshop: Option<&str>, dlc: Option<&str>| {
        TreeNode::Leaf(TreeLeaf {
            name: name.to_owned(),
            selected: workshop.is_some(),
            optional: dlc.is_some(),
            workshop_id: workshop.map(str::to_owned),
            dlc_app_id: dlc.map(str::to_owned),
            missing: false,
        })
    };
    let nested = TreeDirectory {
        name: "Nested".to_owned(),
        marked: true,
        children: vec![leaf("@cba_a3", Some("450814997"), None)],
        ..Default::default()
    };
    let event = TreeDirectory {
        name: "Event".to_owned(),
        selected: true,
        modset_type: Some(ModsetType::Event),
        modset_repository_name: Some("Repo".to_owned()),
        children: vec![
            TreeNode::Directory(nested),
            leaf("GM", None, Some("1042220")),
        ],
        ..Default::default()
    };
    let repo = TreeDirectory {
        name: "Repo".to_owned(),
        optional: true,
        modset_type: Some(ModsetType::Repository),
        children: vec![leaf("@ace", None, None)],
        ..Default::default()
    };
    Profile {
        name: "Main".to_owned(),
        tree: TreeDirectory {
            children: vec![
                TreeNode::Directory(event),
                TreeNode::Directory(repo),
                leaf("@tfar", Some("894678801"), None),
            ],
            ..TreeDirectory::new_root()
        },
        additional_parameters: Some("-world=empty".to_owned()),
        addon_names_by_priority: vec!["@cba_a3".to_owned(), "@ace".to_owned()],
        launcher_options: launcher_options(),
        addon_search_directories: vec!["/games/arma3".to_owned(), "/mods".to_owned()],
        last_added_addon_search_directory: Some("/mods".to_owned()),
    }
}

fn legacy_repository() -> LegacyRepository {
    let mut repository = Repository::new(
        "WOG",
        Protocol::new(
            ProtocolType::Http,
            "repo.example/wog",
            "80",
            "anonymous",
            "",
            false,
        ),
    );
    let mut upload = Protocol::new(
        ProtocolType::Sftp,
        "upload.example/wog",
        "22",
        "admin",
        "pw",
        true,
    );
    upload.read_timeout = None;
    repository.upload_protocol = Some(upload);
    repository.bit_torrent_protocol = Some(Protocol::new(
        ProtocolType::Ftps,
        "bt.example",
        "21",
        "a",
        "b",
        true,
    ));
    repository.revision = 128;
    repository.notify = true;
    repository.auto = true;
    repository.hidden_folder_paths = ["@hidden".to_owned(), "x/y".to_owned()].into();
    repository.no_auto_discover = false;
    repository.exact_match = true;
    repository.default_download_location = Some("/games/arma3".to_owned());
    repository.events_download_location = [
        ("Event A".to_owned(), "/a".to_owned()),
        ("Event B".to_owned(), "/b".to_owned()),
    ]
    .into();
    repository.number_of_client_connections = 3;
    repository.number_of_repository_check_connections = 2;
    repository.maximum_client_download_speed = 2.5;
    repository.upload_compressed_pbo_files_only = true;
    repository.parallel_upload_connections = 7;
    repository.path = Some("/srv/wog".to_owned());
    repository.number_of_connections = 0;
    repository.compressed = true;
    repository.no_partial_file_transfer = true;
    repository.auto_config_url = Some("ftp://repo/.a3s/autoconfig".to_owned());
    repository.favorite_servers_set_to_autoconfig = vec![server("Main", 2302)];
    repository.excluded_files_from_build = ["*.bak".to_owned()].into();
    repository.excluded_folders_from_sync = ["@tmp".to_owned()].into();
    let entry = |sha1: &str, last_modified| FileAttributes {
        sha1: sha1.to_owned(),
        last_modified,
    };
    LegacyRepository {
        repository,
        files_for_sync: [
            (
                "/games/arma3/@ace/mod.cpp".to_owned(),
                entry("0", 1_600_000_000_000),
            ),
            (
                "/games/arma3/@ace/addons/ace_main.pbo".to_owned(),
                entry(
                    "da39a3ee5e6b4b0d3255bfef95601890afd80709",
                    1_700_000_000_123,
                ),
            ),
        ]
        .into(),
        files_for_build: [("/srv/wog/a.pbo".to_owned(), entry("abc", -1))].into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Current-version files

#[test]
fn configuration_current() {
    let expected = legacy_configuration();
    let mut unsorted = expected.clone();
    unsorted.configuration.favorite_servers.reverse();
    let mut b = Builder::new();
    let root = enc_configuration(&mut b, &unsorted, Vec::new(), &[]);
    let (decoded, warnings) = decode::<LegacyConfiguration>(&finish(b, root));
    assert_eq!(warnings, Vec::<String>::new());
    assert_eq!(decoded, expected);
}

#[test]
fn preferences_current() {
    let expected = preferences();
    let mut b = Builder::new();
    let root = enc_preferences(&mut b, &expected, &[]);
    let (decoded, warnings) = decode::<Preferences>(&finish(b, root));
    assert_eq!(warnings, Vec::<String>::new());
    assert_eq!(decoded, expected);
}

#[test]
fn profile_current() {
    let expected = profile();
    let mut b = Builder::new();
    let root = enc_profile(&mut b, &expected, &[]);
    let (decoded, warnings) = decode::<Profile>(&finish(b, root));
    assert_eq!(warnings, Vec::<String>::new());
    assert_eq!(decoded, expected);
}

#[test]
fn repository_current() {
    let expected = legacy_repository();
    let mut b = Builder::new();
    let root = enc_repository(&mut b, &expected, &[]);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert_eq!(warnings, Vec::<String>::new());
    assert_eq!(decoded, expected);
    assert_eq!(decoded.repository.parallel_upload_connections(), 7);
    assert_eq!(decoded.repository.repository_check_connections(), 2);
}

// ---------------------------------------------------------------------------------------------
// Historical variants

#[test]
fn repository_without_connection_fields() {
    let mut b = Builder::new();
    let root = enc_repository(
        &mut b,
        &legacy_repository(),
        &[
            "numberOfRepositoryCheckConnections",
            "parallelUploadConnections",
        ],
    );
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    let r = decoded.repository;
    // Java zero values, normalised by the getters.
    assert_eq!(r.number_of_repository_check_connections, 0);
    assert_eq!(r.parallel_upload_connections, 0);
    assert_eq!(r.repository_check_connections(), 4);
    assert_eq!(r.parallel_upload_connections(), 4);
}

#[test]
fn repository_without_no_auto_discover() {
    let mut expected = legacy_repository();
    expected.repository.no_auto_discover = true;
    let mut b = Builder::new();
    let root = enc_repository(&mut b, &expected, &["noAutoDiscover"]);
    let (decoded, _) = decode::<LegacyRepository>(&finish(b, root));
    // Field initializers don't run on deserialization: the zero value, not `true`.
    assert!(!decoded.repository.no_auto_discover);
}

#[test]
fn preferences_without_prefer_github_updates() {
    let mut b = Builder::new();
    let root = enc_preferences(&mut b, &preferences(), &["preferGitHubUpdates"]);
    let (decoded, warnings) = decode::<Preferences>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(decoded.prefer_github_updates);
    assert_eq!(decoded.look_and_feel, LookAndFeel::LafMetal);
}

#[test]
fn preferences_null_fields_use_getter_defaults() {
    let mut b = Builder::new();
    let fields = [
        "checkRepositoriesFrequency",
        "iconResizeSize",
        "launchPanelGameLaunch",
        "launchPanelMinimized",
        "lookAndFeel",
        "preferGitHubUpdates",
        "startWithOS",
    ]
    .map(|name| obj(name, "Ljava/lang/Object;", Value::Null));
    let root = object(
        &mut b,
        PREFERENCES,
        7_034_433_806_261_372_240,
        fields.into(),
        &[],
    );
    let (decoded, warnings) = decode::<Preferences>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        decoded,
        Preferences {
            launch_panel_game_launch: MinimizationType::TaskBar,
            launch_panel_minimized: MinimizationType::TaskBar,
            look_and_feel: LookAndFeel::LafDefault,
            icon_resize_size: IconResize::Auto,
            start_with_os: StartWithOs::Disabled,
            check_repositories_frequency: CheckRepositoriesFrequency::Freq3,
            prefer_github_updates: true,
        }
    );
}

#[test]
fn configuration_with_aia_options() {
    let mut b = Builder::new();
    let aia = vec![
        s(&mut b, "allinArmaPath", Some("/aia")),
        s(&mut b, "arma2OAPath", None),
        s(&mut b, "arma2Path", Some("/a2")),
        s(&mut b, "armaPath", None),
        s(&mut b, "tohPath", None),
    ];
    let aia = object(
        &mut b,
        "fr.soe.a3s.domain.configration.AiAOptions",
        -2_248_371_007_390_169_026,
        aia,
        &[],
    );
    let extra = vec![obj(
        "aiaOptions",
        "Lfr/soe/a3s/domain/configration/AiAOptions;",
        aia,
    )];
    let expected = legacy_configuration();
    let root = enc_configuration(&mut b, &expected, extra, &[]);
    let (decoded, warnings) = decode::<LegacyConfiguration>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded, expected);
}

#[test]
fn old_favorite_server_and_launcher_options() {
    let mut b = Builder::new();
    // FavoriteServer before `modsetName` and `repositoryName`.
    let old_server = vec![
        int("port", 2302),
        z("selected", true),
        s(&mut b, "ipAddress", Some("1.2.3.4")),
        s(&mut b, "name", Some("Old")),
        s(&mut b, "password", None),
    ];
    let old_server = object(
        &mut b,
        wire::FAVORITE_SERVER,
        4_613_236_292_596_702_132,
        old_server,
        &[],
    );
    let servers = b.array_list(vec![old_server]);
    let launcher =
        enc_launcher_options(&mut b, &launcher_options(), &["launchArma3AsAdministrator"]);
    let fields = vec![
        obj("favoriteServers", LIST, servers),
        obj("launcherOptions", LAUNCHER_OPTIONS_SIG, launcher),
    ];
    let root = object(
        &mut b,
        CONFIGURATION,
        -8_310_476_744_472_497_506,
        fields,
        &[],
    );
    let (decoded, warnings) = decode::<LegacyConfiguration>(&finish(b, root));
    assert_eq!(
        decoded.configuration.favorite_servers,
        [FavoriteServer {
            name: Some("Old".to_owned()),
            ip_address: Some("1.2.3.4".to_owned()),
            port: 2302,
            password: None,
            selected: true,
            modset_name: None,
            repository_name: None,
        }]
    );
    let launcher = decoded.launcher_options.unwrap();
    assert!(!launcher.launch_arma3_as_administrator);
    assert_eq!(
        launcher,
        LauncherOptions {
            launch_arma3_as_administrator: false,
            ..launcher_options()
        }
    );
    // Missing fields of the root take Java zero values; null names fall back with warnings.
    let c = decoded.configuration;
    assert_eq!(c.profile_name, "Default");
    assert_eq!(c.game_version, "Arma 3");
    assert!(!c.view_mode_tree);
    assert_eq!(c.proxy, Proxy::default());
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("profileName"), "{warnings:?}");
}

#[test]
fn old_protocol_and_tree_leaf() {
    let mut b = Builder::new();
    // AbstractProtocole before validateSSLCertificate, timeouts and encryptionMode.
    let base = b.class_desc(
        wire::ABSTRACT_PROTOCOLE,
        -886_357_032_287_815_203,
        S,
        &[
            FieldSpec::object("login", STR),
            FieldSpec::object("password", STR),
            FieldSpec::object("port", STR),
            FieldSpec::object("protocolType", "Lfr/soe/a3s/constant/ProtocolType;"),
            FieldSpec::object("url", STR),
        ],
        None,
    );
    let ftp = b.class_desc(wire::FTP, -1_917_448_655_850_030_044, S, &[], Some(base));
    let id = b.reserve_object(ftp);
    let values = vec![
        b.string("anonymous"),
        Value::Null,
        b.string("21"),
        b.enum_value(wire::PROTOCOL_TYPE, "FTP"),
        b.string("ftp.example/repo"),
    ];
    b.set_object_data(
        id,
        vec![
            ClassData {
                values,
                annotations: Vec::new(),
            },
            ClassData::default(),
        ],
    );
    let fields = vec![
        s(&mut b, "name", Some("Old")),
        obj("protocole", AP, Value::Ref(id)),
    ];
    let root = object(&mut b, REPOSITORY, -8_142_021_113_361_619_970, fields, &[]);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        decoded.repository.protocol,
        Protocol {
            protocol_type: ProtocolType::Ftp,
            url: "ftp.example/repo".to_owned(),
            port: "21".to_owned(),
            login: Some("anonymous".to_owned()),
            password: None,
            connection_timeout: None,
            read_timeout: None,
            validate_ssl_certificate: false,
            encryption_mode: None,
        }
    );
    assert_eq!(decoded.repository.protocol.connection_timeout_ms(), 300_000);
    assert_eq!(decoded.repository.number_of_connections, 0);

    // TreeLeaf before workshopId and dlcAppId.
    let mut expected = profile();
    let mut b = Builder::new();
    let root = enc_profile(&mut b, &expected, &["workshopId", "dlcAppId"]);
    let (decoded, warnings) = decode::<Profile>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    clear_leaf_ids(&mut expected.tree);
    assert_eq!(decoded, expected);
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

/// A protocol of `class` (current descriptors) with a null `protocolType`.
fn untyped_protocol(b: &mut Builder, class: &str) -> Value {
    let desc = wire::protocol_class_desc(b, class);
    let id = b.reserve_object(desc);
    let values = vec![
        Value::Boolean(false),
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Null,
        b.string("1080"),
        Value::Null,
        Value::Null,
        b.string("proxy.example"),
    ];
    b.set_object_data(
        id,
        vec![
            ClassData {
                values,
                annotations: Vec::new(),
            },
            ClassData::default(),
        ],
    );
    Value::Ref(id)
}

#[test]
fn null_protocol_type_falls_back_to_class() {
    for (class, expected) in [
        (wire::SOCKS, ProtocolType::Socks4),
        (wire::FTP, ProtocolType::Ftp),
        (wire::HTTP, ProtocolType::Http),
        (wire::SFTP, ProtocolType::Sftp),
        (wire::WEBDAV, ProtocolType::HttpWebdav),
    ] {
        let mut b = Builder::new();
        let protocol = untyped_protocol(&mut b, class);
        let proxy = vec![z("enableProxy", true), obj("proxyProtocol", AP, protocol)];
        let proxy = object(
            &mut b,
            "fr.soe.a3s.domain.configration.Proxy",
            -6_891_013_124_542_313_138,
            proxy,
            &[],
        );
        let fields = vec![obj(
            "proxy",
            "Lfr/soe/a3s/domain/configration/Proxy;",
            proxy,
        )];
        let root = object(
            &mut b,
            CONFIGURATION,
            -8_310_476_744_472_497_506,
            fields,
            &[],
        );
        let (decoded, _) = decode::<LegacyConfiguration>(&finish(b, root));
        let proxy = decoded.configuration.proxy;
        assert!(proxy.enable_proxy);
        let protocol = proxy.proxy_protocol.unwrap();
        assert_eq!(protocol.protocol_type, expected, "{class}");
        assert_eq!(protocol.url, "proxy.example");
    }
}

#[test]
fn null_protocol_type_strict_decoder_matches_java() {
    // The strict `.a3s/autoconfig` decoder also maps a `Socks` with a
    // null type to SOCKS4 (domain/Socks.java:20-22).
    let mut b = Builder::new();
    let root = untyped_protocol(&mut b, wire::SOCKS);
    let bytes = finish(b, root);
    let protocol: Protocol = a3s_types::decode_stream(&bytes).unwrap();
    assert_eq!(protocol.protocol_type, ProtocolType::Socks4);
}

#[test]
fn legacy_domain_proxy() {
    let mut b = Builder::new();
    let protocol = Protocol::new(ProtocolType::Http, "proxy", "3128", "", "", false).encode(&mut b);
    let proxy = vec![z("proxyEnabled", true), obj("proxyProtocol", AP, protocol)];
    let proxy = object(&mut b, "fr.soe.a3s.domain.Proxy", -50, proxy, &[]);
    let fields = vec![obj("proxy", "Lfr/soe/a3s/domain/Proxy;", proxy)];
    let root = object(
        &mut b,
        CONFIGURATION,
        -8_310_476_744_472_497_506,
        fields,
        &[],
    );
    let (decoded, _) = decode::<LegacyConfiguration>(&finish(b, root));
    let proxy = decoded.configuration.proxy;
    assert!(proxy.enable_proxy);
    assert_eq!(proxy.proxy_protocol.unwrap().port, "3128");
}

// ---------------------------------------------------------------------------------------------
// Leniency and hard errors

#[test]
fn unknown_enum_constant_falls_back_with_warning() {
    let mut b = Builder::new();
    let fields = vec![en(
        &mut b,
        "lookAndFeel",
        "fr.soe.a3s.constant.LookAndFeel",
        "LAF_FOO",
    )];
    let prefs = object(&mut b, PREFERENCES, 7_034_433_806_261_372_240, fields, &[]);
    let (decoded, warnings) = decode::<Preferences>(&finish(b, prefs));
    assert_eq!(decoded.look_and_feel, LookAndFeel::LafDefault);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("lookAndFeel"), "{warnings:?}");
    assert!(warnings[0].contains("LAF_FOO"), "{warnings:?}");
}

#[test]
fn wrong_primitive_type_is_treated_as_missing() {
    let mut b = Builder::new();
    let fields = vec![
        long("height", 5),
        int("width", 7),
        s(&mut b, "gameVersion", Some("Arma 3")),
        s(&mut b, "profileName", Some("P")),
    ];
    let root = object(
        &mut b,
        CONFIGURATION,
        -8_310_476_744_472_497_506,
        fields,
        &[],
    );
    let (decoded, warnings) = decode::<LegacyConfiguration>(&finish(b, root));
    assert_eq!(decoded.configuration.height, 0);
    assert_eq!(decoded.configuration.width, 7);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("height"), "{warnings:?}");
}

#[test]
fn wrong_kinds_and_null_elements() {
    let mut b = Builder::new();
    let a = b.string("@a");
    let c = b.string("@c");
    let priorities = b.array_list(vec![a, Value::Null, c]);
    // A Boolean where a String is expected in a list, and a String where a list is expected.
    let flag = b.boolean(true);
    let d = b.string("/d");
    let directories = b.array_list(vec![flag, d]);
    let launcher = b.string("not launcher options");
    let tree = enc_tree_dir(&mut b, &TreeDirectory::new_root(), None, &[]);
    let fields = vec![
        obj("addonNamesByPriority", LIST, priorities),
        obj("addonSearchDirectories", LIST, directories),
        obj("launcherOptions", LAUNCHER_OPTIONS_SIG, launcher),
        s(&mut b, "name", Some("P")),
        obj("tree", TREE_DIRECTORY_SIG, tree),
    ];
    let root = object(&mut b, PROFILE, -4_395_099_790_449_721_021, fields, &[]);
    let (decoded, warnings) = decode::<Profile>(&finish(b, root));
    assert_eq!(decoded.addon_names_by_priority, ["@a", "@c"]);
    assert_eq!(decoded.addon_search_directories, ["/d"]);
    assert_eq!(decoded.launcher_options, LauncherOptions::default());
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}

#[test]
fn wrong_root_class() {
    let mut b = Builder::new();
    let root = enc_preferences(&mut b, &preferences(), &[]);
    let bytes = finish(b, root);
    let err = decode_legacy_stream::<Profile>(&bytes).unwrap_err();
    assert!(
        matches!(&err, Error::UnexpectedClass { expected, found }
            if *expected == PROFILE && found == PREFERENCES),
        "{err:?}"
    );
    assert!(decode_legacy_stream::<LegacyRepository>(b"not a stream").is_err());
}

#[test]
fn repository_hard_errors() {
    // Null name.
    let mut b = Builder::new();
    let protocol = Protocol::new(ProtocolType::Http, "h", "80", "", "", false).encode(&mut b);
    let fields = vec![
        obj("name", STR, Value::Null),
        obj("protocole", AP, protocol),
    ];
    let root = object(&mut b, REPOSITORY, -8_142_021_113_361_619_970, fields, &[]);
    let err = decode_legacy_stream::<LegacyRepository>(&finish(b, root)).unwrap_err();
    assert!(matches!(err, Error::NullValue("name")), "{err:?}");

    // Null protocol.
    let mut b = Builder::new();
    let fields = vec![
        s(&mut b, "name", Some("R")),
        obj("protocole", AP, Value::Null),
    ];
    let root = object(&mut b, REPOSITORY, -8_142_021_113_361_619_970, fields, &[]);
    let err = decode_legacy_stream::<LegacyRepository>(&finish(b, root)).unwrap_err();
    assert!(matches!(err, Error::NullValue("protocole")), "{err:?}");

    // Not an AbstractProtocole.
    let mut b = Builder::new();
    let not_protocol = b.boolean(true);
    let fields = vec![
        s(&mut b, "name", Some("R")),
        obj("protocole", AP, not_protocol),
    ];
    let root = object(&mut b, REPOSITORY, -8_142_021_113_361_619_970, fields, &[]);
    let err = decode_legacy_stream::<LegacyRepository>(&finish(b, root)).unwrap_err();
    assert!(matches!(err, Error::UnexpectedClass { .. }), "{err:?}");

    // Null profile name.
    let mut b = Builder::new();
    let fields = vec![obj("name", STR, Value::Null)];
    let root = object(&mut b, PROFILE, -4_395_099_790_449_721_021, fields, &[]);
    let err = decode_legacy_stream::<Profile>(&finish(b, root)).unwrap_err();
    assert!(matches!(err, Error::NullValue("name")), "{err:?}");
}

#[test]
fn file_cache_drops_null_entries() {
    let mut b = Builder::new();
    let good = vec![long("lastModified", 5), s(&mut b, "sha1", Some("abc"))];
    let good = object(
        &mut b,
        FILE_ATTRIBUTES,
        -2_500_913_557_576_506_383,
        good,
        &[],
    );
    let no_sha1 = vec![long("lastModified", 6), s(&mut b, "sha1", None)];
    let no_sha1 = object(
        &mut b,
        FILE_ATTRIBUTES,
        -2_500_913_557_576_506_383,
        no_sha1,
        &[],
    );
    let cache = b.hash_map_str(&[
        ("/good".to_owned(), good),
        ("/no_sha1".to_owned(), no_sha1),
        ("/null".to_owned(), Value::Null),
    ]);
    let protocol = Protocol::new(ProtocolType::Http, "h", "80", "", "", false).encode(&mut b);
    let fields = vec![
        obj("mapFilesForSync", MAP, cache),
        s(&mut b, "name", Some("R")),
        obj("protocole", AP, protocol),
    ];
    let root = object(&mut b, REPOSITORY, -8_142_021_113_361_619_970, fields, &[]);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        decoded.files_for_sync,
        BTreeMap::from([(
            "/good".to_owned(),
            FileAttributes {
                sha1: "abc".to_owned(),
                last_modified: 5
            }
        )])
    );
    assert!(decoded.files_for_build.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Deprecated configuration fields for migration

#[test]
fn deprecated_configuration_fields() {
    // Present.
    let mut b = Builder::new();
    let root = enc_configuration(&mut b, &legacy_configuration(), Vec::new(), &[]);
    let (decoded, _) = decode::<LegacyConfiguration>(&finish(b, root));
    assert_eq!(
        decoded.addon_search_directory_paths.as_deref(),
        Some(&["/addons/a".to_owned(), "/addons/b".to_owned()][..])
    );
    assert_eq!(decoded.launcher_options, Some(launcher_options()));

    // Null (Java's `reset*()` after migration).
    let mut nulls = legacy_configuration();
    nulls.addon_search_directory_paths = None;
    nulls.launcher_options = None;
    let mut b = Builder::new();
    let root = enc_configuration(&mut b, &nulls, Vec::new(), &[]);
    let (decoded, warnings) = decode::<LegacyConfiguration>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded, nulls);

    // Absent from the class descriptor.
    let mut b = Builder::new();
    let root = enc_configuration(
        &mut b,
        &legacy_configuration(),
        Vec::new(),
        &["addonSearchDirectoryPaths", "launcherOptions"],
    );
    let (decoded, _) = decode::<LegacyConfiguration>(&finish(b, root));
    assert_eq!(decoded.addon_search_directory_paths, None);
    assert_eq!(decoded.launcher_options, None);
}

// ---------------------------------------------------------------------------------------------
// Shared and cyclic nodes

/// A profile whose root group's `list` is replaced by `make_list(root, first_child)`.
fn profile_with_list(make_list: fn(&mut Builder, Value, Value) -> Value) -> Vec<u8> {
    let mut b = Builder::new();
    let mut root = TreeDirectory::new_root();
    root.children.push(TreeNode::Leaf(TreeLeaf::new("@a")));
    let tree = enc_tree_dir(&mut b, &root, None, &[]);
    let root_id = tree.as_ref().unwrap();
    let list = b.graph().field(root_id, "list").unwrap();
    let first = a3s_jserial::jdk::list(b.graph(), list).unwrap().unwrap()[0];
    let replaced = make_list(&mut b, tree, first);
    let fields = vec![
        s(&mut b, "name", Some("P")),
        obj("tree", TREE_DIRECTORY_SIG, tree),
    ];
    let profile = object(&mut b, PROFILE, -4_395_099_790_449_721_021, fields, &[]);
    let mut graph = b.finish(profile);
    let a3s_jserial::Node::Object(dir) = &mut graph.nodes[root_id.index()] else {
        panic!("expected the tree object");
    };
    // TreeDirectory wire order: marked, optional, selected, list, ...
    dir.class_data[0].values[3] = replaced;
    a3s_jserial::write(&graph).unwrap()
}

#[test]
fn shared_or_cyclic_tree_nodes_are_rejected() {
    let shared_leaf = profile_with_list(|b, _root, leaf| b.array_list(vec![leaf, leaf]));
    let cycle = profile_with_list(|b, root, _leaf| b.array_list(vec![root]));
    for bytes in [shared_leaf, cycle] {
        let err = decode_legacy_stream::<Profile>(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Serialization(a3s_jserial::Error::LimitExceeded(_))
            ),
            "{err:?}"
        );
    }
    // Unchanged list: decodes.
    let fine = profile_with_list(|b, _root, leaf| b.array_list(vec![leaf]));
    assert!(decode_legacy_stream::<Profile>(&fine).is_ok());
}
