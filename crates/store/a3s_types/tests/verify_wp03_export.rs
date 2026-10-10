//! WP03 verifier: `export.a3s.autoconfig` encoder edge cases and descriptor stability (B45-B47,
//! wire table, A14/A15), plus TOML and protocol-helper edge cases (B32-B33, B48-B49).

use std::collections::BTreeMap;

use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Limits, Node, NodeId, Value};
use a3s_types::constants::ModsetType;
use a3s_types::local::protocol::{check_protocol, host_url};
use a3s_types::local::{
    AutoConfigExport, LauncherOptions, Profile, ProtocolCheckError, Repository, TreeDirectory,
    TreeLeaf, TreeNode, decode_legacy_stream, wire as local_wire,
};
use a3s_types::{EncryptionMode, FavoriteServer, Protocol, ProtocolType, encode_stream, wire};

fn root_id(graph: &Graph) -> NodeId {
    graph.root_value().unwrap().as_ref().unwrap()
}

fn list_ids(graph: &Graph, id: NodeId, field: &str) -> Vec<NodeId> {
    a3s_jserial::jdk::list(graph, graph.field(id, field).unwrap())
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|v| v.as_ref().unwrap())
        .collect()
}

fn deep_profile() -> Profile {
    let mut profile = Profile::new("\u{1F680} Pr\u{f6}fil");
    let mut current = TreeDirectory::new("racine");
    current.name = "level 9".to_owned();
    current.modset_type = Some(ModsetType::Event);
    current.modset_repository_name = Some("Repo \u{e9}".to_owned());
    current.children.push(TreeNode::Leaf(TreeLeaf {
        name: "@deep".to_owned(),
        selected: true,
        optional: true,
        workshop_id: Some("123".to_owned()),
        dlc_app_id: Some("1042220".to_owned()),
        missing: false,
    }));
    for level in (0..9).rev() {
        let mut dir = TreeDirectory::new(format!("level {level}"));
        dir.marked = level % 2 == 0;
        dir.children.push(TreeNode::Directory(current));
        dir.children
            .push(TreeNode::Leaf(TreeLeaf::new(format!("@leaf{level}"))));
        current = dir;
    }
    // A nested directory literally named "racine" must not be confused with the root.
    current
        .children
        .push(TreeNode::Directory(TreeDirectory::new("racine")));
    profile.tree.children.push(TreeNode::Directory(current));
    profile.addon_names_by_priority = vec!["@b".to_owned(), "@a".to_owned()];
    profile.addon_search_directories = vec!["C:\\Arma 3\\!Workshop".to_owned()];
    profile.additional_parameters = Some("-world=empty".to_owned());
    profile.launcher_options.window_mode = true;
    profile
}

/// Asserts every directory's children point back to it, recursively; returns the depth.
fn check_parents(graph: &Graph, dir: NodeId) -> usize {
    let mut depth = 0;
    for child in list_ids(graph, dir, "list") {
        assert_eq!(graph.field(child, "parent").unwrap(), Value::Ref(dir));
        if graph.class_name_of(child).unwrap() == local_wire::TREE_DIRECTORY {
            depth = depth.max(1 + check_parents(graph, child));
        }
    }
    depth
}

#[test]
fn verify_export_deep_unicode_profile_round_trips() {
    let export = AutoConfigExport {
        profiles: vec![deep_profile()],
        ..AutoConfigExport::default()
    };
    let bytes = encode_stream(&export).unwrap();
    let graph = a3s_jserial::read_with_limits(&bytes, Limits::JAVA_A3S).unwrap();
    let profile = list_ids(&graph, root_id(&graph), "profiles")[0];
    let tree = graph.field(profile, "tree").unwrap().as_ref().unwrap();
    assert_eq!(graph.field(tree, "parent").unwrap(), Value::Null);
    assert_eq!(check_parents(&graph, tree), 10);

    let legacy = decode_legacy_stream::<AutoConfigExport>(&bytes).unwrap();
    assert!(legacy.warnings.is_empty(), "{:?}", legacy.warnings);
    let mut expected = deep_profile();
    expected.launcher_options = LauncherOptions::default();
    assert_eq!(legacy.value.profiles, [expected]);
}

