//! `export.a3s.autoconfig` encoding: what Java's `CommonService.exportAutoConfig` writes.

use a3s_jserial::{Graph, Limits, Node, NodeId, Value};
use a3s_types::constants::ModsetType;
use a3s_types::local::{
    AutoConfigExport, LauncherOptions, Profile, Repository, TreeDirectory, TreeLeaf, TreeNode,
    decode_legacy_stream,
};
use a3s_types::{FavoriteServer, Protocol, ProtocolType, decode_stream, encode_stream};

/// JDK classes on Java's reader allow-list (`dao/A3SFilesAccessor.java:116-141`).
const ALLOWED_JDK: &[&str] = &[
    "java.lang.String",
    "java.lang.Enum",
    "java.lang.Boolean",
    "java.lang.Byte",
    "java.lang.Character",
    "java.lang.Double",
    "java.lang.Float",
    "java.lang.Integer",
    "java.lang.Long",
    "java.lang.Short",
    "java.util.ArrayList",
    "java.util.HashMap",
    "java.util.HashSet",
    "java.util.LinkedHashMap",
    "java.util.LinkedHashSet",
    "java.util.Properties",
    "java.util.TreeMap",
    "java.util.TreeSet",
    "java.util.Date",
    "java.util.Map$Entry",
    "javax.crypto.SealedObject",
];

fn sample() -> AutoConfigExport {
    let mut group = TreeDirectory::new("Event");
    group.modset_type = Some(ModsetType::Event);
    group.modset_repository_name = Some("WOG".to_owned());
    let mut nested = TreeDirectory::new("Nested");
    nested.add_child(TreeNode::Leaf(TreeLeaf {
        workshop_id: Some("450814997".to_owned()),
        ..TreeLeaf::new("@cba_a3")
    }));
    group.add_child(TreeNode::Directory(nested));
    group.add_child(TreeNode::Leaf(TreeLeaf {
        dlc_app_id: Some("1042220".to_owned()),
        optional: true,
        ..TreeLeaf::new("GM")
    }));
    let mut main = Profile::new("Main");
    main.tree.add_child(TreeNode::Directory(group));
    main.tree.add_child(TreeNode::Leaf(TreeLeaf::new("@ace")));
    main.launcher_options = LauncherOptions {
        cpu_count_selection: 8,
        arma3_exe_path: Some("C:\\arma3_x64.exe".to_owned()),
        window_mode: true,
        ..Default::default()
    };
    main.addon_names_by_priority = vec!["@cba_a3".to_owned()];
    main.addon_search_directories = vec!["/games/arma3".to_owned()];

    let server = |name: &str, port| FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some("10.0.0.1".to_owned()),
        port,
        password: None,
        selected: false,
        modset_name: Some("Event".to_owned()),
        repository_name: Some("WOG".to_owned()),
    };

    let mut repository = Repository::new(
        "WOG",
        Protocol::new(
            ProtocolType::Ftp,
            "88.99.69.243/Repo/WOG",
            "21",
            "anonymous",
            "",
            false,
        ),
    );
    repository.upload_protocol = Some(Protocol::new(
        ProtocolType::Sftp,
        "upload.example/wog",
        "22",
        "admin",
        "secret",
        true,
    ));
    repository.revision = 99;
    repository.no_auto_discover = false;
    repository.parallel_upload_connections = 9;
    repository.number_of_connections = 5;
    repository.hidden_folder_paths = ["@hidden".to_owned()].into();
    repository.events_download_location = [("Event".to_owned(), "/e".to_owned())].into();
    repository.excluded_files_from_build = ["*.bak".to_owned()].into();
    repository.favorite_servers_set_to_autoconfig = vec![server("x", 1)];
    repository.path = Some("/srv/wog".to_owned());

    AutoConfigExport {
        profiles: vec![main, Profile::new("Default")],
        favorite_servers: vec![server("Alpha", 2302), server("Bravo", 2402)],
        repositories: vec![repository],
    }
}

fn object_list(graph: &Graph, id: NodeId, field: &str) -> Vec<NodeId> {
    a3s_jserial::jdk::list(graph, graph.field(id, field).unwrap())
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|v| v.as_ref().unwrap())
        .collect()
}

/// Checks that every child of the directory `id` points back to it, recursively.
fn check_parents(graph: &Graph, id: NodeId, leaves: &mut usize) {
    for child in object_list(graph, id, "list") {
        assert_eq!(graph.field(child, "parent").unwrap(), Value::Ref(id));
        match graph.class_name_of(child).unwrap() {
            "fr.soe.a3s.domain.TreeDirectory" => check_parents(graph, child, leaves),
            "fr.soe.a3s.domain.TreeLeaf" => {
                assert_eq!(
                    graph.field(child, "duplicate").unwrap(),
                    Value::Boolean(false)
                );
                *leaves += 1;
            }
            other => panic!("unexpected tree class {other}"),
        }
    }
}

