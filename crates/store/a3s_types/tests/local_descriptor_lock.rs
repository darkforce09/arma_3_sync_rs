//! Locks the Java class descriptors written to `export.a3s.autoconfig` (A15).
//!
//! Every descriptor in `a3s_types::local::wire` is listed here with its exact name,
//! serialVersionUID, flags, fields in wire order and superclass. Java clients reject files whose
//! descriptors differ, so this file must only change together with a deliberate, documented
//! wire-format decision. Agents are not allowed to edit it.

use a3s_jserial::jdk::Builder;
use a3s_jserial::{NodeId, flags};
use a3s_types::local::wire;

/// (type code, field name, JVM signature for object fields).
type Field = (char, &'static str, Option<&'static str>);

/// A descriptor constructor of `local::wire`.
type MakeDesc = fn(&mut Builder) -> NodeId;

struct Expected {
    name: &'static str,
    uid: i64,
    flags: u8,
    fields: &'static [Field],
    super_class: Option<&'static str>,
}

const S: u8 = flags::SERIALIZABLE;
const STR: Option<&str> = Some("Ljava/lang/String;");
const LIST: Option<&str> = Some("Ljava/util/List;");
const SET: Option<&str> = Some("Ljava/util/Set;");
const MAP: Option<&str> = Some("Ljava/util/Map;");
const AP: Option<&str> = Some("Lfr/soe/a3s/domain/AbstractProtocole;");
const TREE_DIRECTORY: Option<&str> = Some("Lfr/soe/a3s/domain/TreeDirectory;");

fn profile_classes() -> Vec<(MakeDesc, Expected)> {
    vec![
        (
            wire::auto_config_export_desc,
            Expected {
                name: "fr.soe.a3s.domain.AutoConfig",
                // PROVISIONAL — confirm with WP01: JVM-computed default UID (the class declares
                // none), computed from the Java source.
                uid: 6_424_431_620_520_340_750,
                flags: S,
                fields: &[
                    ('L', "favoriteServers", LIST),
                    ('L', "profiles", LIST),
                    ('L', "repositories", LIST),
                ],
                super_class: None,
            },
        ),
        (
            wire::profile_desc,
            Expected {
                name: "fr.soe.a3s.domain.Profile",
                uid: -4_395_099_790_449_721_021,
                flags: S,
                fields: &[
                    ('L', "additionalParameters", STR),
                    ('L', "addonNamesByPriority", LIST),
                    ('L', "addonSearchDirectories", LIST),
                    ('L', "lastAddedAddonSearchDirectory", STR),
                    (
                        'L',
                        "launcherOptions",
                        Some("Lfr/soe/a3s/domain/configration/LauncherOptions;"),
                    ),
                    ('L', "name", STR),
                    ('L', "tree", TREE_DIRECTORY),
                ],
                super_class: None,
            },
        ),
        (
            wire::tree_directory_desc,
            Expected {
                name: "fr.soe.a3s.domain.TreeDirectory",
                uid: -3_917_102_856_123_611_985,
                flags: S,
                fields: &[
                    ('Z', "marked", None),
                    ('Z', "optional", None),
                    ('Z', "selected", None),
                    ('L', "list", LIST),
                    ('L', "modsetRepositoryName", STR),
                    ('L', "modsetType", Some("Lfr/soe/a3s/constant/ModsetType;")),
                    ('L', "name", STR),
                    ('L', "parent", TREE_DIRECTORY),
                ],
                super_class: None,
            },
        ),
        (
            wire::tree_leaf_desc,
            Expected {
                name: "fr.soe.a3s.domain.TreeLeaf",
                uid: 8_681_722_025_933_532_378,
                flags: S,
                fields: &[
                    ('Z', "duplicate", None),
                    ('Z', "optional", None),
                    ('Z', "selected", None),
                    ('L', "dlcAppId", STR),
                    ('L', "name", STR),
                    ('L', "parent", TREE_DIRECTORY),
                    ('L', "workshopId", STR),
                ],
                super_class: None,
            },
        ),
        (
            wire::modset_type_desc,
            Expected {
                name: "fr.soe.a3s.constant.ModsetType",
                uid: 0,
                flags: S | flags::ENUM,
                fields: &[],
                super_class: Some("java.lang.Enum"),
            },
        ),
    ]
}