#[test]
fn verify_export_empty_and_deterministic() {
    let empty = AutoConfigExport::default();
    let bytes = encode_stream(&empty).unwrap();
    let graph = a3s_jserial::read_with_limits(&bytes, Limits::JAVA_A3S).unwrap();
    let root = root_id(&graph);
    for field in ["favoriteServers", "profiles", "repositories"] {
        let value = graph.field(root, field).unwrap();
        let id = value
            .as_ref()
            .expect("lists are never null (Java initializers)");
        assert_eq!(graph.class_name_of(id).unwrap(), "java.util.ArrayList");
        assert!(list_ids(&graph, root, field).is_empty());
    }
    let legacy = decode_legacy_stream::<AutoConfigExport>(&bytes).unwrap();
    assert_eq!(legacy.value, empty);
    assert!(legacy.warnings.is_empty());

    let export = AutoConfigExport {
        profiles: vec![deep_profile()],
        favorite_servers: vec![FavoriteServer {
            name: None,
            ip_address: Some("1.2.3.4".to_owned()),
            port: 2302,
            password: None,
            selected: false,
            modset_name: None,
            repository_name: None,
        }],
        repositories: vec![Repository::new(
            "R",
            Protocol::new(ProtocolType::Https, "h/x", "443", "", "", true),
        )],
    };
    assert_eq!(
        encode_stream(&export).unwrap(),
        encode_stream(&export).unwrap()
    );
}

#[test]
fn verify_export_keeps_favorite_server_and_profile_order() {
    // Export lists are written in the caller's order; decode must not re-sort them (only
    // `Configuration` sorts favorite servers).
    let server = |name: &str| FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: None,
        port: 1,
        password: None,
        selected: true,
        modset_name: Some("m".to_owned()),
        repository_name: Some("r".to_owned()),
    };
    let export = AutoConfigExport {
        profiles: vec![Profile::new("zeta"), Profile::new("Alpha")],
        favorite_servers: vec![server("Zulu"), server("alpha"), server("Mike")],
        repositories: vec![
            Repository::new(
                "z",
                Protocol::new(ProtocolType::Ftp, "a", "21", "", "", false),
            ),
            Repository::new(
                "A",
                Protocol::new(ProtocolType::Sftp, "b", "22", "", "", false),
            ),
        ],
    };
    let bytes = encode_stream(&export).unwrap();
    let decoded = decode_legacy_stream::<AutoConfigExport>(&bytes)
        .unwrap()
        .value;
    assert_eq!(decoded.favorite_servers, export.favorite_servers);
    let names: Vec<&str> = decoded.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["zeta", "Alpha"]);
    let names: Vec<&str> = decoded
        .repositories
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["z", "A"]);
}

#[test]
fn verify_export_repository_protocol_is_kept_for_every_type() {
    // Java exports `new Repository(name, protocol)` with the live protocol object, including
    // credentials, timeouts and encryption mode.
    for t in [
        ProtocolType::Ftp,
        ProtocolType::Ftps,
        ProtocolType::Sftp,
        ProtocolType::Http,
        ProtocolType::Https,
        ProtocolType::HttpWebdav,
        ProtocolType::HttpsWebdav,
    ] {
        let mut protocol = Protocol::new(t, "host.example/\u{e9}/repo", "1234", "me", "pw", true);
        protocol.connection_timeout = Some("1".to_owned());
        protocol.read_timeout = None;
        protocol.encryption_mode = Some(EncryptionMode::ExplicitSsl);
        let mut repository = Repository::new("R", protocol.clone());
        repository.upload_protocol = Some(protocol.clone());
        repository.bit_torrent_protocol = Some(protocol.clone());
        repository.revision = 99;
        let export = AutoConfigExport {
            repositories: vec![repository],
            ..AutoConfigExport::default()
        };
        let bytes = encode_stream(&export).unwrap();
        a3s_jserial::read_with_limits(&bytes, Limits::JAVA_A3S).unwrap();
        let decoded = decode_legacy_stream::<AutoConfigExport>(&bytes).unwrap();
        assert!(decoded.warnings.is_empty(), "{t:?}: {:?}", decoded.warnings);
        assert_eq!(
            decoded.value.repositories,
            [Repository::new("R", protocol)],
            "{t:?}"
        );
    }
}

