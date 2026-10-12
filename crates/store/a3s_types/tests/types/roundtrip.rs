//! Encode → decode round trips of every `.a3s` file type.
//!
//! Byte-for-byte comparison against Java output lives in `a3s_repo_files`' golden tests.

use std::collections::{BTreeMap, BTreeSet};

use a3s_jserial::Limits;
use a3s_types::{
    AutoConfig, Changelog, Changelogs, Event, Events, FavoriteServer, JavaCodec, Protocol,
    ProtocolType, ServerInfo, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode, decode_stream,
    encode_stream,
};

fn round_trip<T: JavaCodec + PartialEq + std::fmt::Debug>(value: &T) -> Vec<u8> {
    let bytes = encode_stream(value).unwrap();
    // Everything we write must be accepted by Arma3Sync's Java reader filter.
    a3s_jserial::read_with_limits(&bytes, Limits::JAVA_A3S).unwrap();
    let decoded: T = decode_stream(&bytes).unwrap();
    assert_eq!(&decoded, value);
    bytes
}

fn leaf(name: &str, size: i64) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: Some(if size == 0 {
            "0".to_owned()
        } else {
            format!("{size:040x}")
        }),
        size,
        ..Default::default()
    })
}

fn sample_tree() -> SyncTreeDirectory {
    let mut addons = SyncTreeDirectory::new("addons");
    addons.add_child(leaf("cba_main.pbo", 1234));
    addons.add_child(leaf("cba_main.pbo.cba_3.bisign", 567));

    let mut mod_dir = SyncTreeDirectory::new("@CBA_A3");
    mod_dir.mark_as_addon = true;
    mod_dir.add_child(leaf("mod.cpp", 0));
    mod_dir.add_child(SyncTreeNode::Directory(addons));

    let mut root = SyncTreeDirectory::new_root();
    root.add_child(leaf("readme.txt", 10));
    root.add_child(SyncTreeNode::Directory(mod_dir));
    root.add_child(SyncTreeNode::Directory(SyncTreeDirectory::new("@ace")));
    root
}

#[test]
fn sync_tree() {
    let tree = sample_tree();
    // Directories first, case-insensitive order.
    let names: Vec<&str> = tree.children.iter().map(|c| c.name()).collect();
    assert_eq!(names, ["@ace", "@CBA_A3", "readme.txt"]);
    assert_eq!(tree.leaf_count(), 4);

    let bytes = round_trip(&tree);

    // Parent back-references exist in the Java graph.
    let graph = a3s_jserial::read(&bytes).unwrap();
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let children = a3s_jserial::jdk::list(&graph, graph.field(root, "list").unwrap())
        .unwrap()
        .unwrap();
    for child in children {
        let child = child.as_ref().unwrap();
        assert_eq!(
            graph.field(child, "parent").unwrap(),
            a3s_jserial::Value::Ref(root)
        );
    }
    assert_eq!(
        graph.field(root, "parent").unwrap(),
        a3s_jserial::Value::Null
    );
}

#[test]
fn server_info() {
    round_trip(&ServerInfo::default());
    round_trip(&ServerInfo {
        revision: 42,
        build_date: Some(1_728_000_000_000),
        number_of_files: 1234,
        total_files_size: 50_000_000_000,
        hidden_folder_paths: BTreeSet::from(["@secret".to_owned(), "a/b".to_owned()]),
        number_of_connections: 4,
        no_partial_file_transfer: true,
        repository_content_updated: true,
        compressed_pbo_files_only: false,
    });
}

#[test]
fn changelogs() {
    round_trip(&Changelogs::default());
    round_trip(&Changelogs {
        list: vec![
            Changelog {
                revision: 1,
                build_date: Some(1),
                content_updated: true,
                new_addons: vec!["@ace".to_owned()],
                updated_addons: vec![],
                deleted_addons: vec![],
                addons: vec!["@ace".to_owned()],
            },
            Changelog {
                revision: 2,
                build_date: None,
                content_updated: false,
                new_addons: vec![],
                updated_addons: vec!["@ace".to_owned()],
                deleted_addons: vec!["@old".to_owned()],
                addons: vec!["@ace".to_owned(), "@cba".to_owned()],
            },
        ],
    });
}

#[test]
fn events() {
    round_trip(&Events::default());
    round_trip(&Events {
        list: vec![Event {
            name: "Sunday op".to_owned(),
            description: Some("Main event".to_owned()),
            addon_names: BTreeMap::from([("@ace".to_owned(), false), ("@tfar".to_owned(), true)]),
            dlc_names: BTreeMap::from([("Contact".to_owned(), true)]),
            userconfig_folder_names: BTreeMap::new(),
        }],
    });
}

#[test]
fn auto_config() {
    for protocol_type in [
        ProtocolType::Http,
        ProtocolType::Https,
        ProtocolType::Ftp,
        ProtocolType::Ftps,
        ProtocolType::Sftp,
        ProtocolType::HttpWebdav,
        ProtocolType::HttpsWebdav,
    ] {
        round_trip(&AutoConfig {
            repository_name: Some("OFCRA".to_owned()),
            protocol: Some(Protocol::new(
                protocol_type,
                "repo.example.org/arma",
                protocol_type.default_port(),
                "anonymous",
                "",
                true,
            )),
            favorite_servers: vec![FavoriteServer {
                name: Some("Main".to_owned()),
                ip_address: Some("1.2.3.4".to_owned()),
                port: 2302,
                password: Some("pw".to_owned()),
                selected: false,
                modset_name: None,
                repository_name: Some("OFCRA".to_owned()),
            }],
        });
    }
}

#[test]
fn protocol_paths() {
    let p = Protocol::new(ProtocolType::Https, "host.org/a/b", "443", "x", "y", true);
    assert_eq!(p.hostname(), "host.org");
    assert_eq!(p.remote_path(), "/a/b");
    let p = Protocol::new(ProtocolType::Https, "host.org", "443", "x", "y", true);
    assert_eq!(p.remote_path(), "");
    assert!(!p.is_anonymous());
}

/// A crafted stream whose directory lists the same child directory twice, nested, must be
/// rejected instead of being expanded exponentially.
#[test]
fn shared_directory_nodes_are_rejected() {
    use a3s_jserial::{ClassData, Value as V, jdk::Builder};
    let mut b = Builder::new();
    let class = a3s_types::wire::sync_tree_directory_desc(&mut b);
    // Build 40 levels where each level's list holds the next level twice.
    let mut inner = b.reserve_object(class);
    let empty = b.array_list(vec![]);
    let name = b.string("leaf-dir");
    b.set_object_data(
        inner,
        vec![ClassData {
            values: vec![
                V::Boolean(false),
                V::Boolean(false),
                V::Boolean(false),
                V::Boolean(false),
                empty,
                name,
                V::Null,
            ],
            annotations: vec![],
        }],
    );
    for level in 0..40 {
        let outer = b.reserve_object(class);
        let list = b.array_list(vec![V::Ref(inner), V::Ref(inner)]);
        let name = b.string(&format!("level{level}"));
        b.set_object_data(
            outer,
            vec![ClassData {
                values: vec![
                    V::Boolean(false),
                    V::Boolean(false),
                    V::Boolean(false),
                    V::Boolean(false),
                    list,
                    name,
                    V::Null,
                ],
                annotations: vec![],
            }],
        );
        inner = outer;
    }
    let bytes = a3s_jserial::write(&b.finish(V::Ref(inner))).unwrap();
    let start = std::time::Instant::now();
    let result: Result<SyncTreeDirectory, _> = decode_stream(&bytes);
    assert!(result.is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}
