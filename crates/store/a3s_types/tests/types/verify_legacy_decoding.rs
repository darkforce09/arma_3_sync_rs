//! Adversarial lenient-decoding cases (Java-like streams built with `Builder`).
//!
//! Covers edge cases not exercised by `local_legacy.rs`: unknown protocol types and
//! subclasses, extra fields and UIDs of other class versions, zero values of missing fields,
//! alternative collection classes, cache edge values (empty SHA-1, Windows paths, i64 extremes),
//! modified-UTF-8 strings, cyclic and shared profile trees, and malformed streams.

use std::collections::{BTreeMap, BTreeSet};

use a3s_jserial::jdk::{Builder, FieldSpec, block_data};
use a3s_jserial::{ClassData, Content, FieldType, NodeId, Value, flags};
use a3s_types::constants::{IconResize, LookAndFeel, MinimizationType, StartWithOs};
use a3s_types::local::{
    AutoConfigExport, Configuration, FileAttributes, LegacyConfiguration, LegacyDecode,
    LegacyRepository, Preferences, Profile, Proxy, TreeNode, decode_legacy_stream,
};
use a3s_types::{
    EncryptionMode, Error, JavaCodec as _, Protocol, ProtocolType, encode_stream, wire,
};

const S: u8 = flags::SERIALIZABLE;
const STR: &str = "Ljava/lang/String;";
const LIST: &str = "Ljava/util/List;";
const SET: &str = "Ljava/util/Set;";
const MAP: &str = "Ljava/util/Map;";
const AP: &str = "Lfr/soe/a3s/domain/AbstractProtocole;";
const TREE_DIRECTORY_SIG: &str = "Lfr/soe/a3s/domain/TreeDirectory;";

const CONFIGURATION: &str = "fr.soe.a3s.domain.configration.Configuration";
const PREFERENCES: &str = "fr.soe.a3s.domain.Preferences";
const PROFILE: &str = "fr.soe.a3s.domain.Profile";
const TREE_DIRECTORY: &str = "fr.soe.a3s.domain.TreeDirectory";
const TREE_LEAF: &str = "fr.soe.a3s.domain.TreeLeaf";
const REPOSITORY: &str = "fr.soe.a3s.domain.repository.Repository";
const FILE_ATTRIBUTES: &str = "fr.soe.a3s.domain.repository.FileAttributes";

const REPOSITORY_UID: i64 = -8_142_021_113_361_619_970;

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

/// An object of a class without serializable superclass. Fields must be in wire order.
fn object(b: &mut Builder, class: &str, uid: i64, fields: Vec<F>) -> Value {
    let specs: Vec<FieldSpec<'_>> = fields.iter().map(|(f, _)| *f).collect();
    let desc = b.class_desc(class, uid, S, &specs, None);
    b.simple_object(desc, fields.into_iter().map(|(_, v)| v).collect())
}

fn finish(b: Builder, root: Value) -> Vec<u8> {
    a3s_jserial::write(&b.finish(root)).unwrap()
}

fn decode<T: LegacyDecode>(bytes: &[u8]) -> (T, Vec<String>) {
    let legacy = decode_legacy_stream::<T>(bytes).unwrap();
    (legacy.value, legacy.warnings)
}