#[test]
fn verify_export_collections_are_separate_instances() {
    let export = AutoConfigExport {
        profiles: vec![Profile::new("a"), Profile::new("b")],
        repositories: vec![
            Repository::new(
                "r1",
                Protocol::new(ProtocolType::Http, "h", "80", "", "", false),
            ),
            Repository::new(
                "r2",
                Protocol::new(ProtocolType::Http, "h", "80", "", "", false),
            ),
        ],
        ..AutoConfigExport::default()
    };
    let bytes = encode_stream(&export).unwrap();
    let graph = a3s_jserial::read(&bytes).unwrap();
    let root = root_id(&graph);
    let mut ids = Vec::new();
    for profile in list_ids(&graph, root, "profiles") {
        for field in ["addonNamesByPriority", "addonSearchDirectories"] {
            ids.push(graph.field(profile, field).unwrap().as_ref().unwrap());
        }
        let tree = graph.field(profile, "tree").unwrap().as_ref().unwrap();
        ids.push(graph.field(tree, "list").unwrap().as_ref().unwrap());
    }
    for repository in list_ids(&graph, root, "repositories") {
        for field in [
            "excludedFilesFromBuild",
            "excludedFoldersFromSync",
            "favoriteServersSetToAutoconfig",
            "hidddenFolderPaths",
            "mapEventsDownloadLocation",
            "mapFilesForBuild",
            "mapFilesForSync",
        ] {
            ids.push(graph.field(repository, field).unwrap().as_ref().unwrap());
        }
        // Protocol objects are not shared between repositories either.
        ids.push(
            graph
                .field(repository, "protocole")
                .unwrap()
                .as_ref()
                .unwrap(),
        );
    }
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count, "collections must be separate instances");
}

// ---------------------------------------------------------------------------------------------
// Descriptor stability: every fr.soe.a3s descriptor in a real export equals the locked one

fn reference_descriptor(name: &str) -> Option<(Builder, NodeId)> {
    let mut b = Builder::new();
    let id = match name {
        local_wire::AUTO_CONFIG_EXPORT => local_wire::auto_config_export_desc(&mut b),
        local_wire::PROFILE => local_wire::profile_desc(&mut b),
        local_wire::TREE_DIRECTORY => local_wire::tree_directory_desc(&mut b),
        local_wire::TREE_LEAF => local_wire::tree_leaf_desc(&mut b),
        local_wire::MODSET_TYPE => local_wire::modset_type_desc(&mut b),
        local_wire::REPOSITORY => local_wire::repository_desc(&mut b),
        wire::FAVORITE_SERVER => wire::favorite_server_desc(&mut b),
        wire::ABSTRACT_PROTOCOLE => wire::abstract_protocole_desc(&mut b),
        wire::HTTP | wire::FTP | wire::WEBDAV | wire::SOCKS | wire::SFTP => {
            wire::protocol_class_desc(&mut b, name)
        }
        wire::PROTOCOL_TYPE | wire::ENCRYPTION_MODE => {
            let constant = b.enum_value(name, "X").as_ref().unwrap();
            let Node::Enum(e) = b.graph().node(constant).unwrap() else {
                unreachable!()
            };
            e.class
        }
        _ => return None,
    };
    Some((b, id))
}

type Shape = (
    String,
    i64,
    u8,
    Vec<(u8, String, Option<String>)>,
    Option<String>,
);