fn repository_classes() -> Vec<(MakeDesc, Expected)> {
    vec![(
        wire::repository_desc,
        Expected {
            name: "fr.soe.a3s.domain.repository.Repository",
            uid: -8_142_021_113_361_619_970,
            flags: S,
            fields: &[
                ('Z', "auto", None),
                ('Z', "compressed", None),
                ('Z', "exactMatch", None),
                ('D', "maximumClientDownloadSpeed", None),
                ('Z', "noAutoDiscover", None),
                ('Z', "noPartialFileTransfer", None),
                ('Z', "notify", None),
                ('I', "numberOfClientConnections", None),
                ('I', "numberOfConnections", None),
                ('I', "numberOfRepositoryCheckConnections", None),
                ('Z', "outOfSynk", None),
                ('I', "parallelUploadConnections", None),
                ('I', "revision", None),
                ('Z', "uploadCompressedPboFilesOnly", None),
                ('L', "autoConfigURL", STR),
                ('L', "bitTorrentProtocole", AP),
                ('L', "defaultDownloadLocation", STR),
                ('L', "excludedFilesFromBuild", SET),
                ('L', "excludedFoldersFromSync", SET),
                ('L', "favoriteServersSetToAutoconfig", LIST),
                ('L', "hidddenFolderPaths", SET),
                ('L', "mapEventsDownloadLocation", MAP),
                ('L', "mapFilesForBuild", MAP),
                ('L', "mapFilesForSync", MAP),
                ('L', "name", STR),
                ('L', "path", STR),
                ('L', "protocole", AP),
                ('L', "uploadProtocole", AP),
            ],
            super_class: None,
        },
    )]
}

fn check(make: MakeDesc, want: &Expected) {
    let mut b = Builder::new();
    let id = make(&mut b);
    let graph = b.graph();
    let desc = graph.class_desc(id).unwrap();
    assert_eq!(desc.name, want.name);
    assert_eq!(desc.serial_version_uid, want.uid, "{}", want.name);
    assert_eq!(desc.flags, want.flags, "{}", want.name);
    let fields: Vec<(char, String, Option<String>)> = desc
        .fields
        .iter()
        .map(|f| {
            let sig = f.class_name.map(|s| graph.string(s).unwrap().to_owned());
            (f.field_type.code() as char, f.name.clone(), sig)
        })
        .collect();
    let expected: Vec<(char, String, Option<String>)> = want
        .fields
        .iter()
        .map(|(c, n, s)| (*c, (*n).to_owned(), s.map(str::to_owned)))
        .collect();
    assert_eq!(fields, expected, "{}", want.name);
    let super_name = desc
        .super_class
        .map(|s| graph.class_desc(s).unwrap().name.clone());
    assert_eq!(super_name.as_deref(), want.super_class, "{}", want.name);
}

#[test]
fn a15_class_descriptors_are_locked() {
    let all: Vec<_> = profile_classes()
        .into_iter()
        .chain(repository_classes())
        .collect();
    assert_eq!(all.len(), 6, "every local wire class must be listed");
    for (make, want) in &all {
        check(*make, want);
    }
}

#[test]
fn a15_class_names_and_enum_constants_are_locked() {
    use a3s_types::constants::ModsetType;
    assert_eq!(wire::AUTO_CONFIG_EXPORT, "fr.soe.a3s.domain.AutoConfig");
    assert_eq!(wire::MODSET_TYPE, "fr.soe.a3s.constant.ModsetType");
    assert_eq!(wire::REPOSITORY, "fr.soe.a3s.domain.repository.Repository");
    assert_eq!(wire::AUTO_CONFIG_EXPORT_UID, 6_424_431_620_520_340_750);
    let names: Vec<&str> = ModsetType::ALL.iter().map(|t| t.java_name()).collect();
    assert_eq!(names, ["EVENT", "REPOSITORY"]);
    assert_eq!(a3s_types::local::PROFILE_TREE_ROOT_NAME, "racine");

    // `modset_type_desc` matches the descriptor `Builder::enum_value` creates.
    let mut b = Builder::new();
    let desc = wire::modset_type_desc(&mut b);
    let value = b.enum_value(wire::MODSET_TYPE, "EVENT");
    let id = value.as_ref().unwrap();
    let graph = b.graph();
    let a3s_jserial::Node::Enum(constant) = graph.node(id).unwrap() else {
        panic!("expected an enum constant");
    };
    assert_eq!(constant.class, desc);
}