/// A protocol object of `class` (whose descriptor must already exist or be created by
/// `class_desc`), with the given `protocolType` and `encryptionMode` constants.
fn protocol_object(
    b: &mut Builder,
    class_desc: NodeId,
    protocol_type: Option<&str>,
    encryption_mode: Option<&str>,
) -> Value {
    let id = b.reserve_object(class_desc);
    let encryption =
        encryption_mode.map_or(Value::Null, |m| b.enum_value(wire::ENCRYPTION_MODE, m));
    let ptype = protocol_type.map_or(Value::Null, |t| b.enum_value(wire::PROTOCOL_TYPE, t));
    let values = vec![
        Value::Boolean(true),
        b.string("5000"),
        encryption,
        b.string("user"),
        b.string("secret"),
        b.string("2121"),
        ptype,
        b.string("6000"),
        b.string("files.example/a3s"),
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

/// Repository with just `name` and `protocole` (plus `extra` fields, inserted in wire order by
/// the caller).
fn repository_root(b: &mut Builder, uid: i64, mut fields: Vec<F>, protocol: Value) -> Value {
    fields.push(s(b, "name", Some("R")));
    fields.push(obj("protocole", AP, protocol));
    object(b, REPOSITORY, uid, fields)
}

fn http_protocol(b: &mut Builder) -> Value {
    Protocol::new(ProtocolType::Http, "h", "80", "", "", false).encode(b)
}

// ---------------------------------------------------------------------------------------------
// Protocols

#[test]
fn verify_unknown_protocol_type_falls_back_to_class_with_warning() {
    let mut b = Builder::new();
    let desc = wire::protocol_class_desc(&mut b, wire::FTP);
    let protocol = protocol_object(&mut b, desc, Some("GOPHER"), Some("NO_ENCRYPTION"));
    let root = repository_root(&mut b, REPOSITORY_UID, Vec::new(), protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    let p = decoded.repository.protocol;
    assert_eq!(
        p.protocol_type,
        ProtocolType::Ftp,
        "class fallback (Ftp.java:21-27)"
    );
    assert_eq!(p.url, "files.example/a3s");
    assert_eq!(p.port, "2121");
    assert_eq!(p.login.as_deref(), Some("user"));
    assert_eq!(p.password.as_deref(), Some("secret"));
    assert_eq!(p.connection_timeout.as_deref(), Some("5000"));
    assert_eq!(p.read_timeout.as_deref(), Some("6000"));
    assert!(p.validate_ssl_certificate);
    assert_eq!(p.encryption_mode, Some(EncryptionMode::NoEncryption));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("protocolType"), "{warnings:?}");
    assert!(warnings[0].contains("GOPHER"), "{warnings:?}");
}

#[test]
fn verify_unknown_encryption_mode_reads_none_with_warning() {
    let mut b = Builder::new();
    let desc = wire::protocol_class_desc(&mut b, wire::FTP);
    let protocol = protocol_object(&mut b, desc, Some("FTPS"), Some("QUANTUM_SSL"));
    let root = repository_root(&mut b, REPOSITORY_UID, Vec::new(), protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    let p = decoded.repository.protocol;
    assert_eq!(p.protocol_type, ProtocolType::Ftps);
    assert_eq!(p.encryption_mode, None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("encryptionMode"), "{warnings:?}");
}

#[test]
fn verify_unknown_protocol_subclass() {
    // A future/unknown subclass of AbstractProtocole with a known protocolType decodes through
    // the hierarchy check (any class whose hierarchy contains AbstractProtocole).
    let mut b = Builder::new();
    let base = wire::abstract_protocole_desc(&mut b);
    let desc = b.class_desc("fr.soe.a3s.domain.Gopher", 77, S, &[], Some(base));
    let protocol = protocol_object(&mut b, desc, Some("HTTPS"), None);
    let root = repository_root(&mut b, REPOSITORY_UID, Vec::new(), protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert_eq!(
        decoded.repository.protocol.protocol_type,
        ProtocolType::Https
    );
    assert!(warnings.is_empty(), "{warnings:?}");

    // Same subclass with a null protocolType: required `protocole` is a hard error...
    let mut b = Builder::new();
    let base = wire::abstract_protocole_desc(&mut b);
    let desc = b.class_desc("fr.soe.a3s.domain.Gopher", 77, S, &[], Some(base));
    let protocol = protocol_object(&mut b, desc, None, None);
    let root = repository_root(&mut b, REPOSITORY_UID, Vec::new(), protocol);
    let err = decode_legacy_stream::<LegacyRepository>(&finish(b, root)).unwrap_err();
    assert!(matches!(err, Error::UnexpectedClass { .. }), "{err:?}");

    // ...while the optional `uploadProtocole` is dropped with a warning.
    let mut b = Builder::new();
    let base = wire::abstract_protocole_desc(&mut b);
    let desc = b.class_desc("fr.soe.a3s.domain.Gopher", 77, S, &[], Some(base));
    let upload = protocol_object(&mut b, desc, None, None);
    let main = http_protocol(&mut b);
    let fields = vec![
        s(&mut b, "name", Some("R")),
        obj("protocole", AP, main),
        obj("uploadProtocole", AP, upload),
    ];
    let root = object(&mut b, REPOSITORY, REPOSITORY_UID, fields);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert_eq!(decoded.repository.upload_protocol, None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("uploadProtocole"), "{warnings:?}");
}

#[test]
fn verify_protocol_without_url_and_port_reads_empty_strings() {
    // An AbstractProtocole class version that only has protocolType (url/port missing
    // read as "", login/password/timeouts stay None, validateSSL false).
    let mut b = Builder::new();
    let base = b.class_desc(
        wire::ABSTRACT_PROTOCOLE,
        12345,
        S,
        &[FieldSpec::object(
            "protocolType",
            "Lfr/soe/a3s/constant/ProtocolType;",
        )],
        None,
    );
    let desc = b.class_desc(wire::SOCKS, -40, S, &[], Some(base));
    let id = b.reserve_object(desc);
    let ptype = b.enum_value(wire::PROTOCOL_TYPE, "SOCKS5");
    b.set_object_data(
        id,
        vec![
            ClassData {
                values: vec![ptype],
                annotations: Vec::new(),
            },
            ClassData::default(),
        ],
    );
    let root = repository_root(&mut b, REPOSITORY_UID, Vec::new(), Value::Ref(id));
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    let p = decoded.repository.protocol;
    assert_eq!(p.protocol_type, ProtocolType::Socks5);
    assert_eq!(p.url, "");
    assert_eq!(p.port, "");
    assert_eq!(p.login, None);
    assert_eq!(p.password, None);
    assert_eq!(p.connection_timeout, None);
    assert_eq!(p.read_timeout, None);
    assert!(!p.validate_ssl_certificate);
    assert_eq!(p.encryption_mode, None);
}

// ---------------------------------------------------------------------------------------------
// Class versions: UIDs, extra and missing fields

#[test]
fn verify_other_uid_and_extra_fields_are_ignored_silently() {
    let mut b = Builder::new();
    let unknown_class = b.class_desc(
        "fr.soe.a3s.domain.repository.SomethingNew",
        42,
        S,
        &[FieldSpec::prim(FieldType::Int, "x")],
        None,
    );
    let unknown = b.simple_object(unknown_class, vec![Value::Int(1)]);
    let fields = vec![
        z("auto", true),
        long("bogusLong", 99),
        int("revision", 17),
        s(&mut b, "bogusString", Some("x")),
        obj(
            "bogusThing",
            "Lfr/soe/a3s/domain/repository/SomethingNew;",
            unknown,
        ),
    ];
    let protocol = http_protocol(&mut b);
    let root = repository_root(&mut b, 1, fields, protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert!(
        warnings.is_empty(),
        "unknown fields must not warn: {warnings:?}"
    );
    assert!(decoded.repository.auto);
    assert_eq!(decoded.repository.revision, 17);
}

#[test]
fn verify_minimal_repository_takes_java_zero_values() {
    let mut b = Builder::new();
    let protocol = http_protocol(&mut b);
    let root = repository_root(&mut b, REPOSITORY_UID, Vec::new(), protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    let r = decoded.repository;
    // Java zero values, not field initializers.
    assert!(!r.no_auto_discover);
    assert_eq!(r.number_of_connections, 0, "raw value kept");
    assert_eq!(r.number_of_repository_check_connections, 0);
    assert_eq!(r.parallel_upload_connections, 0);
    assert_eq!(r.repository_check_connections(), 4);
    assert_eq!(r.parallel_upload_connections(), 4);
    assert_eq!(r.maximum_client_download_speed.to_bits(), 0.0_f64.to_bits());
    assert!(r.use_partial_file_transfer());
    assert!(r.hidden_folder_paths.is_empty());
    assert!(r.events_download_location.is_empty());
    assert!(r.favorite_servers_set_to_autoconfig.is_empty());
    assert_eq!(r.bit_torrent_protocol, None);
    assert!(decoded.files_for_sync.is_empty());
    assert!(decoded.files_for_build.is_empty());
}

#[test]
fn verify_empty_configuration_class_takes_zero_values() {
    let mut b = Builder::new();
    let root = object(&mut b, CONFIGURATION, 1, Vec::new());
    let (decoded, warnings) = decode::<LegacyConfiguration>(&finish(b, root));
    let c = decoded.configuration;
    // Missing boolean is Java's zero value `false`, not the initializer `true`.
    assert!(!c.view_mode_tree);
    assert_eq!(c.height, 0);
    assert_eq!(c.width, 0);
    // Null profileName/gameVersion fall back with a warning each.
    assert_eq!(c.profile_name, "Default");
    assert_eq!(c.game_version, "Arma 3");
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert_eq!(c.proxy, Proxy::default());
    let expected_rest = Configuration {
        view_mode_tree: false,
        ..Configuration::default()
    };
    assert_eq!(c, expected_rest);
    assert_eq!(decoded.addon_search_directory_paths, None);
    assert_eq!(decoded.launcher_options, None);
}

#[test]
fn verify_empty_preferences_class_uses_getter_defaults() {
    let mut b = Builder::new();
    let root = object(&mut b, PREFERENCES, 1, Vec::new());
    let (p, warnings) = decode::<Preferences>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(p.look_and_feel, LookAndFeel::LafDefault);
    assert_eq!(p.icon_resize_size, IconResize::Auto, "null getter is AUTO");
    assert_eq!(p.start_with_os, StartWithOs::Disabled);
    assert!(p.prefer_github_updates);
    assert_eq!(p.launch_panel_game_launch, MinimizationType::TaskBar);
    assert_eq!(p.launch_panel_minimized, MinimizationType::TaskBar);
}

#[test]
fn verify_preferences_value_kinds() {
    let mut b = Builder::new();
    let f = b.boolean(false);
    let fields = vec![
        en(
            &mut b,
            "iconResizeSize",
            "fr.soe.a3s.constant.IconResize",
            "NONE",
        ),
        en(
            &mut b,
            "launchPanelMinimized",
            "fr.soe.a3s.constant.MinimizationType",
            "CLOSE",
        ),
        obj("preferGitHubUpdates", "Ljava/lang/Boolean;", f),
    ];
    let root = object(&mut b, PREFERENCES, 7_034_433_806_261_372_240, fields);
    let (p, warnings) = decode::<Preferences>(&finish(b, root));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(p.icon_resize_size, IconResize::Disabled);
    assert_eq!(p.launch_panel_minimized, MinimizationType::Close);
    assert!(!p.prefer_github_updates, "Boolean.FALSE must be honoured");

    // A String where a Boolean is expected: getter default `true` plus a warning.
    let mut b = Builder::new();
    let fields = vec![s(&mut b, "preferGitHubUpdates", Some("false"))];
    let root = object(&mut b, PREFERENCES, 7_034_433_806_261_372_240, fields);
    let (p, warnings) = decode::<Preferences>(&finish(b, root));
    assert!(p.prefer_github_updates);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

// ---------------------------------------------------------------------------------------------
// Collections (wire-compat notes: any supported List/Set/Map)

fn tree_set(b: &mut Builder, elements: &[&str]) -> Value {
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

fn linked_hash_map(b: &mut Builder, entries: &[(&str, &str)]) -> Value {
    let base = b.class_desc(
        "java.util.HashMap",
        362_498_820_763_181_265,
        S | flags::WRITE_METHOD,
        &[
            FieldSpec::prim(FieldType::Float, "loadFactor"),
            FieldSpec::prim(FieldType::Int, "threshold"),
        ],
        None,
    );
    let class = b.class_desc(
        "java.util.LinkedHashMap",
        3_801_124_242_820_219_131,
        S,
        &[FieldSpec::prim(FieldType::Boolean, "accessOrder")],
        Some(base),
    );
    let id = b.reserve_object(class);
    let mut header = Vec::new();
    header.extend_from_slice(&16_i32.to_be_bytes());
    header.extend_from_slice(&i32::try_from(entries.len()).unwrap().to_be_bytes());
    let mut annotations = block_data(&header);
    for (k, v) in entries {
        let k = b.string(k);
        let v = b.string(v);
        annotations.push(Content::Value(k));
        annotations.push(Content::Value(v));
    }
    b.set_object_data(
        id,
        vec![
            ClassData {
                values: vec![Value::Float(0.75), Value::Int(12)],
                annotations,
            },
            ClassData {
                values: vec![Value::Boolean(false)],
                annotations: Vec::new(),
            },
        ],
    );
    Value::Ref(id)
}

fn linked_list(b: &mut Builder, elements: Vec<Value>) -> Value {
    let class = b.class_desc(
        "java.util.LinkedList",
        876_323_262_645_176_354,
        S | flags::WRITE_METHOD,
        &[],
        None,
    );
    let id = b.reserve_object(class);
    let mut annotations = block_data(&i32::try_from(elements.len()).unwrap().to_be_bytes());
    annotations.extend(elements.into_iter().map(Content::Value));
    b.set_object_data(
        id,
        vec![ClassData {
            values: Vec::new(),
            annotations,
        }],
    );
    Value::Ref(id)
}

#[test]
fn verify_alternative_collection_classes() {
    let mut b = Builder::new();
    let hidden = tree_set(&mut b, &["/b", "/a"]);
    let events = linked_hash_map(&mut b, &[("ev1", "C:\\Arma 3\\@ev1"), ("ev2", "/srv/ev2")]);
    let n1 = b.string("x");
    let n2 = b.string("y");
    let excluded = linked_list(&mut b, vec![n1, Value::Null, n2]);
    let fields = vec![
        obj("excludedFoldersFromSync", SET, Value::Null),
        obj("favoriteServersSetToAutoconfig", LIST, excluded),
        obj("hidddenFolderPaths", SET, hidden),
        obj("mapEventsDownloadLocation", MAP, events),
    ];
    let protocol = http_protocol(&mut b);
    let root = repository_root(&mut b, REPOSITORY_UID, fields, protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    let r = decoded.repository;
    assert_eq!(
        r.hidden_folder_paths,
        BTreeSet::from(["/a".to_owned(), "/b".to_owned()])
    );
    assert_eq!(
        r.events_download_location,
        BTreeMap::from([
            ("ev1".to_owned(), "C:\\Arma 3\\@ev1".to_owned()),
            ("ev2".to_owned(), "/srv/ev2".to_owned()),
        ])
    );
    assert!(r.excluded_folders_from_sync.is_empty());
    // Strings in a list of FavoriteServer: skipped with warnings, null skipped silently.
    assert!(r.favorite_servers_set_to_autoconfig.is_empty());
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}

// ---------------------------------------------------------------------------------------------
// SHA-1 caches

fn file_attributes(b: &mut Builder, sha1: Option<&str>, last_modified: i64) -> Value {
    let fields = vec![long("lastModified", last_modified), s(b, "sha1", sha1)];
    object(b, FILE_ATTRIBUTES, -2_500_913_557_576_506_383, fields)
}

#[test]
fn verify_file_cache_edge_values() {
    let mut b = Builder::new();
    let windows = "C:\\Arma 3\\@mod\\addons\\a.pbo";
    let unix_like = "c:/arma 3/@mod/addons/a.pbo";
    let unicode = "/home/j\u{e9}r\u{f4}me/\u{1F600}/\u{0}x.pbo";
    let entries = vec![
        (
            windows.to_owned(),
            file_attributes(&mut b, Some("aa"), i64::MAX),
        ),
        (
            unix_like.to_owned(),
            file_attributes(&mut b, Some("bb"), i64::MIN),
        ),
        (unicode.to_owned(), file_attributes(&mut b, Some("0"), -1)),
        ("/empty".to_owned(), file_attributes(&mut b, Some(""), 0)),
    ];
    let not_attributes = b.string("not FileAttributes");
    let mut all = entries;
    all.push(("/wrong".to_owned(), not_attributes));
    let cache = b.hash_map_str(&all);
    let fields = vec![obj("mapFilesForBuild", MAP, cache)];
    let protocol = http_protocol(&mut b);
    let root = repository_root(&mut b, REPOSITORY_UID, fields, protocol);
    let (decoded, warnings) = decode::<LegacyRepository>(&finish(b, root));
    let expected = BTreeMap::from([
        (
            windows.to_owned(),
            FileAttributes {
                sha1: "aa".to_owned(),
                last_modified: i64::MAX,
            },
        ),
        (
            unix_like.to_owned(),
            FileAttributes {
                sha1: "bb".to_owned(),
                last_modified: i64::MIN,
            },
        ),
        (
            unicode.to_owned(),
            FileAttributes {
                sha1: "0".to_owned(),
                last_modified: -1,
            },
        ),
        (
            "/empty".to_owned(),
            FileAttributes {
                sha1: String::new(),
                last_modified: 0,
            },
        ),
    ]);
    assert_eq!(decoded.files_for_build, expected);
    assert!(decoded.files_for_sync.is_empty());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

// ---------------------------------------------------------------------------------------------
// Profiles: strings, ordering, cycles, sharing

fn tree_dir_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
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
    )
}

fn set_dir(b: &mut Builder, id: NodeId, name: &str, children: Vec<Value>, parent: Option<NodeId>) {
    let list = b.array_list(children);
    let name = b.string(name);
    b.set_object_data(
        id,
        vec![ClassData {
            values: vec![
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(false),
                list,
                Value::Null,
                Value::Null,
                name,
                parent.map_or(Value::Null, Value::Ref),
            ],
            annotations: Vec::new(),
        }],
    );
}

fn leaf(b: &mut Builder, name: &str, parent: NodeId) -> Value {
    let fields = vec![
        z("duplicate", false),
        z("optional", false),
        z("selected", true),
        s(b, "dlcAppId", None),
        s(b, "name", Some(name)),
        obj("parent", TREE_DIRECTORY_SIG, Value::Ref(parent)),
        s(b, "workshopId", None),
    ];
    object(b, TREE_LEAF, 8_681_722_025_933_532_378, fields)
}

fn profile_root(b: &mut Builder, name: &str, tree: Value, priorities: &[String]) -> Value {
    let priorities: Vec<Value> = priorities.iter().map(|p| b.string(p)).collect();
    let priorities = b.array_list(priorities);
    let fields = vec![
        obj("addonNamesByPriority", LIST, priorities),
        s(b, "name", Some(name)),
        obj("tree", TREE_DIRECTORY_SIG, tree),
    ];
    object(b, PROFILE, -4_395_099_790_449_721_021, fields)
}

#[test]
fn verify_tree_keeps_stream_order_with_case_duplicates() {
    let mut b = Builder::new();
    let desc = tree_dir_desc(&mut b);
    let root = b.reserve_object(desc);
    let z_upper = b.reserve_object(desc);
    let z_lower = b.reserve_object(desc);
    set_dir(&mut b, z_upper, "Z", Vec::new(), Some(root));
    set_dir(&mut b, z_lower, "z", Vec::new(), Some(root));
    let leaf_b = leaf(&mut b, "b", root);
    let leaf_a_upper = leaf(&mut b, "A", root);
    let leaf_a_lower = leaf(&mut b, "a", root);
    set_dir(
        &mut b,
        root,
        "racine",
        vec![
            leaf_b,
            Value::Ref(z_upper),
            leaf_a_upper,
            leaf_a_lower,
            Value::Ref(z_lower),
        ],
        None,
    );
    let profile = profile_root(&mut b, "P", Value::Ref(root), &[]);
    let (decoded, warnings) = decode::<Profile>(&finish(b, profile));
    assert!(warnings.is_empty(), "{warnings:?}");
    let names: Vec<(&str, bool)> = decoded
        .tree
        .children
        .iter()
        .map(|c| (c.name(), c.is_leaf()))
        .collect();
    assert_eq!(
        names,
        [
            ("b", true),
            ("Z", false),
            ("A", true),
            ("a", true),
            ("z", false)
        ],
        "decode must keep stream order and not re-sort"
    );
}

#[test]
fn verify_unicode_and_long_strings() {
    // Supplementary characters (surrogate pairs in modified UTF-8), NUL (0xC0 0x80) and a
    // string longer than 65535 bytes (TC_LONGSTRING).
    let name = "Pr\u{f6}fil \u{1F680} \u{0} \u{4E2D}\u{6587}";
    let long = format!("/mods/{}", "\u{e9}".repeat(40_000));
    let mut b = Builder::new();
    let desc = tree_dir_desc(&mut b);
    let root = b.reserve_object(desc);
    let l = leaf(&mut b, "@\u{1F600}mod", root);
    set_dir(&mut b, root, "racine", vec![l], None);
    let profile = profile_root(&mut b, name, Value::Ref(root), std::slice::from_ref(&long));
    let (decoded, warnings) = decode::<Profile>(&finish(b, profile));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(decoded.name, name);
    assert_eq!(decoded.addon_names_by_priority, [long]);
    assert_eq!(decoded.tree.children[0].name(), "@\u{1F600}mod");
}

#[test]
fn verify_cyclic_tree_is_an_error_not_a_crash() {
    let mut b = Builder::new();
    let desc = tree_dir_desc(&mut b);
    let root = b.reserve_object(desc);
    set_dir(&mut b, root, "racine", vec![Value::Ref(root)], None);
    let profile = profile_root(&mut b, "P", Value::Ref(root), &[]);
    let bytes = finish(b, profile);
    let result = decode_legacy_stream::<Profile>(&bytes);
    assert!(
        result.is_err(),
        "a self-containing directory must be rejected"
    );
}

/// Counts every node of a decoded tree.
fn count_nodes(children: &[TreeNode]) -> usize {
    children
        .iter()
        .map(|c| match c {
            TreeNode::Directory(d) => 1 + count_nodes(&d.children),
            TreeNode::Leaf(_) => 1,
        })
        .sum()
}

#[test]
fn verify_shared_subtrees_do_not_expand_exponentially() {
    // A hostile `export.a3s.autoconfig` or profile can reference the same TreeDirectory twice
    // from its parent at every level. Java keeps the DAG (one object per level); the Rust
    // decoder copies each shared subtree, so the decoded size is 2^depth. With the reader's
    // default depth limit (1000) a few hundred bytes can describe 2^400 nodes, which hangs the
    // client. Depth 16 keeps the test fast; a legitimate (tree-shaped) profile never decodes
    // to more tree nodes than the stream has nodes, so that is the bound checked here.
    const DEPTH: usize = 16;
    let mut b = Builder::new();
    let desc = tree_dir_desc(&mut b);
    let ids: Vec<NodeId> = (0..=DEPTH).map(|_| b.reserve_object(desc)).collect();
    for level in (0..=DEPTH).rev() {
        let parent = (level > 0).then(|| ids[level - 1]);
        let children = if level == DEPTH {
            vec![leaf(&mut b, "@x", ids[level])]
        } else {
            vec![Value::Ref(ids[level + 1]), Value::Ref(ids[level + 1])]
        };
        let name = format!("d{level}");
        set_dir(&mut b, ids[level], &name, children, parent);
    }
    let profile = profile_root(&mut b, "P", Value::Ref(ids[0]), &[]);
    let bytes = finish(b, profile);
    let stream_objects = a3s_jserial::read(&bytes).unwrap().nodes.len();
    match decode_legacy_stream::<Profile>(&bytes) {
        Err(_) => {}
        Ok(legacy) => {
            let nodes = count_nodes(&legacy.value.tree.children);
            assert!(
                nodes <= stream_objects,
                "decoded {nodes} tree nodes from a stream of {stream_objects} nodes: shared \
                 subtrees are expanded without a budget"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Malformed input

#[test]
fn verify_malformed_streams_are_errors() {
    assert!(decode_legacy_stream::<Profile>(&[]).is_err());
    assert!(decode_legacy_stream::<Preferences>(&[0xAC, 0xED, 0x00, 0x05]).is_err());

    // Null root and a String root.
    let b = Builder::new();
    let bytes = finish(b, Value::Null);
    assert!(decode_legacy_stream::<LegacyConfiguration>(&bytes).is_err());
    let mut b = Builder::new();
    let root = b.string("fr.soe.a3s.domain.Preferences");
    let bytes = finish(b, root);
    assert!(decode_legacy_stream::<Preferences>(&bytes).is_err());

    // Every strict prefix of a valid export is rejected without panicking.
    let mut export = AutoConfigExport::default();
    export.profiles.push(Profile::new("P"));
    export.repositories.push(a3s_types::local::Repository::new(
        "R",
        Protocol::new(ProtocolType::Ftp, "h/p", "21", "u", "p", true),
    ));
    let bytes = encode_stream(&export).unwrap();
    for len in 0..bytes.len() {
        assert!(
            decode_legacy_stream::<AutoConfigExport>(&bytes[..len]).is_err(),
            "prefix of {len} bytes accepted"
        );
    }
    assert!(decode_legacy_stream::<AutoConfigExport>(&bytes).is_ok());
}

#[test]
fn verify_export_elements_with_hard_errors_are_skipped() {
    // Hard errors inside an export element skip that element (with a warning) instead of
    // failing the whole import.
    let mut b = Builder::new();
    let class = a3s_types::local::wire::auto_config_export_desc(&mut b);
    let bad_repo = {
        let protocol = Value::Null;
        let fields = vec![s(&mut b, "name", Some("R")), obj("protocole", AP, protocol)];
        object(&mut b, REPOSITORY, REPOSITORY_UID, fields)
    };
    let good_repo = {
        let protocol = http_protocol(&mut b);
        repository_root(&mut b, REPOSITORY_UID, Vec::new(), protocol)
    };
    let bad_profile = {
        let fields = vec![obj("name", STR, Value::Null)];
        object(&mut b, PROFILE, -4_395_099_790_449_721_021, fields)
    };
    let servers = b.array_list(Vec::new());
    let profiles = b.array_list(vec![bad_profile]);
    let repositories = b.array_list(vec![bad_repo, good_repo]);
    let root = b.simple_object(class, vec![servers, profiles, repositories]);
    let (decoded, warnings) = decode::<AutoConfigExport>(&finish(b, root));
    assert!(decoded.profiles.is_empty());
    assert_eq!(decoded.repositories.len(), 1);
    assert_eq!(decoded.repositories[0].name, "R");
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}