fn shape(graph: &Graph, id: NodeId) -> Shape {
    let desc = graph.class_desc(id).unwrap();
    let fields = desc
        .fields
        .iter()
        .map(|f| {
            let sig = f.class_name.map(|s| graph.string(s).unwrap().to_owned());
            (f.field_type.code(), f.name.clone(), sig)
        })
        .collect();
    let super_name = desc
        .super_class
        .map(|s| graph.class_desc(s).unwrap().name.clone());
    (
        desc.name.clone(),
        desc.serial_version_uid,
        desc.flags,
        fields,
        super_name,
    )
}

#[test]
fn verify_export_stream_descriptors_match_locked_descriptors() {
    let mut profile = deep_profile();
    profile.tree.modset_type = Some(ModsetType::Repository);
    let export = AutoConfigExport {
        profiles: vec![profile],
        favorite_servers: vec![FavoriteServer {
            name: Some("s".to_owned()),
            ip_address: None,
            port: 0,
            password: None,
            selected: false,
            modset_name: None,
            repository_name: None,
        }],
        repositories: [
            ProtocolType::Ftp,
            ProtocolType::Sftp,
            ProtocolType::Http,
            ProtocolType::HttpWebdav,
        ]
        .into_iter()
        .map(|t| Repository::new(format!("{t:?}"), Protocol::new(t, "h", "1", "", "", false)))
        .collect(),
    };
    let bytes = encode_stream(&export).unwrap();
    let graph = a3s_jserial::read(&bytes).unwrap();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let Node::ClassDesc(desc) = node else {
            continue;
        };
        *seen.entry(desc.name.clone()).or_default() += 1;
        if !desc.name.starts_with("fr.soe.a3s.") {
            continue;
        }
        let (b, reference) = reference_descriptor(&desc.name)
            .unwrap_or_else(|| panic!("unexpected class {} in export", desc.name));
        let id = NodeId(u32::try_from(index).unwrap());
        assert_eq!(
            shape(&graph, id),
            shape(b.graph(), reference),
            "{}",
            desc.name
        );
    }
    for (name, count) in &seen {
        assert_eq!(*count, 1, "class {name} described {count} times");
    }
    for name in [
        local_wire::AUTO_CONFIG_EXPORT,
        local_wire::PROFILE,
        local_wire::TREE_DIRECTORY,
        local_wire::TREE_LEAF,
        local_wire::MODSET_TYPE,
        local_wire::REPOSITORY,
        wire::FAVORITE_SERVER,
        wire::ABSTRACT_PROTOCOLE,
    ] {
        assert!(seen.contains_key(name), "{name} missing from export stream");
    }
}

// ---------------------------------------------------------------------------------------------
// TOML (B48, B49)

#[test]
fn verify_toml_runtime_flags_are_not_persisted() {
    let mut profile = Profile::new("P");
    let mut dir = TreeDirectory::new("g");
    dir.updated = true;
    let mut leaf = TreeLeaf::new("@a");
    leaf.missing = true;
    dir.children.push(TreeNode::Leaf(leaf));
    profile.tree.children.push(TreeNode::Directory(dir));
    let text = toml::to_string(&profile).unwrap();
    assert!(!text.contains("updated"), "{text}");
    assert!(!text.contains("missing"), "{text}");
    let back: Profile = toml::from_str(&text).unwrap();
    let TreeNode::Directory(dir) = &back.tree.children[0] else {
        panic!("expected a directory");
    };
    assert!(!dir.updated);
    let TreeNode::Leaf(leaf) = &dir.children[0] else {
        panic!("expected a leaf");
    };
    assert!(!leaf.missing);
}

#[test]
fn verify_toml_edge_values_round_trip() {
    let mut repository = Repository::new(
        "R \"quoted\" \u{1F600}",
        Protocol::new(ProtocolType::Ftp, "h", "21", "u", "p", true),
    );
    repository.revision = i32::MIN;
    repository.number_of_connections = 0;
    repository.parallel_upload_connections = -3;
    repository.maximum_client_download_speed = 1e300;
    repository
        .events_download_location
        .insert("C:\\Events\\a.b".to_owned(), "D:\\\u{e9}".to_owned());
    repository.hidden_folder_paths.insert("\t/tab".to_owned());
    let text = toml::to_string(&repository).unwrap();
    let back: Repository = toml::from_str(&text).unwrap();
    assert_eq!(back, repository);
    assert_eq!(back.parallel_upload_connections(), 4);
}