#[test]
fn export_stream_is_java_readable() {
    let export = sample();
    let bytes = encode_stream(&export).unwrap();
    let graph = a3s_jserial::read_with_limits(&bytes, Limits::JAVA_A3S).unwrap();

    // Root class with the JVM-computed UID.
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let desc = graph.class_desc(graph.object(root).unwrap().class).unwrap();
    assert_eq!(desc.name, "fr.soe.a3s.domain.AutoConfig");
    assert_eq!(desc.serial_version_uid, 6_424_431_620_520_340_750);

    // Only fr.soe.a3s.* and allow-listed JDK classes.
    for node in &graph.nodes {
        if let Node::ClassDesc(desc) = node {
            assert!(
                desc.name.starts_with("fr.soe.a3s.") || ALLOWED_JDK.contains(&desc.name.as_str()),
                "class {} is not on Java's allow-list",
                desc.name
            );
        }
    }

    // Profiles: launcher options cleared, tree back-references.
    let profiles = object_list(&graph, root, "profiles");
    assert_eq!(profiles.len(), 2);
    for &profile in &profiles {
        assert_eq!(
            graph.field(profile, "launcherOptions").unwrap(),
            Value::Null
        );
        let tree = graph.field(profile, "tree").unwrap().as_ref().unwrap();
        assert_eq!(graph.field(tree, "parent").unwrap(), Value::Null);
        assert_eq!(graph.field_string(tree, "name").unwrap().unwrap(), "racine");
    }
    let tree = graph.field(profiles[0], "tree").unwrap().as_ref().unwrap();
    let mut leaves = 0;
    check_parents(&graph, tree, &mut leaves);
    assert_eq!(leaves, 3);

    assert_eq!(object_list(&graph, root, "favoriteServers").len(), 2);
}

#[test]
fn repositories_are_written_fresh() {
    let bytes = encode_stream(&sample()).unwrap();
    let graph = a3s_jserial::read_with_limits(&bytes, Limits::JAVA_A3S).unwrap();
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let repositories = object_list(&graph, root, "repositories");
    let [repository] = repositories[..] else {
        panic!("expected one repository");
    };
    let g = &graph;
    let bool_field = |name: &str| g.field_bool(repository, name).unwrap();
    let int_field = |name: &str| g.field_i32(repository, name).unwrap();
    assert!(bool_field("noAutoDiscover"));
    for name in [
        "auto",
        "compressed",
        "exactMatch",
        "noPartialFileTransfer",
        "notify",
        "outOfSynk",
        "uploadCompressedPboFilesOnly",
    ] {
        assert!(!bool_field(name), "{name}");
    }
    assert_eq!(int_field("numberOfClientConnections"), 0);
    assert_eq!(int_field("numberOfConnections"), 1);
    assert_eq!(int_field("numberOfRepositoryCheckConnections"), 4);
    assert_eq!(int_field("parallelUploadConnections"), 4);
    assert_eq!(int_field("revision"), 0);
    assert_eq!(
        g.field_f64(repository, "maximumClientDownloadSpeed")
            .unwrap(),
        0.0
    );
    for name in [
        "autoConfigURL",
        "bitTorrentProtocole",
        "defaultDownloadLocation",
        "path",
        "uploadProtocole",
    ] {
        assert_eq!(g.field(repository, name).unwrap(), Value::Null, "{name}");
    }
    assert_eq!(g.field_string(repository, "name").unwrap().unwrap(), "WOG");

    // Fresh, separate, empty collections of Java's initializer classes.
    let mut instances = Vec::new();
    for (name, class) in [
        ("excludedFilesFromBuild", "java.util.HashSet"),
        ("excludedFoldersFromSync", "java.util.HashSet"),
        ("hidddenFolderPaths", "java.util.HashSet"),
        ("mapEventsDownloadLocation", "java.util.HashMap"),
        ("mapFilesForBuild", "java.util.HashMap"),
        ("mapFilesForSync", "java.util.HashMap"),
        ("favoriteServersSetToAutoconfig", "java.util.ArrayList"),
    ] {
        let value = g.field(repository, name).unwrap();
        let id = value.as_ref().unwrap();
        assert_eq!(g.class_name_of(id).unwrap(), class, "{name}");
        let len = match class {
            "java.util.HashSet" => a3s_jserial::jdk::set(g, value).unwrap().unwrap().len(),
            "java.util.HashMap" => a3s_jserial::jdk::map(g, value).unwrap().unwrap().len(),
            _ => a3s_jserial::jdk::list(g, value).unwrap().unwrap().len(),
        };
        assert_eq!(len, 0, "{name}");
        instances.push(id);
    }
    instances.sort();
    instances.dedup();
    assert_eq!(instances.len(), 7);

    // The download protocol is kept.
    let protocol = g.field(repository, "protocole").unwrap().as_ref().unwrap();
    assert_eq!(g.class_name_of(protocol).unwrap(), "fr.soe.a3s.domain.Ftp");
}

#[test]
fn lenient_decode_of_export() {
    let export = sample();
    let bytes = encode_stream(&export).unwrap();
    let legacy = decode_legacy_stream::<AutoConfigExport>(&bytes).unwrap();
    assert!(legacy.warnings.is_empty(), "{:?}", legacy.warnings);
    let decoded = legacy.value;

    // Profiles come back with default launcher options.
    let mut expected_profiles = export.profiles.clone();
    for p in &mut expected_profiles {
        p.launcher_options = LauncherOptions::default();
    }
    assert_eq!(decoded.profiles, expected_profiles);
    assert_eq!(decoded.favorite_servers, export.favorite_servers);

    // Repositories are reduced to `new Repository(name, protocol)`.
    let original = &export.repositories[0];
    assert_eq!(
        decoded.repositories,
        [Repository::new(
            original.name.clone(),
            original.protocol.clone()
        )]
    );

    // `JavaCodec::decode` is the same lenient decode.
    let via_codec: AutoConfigExport = decode_stream(&bytes).unwrap();
    assert_eq!(via_codec, decoded);
}