#[test]
fn verify_toml_repository_requires_name_and_protocol() {
    let protocol = "[protocol]\nprotocol_type = \"Ftp\"\nurl = \"h\"\nport = \"21\"\n";
    assert!(
        toml::from_str::<Repository>(protocol).is_err(),
        "name is required"
    );
    assert!(
        toml::from_str::<Repository>("name = \"R\"\n").is_err(),
        "protocol is required"
    );
}

// ---------------------------------------------------------------------------------------------
// Protocol helpers (B32, B33)

fn valid() -> Protocol {
    Protocol::new(ProtocolType::Ftp, "host/path", "21", "user", "", false)
}

#[test]
fn verify_check_protocol_java_int_edges() {
    let check = |port: &str| {
        let mut p = valid();
        p.port = port.to_owned();
        check_protocol(&p)
    };
    assert_eq!(check("+65535"), Ok(()));
    assert_eq!(check("-2147483648"), Ok(()), "Integer.MIN_VALUE parses");
    assert_eq!(check("0"), Ok(()));
    assert_eq!(check("0065535"), Ok(()), "leading zeros parse");
    assert_eq!(check("-2147483649"), Err(ProtocolCheckError::PortInvalid));
    assert_eq!(check("+"), Err(ProtocolCheckError::PortInvalid));
    assert_eq!(check("-"), Err(ProtocolCheckError::PortInvalid));
    assert_eq!(check("+-1"), Err(ProtocolCheckError::PortInvalid));
    assert_eq!(
        check(" "),
        Err(ProtocolCheckError::PortInvalid),
        "blank is not empty"
    );
    assert_eq!(check("21 "), Err(ProtocolCheckError::PortInvalid));
    assert_eq!(check("0x50"), Err(ProtocolCheckError::PortInvalid));
    assert_eq!(check("2147483647"), Err(ProtocolCheckError::PortOutOfRange));

    // Port error wins over timeout errors; blank login passes.
    let mut p = valid();
    p.port = "65536".to_owned();
    p.connection_timeout = Some(String::new());
    assert_eq!(check_protocol(&p), Err(ProtocolCheckError::PortOutOfRange));
    let mut p = valid();
    p.login = Some(" ".to_owned());
    p.connection_timeout = Some("-1".to_owned());
    p.read_timeout = Some("+0".to_owned());
    assert_eq!(check_protocol(&p), Ok(()));
    let mut p = valid();
    p.login = None;
    p.url = String::new();
    assert_eq!(check_protocol(&p), Err(ProtocolCheckError::UrlEmpty));
    let mut p = valid();
    p.read_timeout = Some("3e5".to_owned());
    assert_eq!(
        check_protocol(&p),
        Err(ProtocolCheckError::ReadTimeoutInvalid)
    );
}

#[test]
fn verify_host_url_prompts() {
    let url = |t: ProtocolType, url: &str, port: &str| {
        host_url(&Protocol::new(t, url, port, "", "", false))
    };
    assert_eq!(url(ProtocolType::Ftp, "host", "21"), "ftp://host:21");
    assert_eq!(url(ProtocolType::Ftps, "h/a/b", "990"), "ftps://h:990/a/b");
    assert_eq!(url(ProtocolType::Sftp, "h/", "22"), "sftp://h:22/");
    assert_eq!(url(ProtocolType::HttpWebdav, "h/d", "80"), "http://h:80/d");
    assert_eq!(url(ProtocolType::HttpsWebdav, "h", "443"), "https://h:443");
    assert_eq!(url(ProtocolType::Socks4, "p", "1080"), "socks4://p:1080");
    assert_eq!(url(ProtocolType::Socks5, "p", "1080"), "socks5://p:1080");
    assert_eq!(url(ProtocolType::A3s, "x/y", ""), "a3s://x:/y");
    assert_eq!(url(ProtocolType::Http, "", "80"), "http://:80");
}
