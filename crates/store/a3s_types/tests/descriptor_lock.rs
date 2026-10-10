//! Locks the Java class descriptors written to repositories.
//!
//! Every class in `ARCHITECTURE.md` ("Repository wire format") is listed here with its exact
//! name, serialVersionUID, flags and fields in wire order. Java clients reject files whose
//! descriptors differ, so this file must only change together with a deliberate, documented
//! wire-format decision. Agents are not allowed to edit it.

use a3s_jserial::jdk::Builder;
use a3s_jserial::{FieldType, NodeId, flags};
use a3s_types::wire;

/// (type code, field name, JVM signature for object fields).
type Field = (char, &'static str, Option<&'static str>);

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
const MAP: Option<&str> = Some("Ljava/util/Map;");

/// Creates a class descriptor in a builder.
type MakeDesc = fn(&mut Builder) -> NodeId;

#[expect(clippy::too_many_lines, reason = "one literal table of all wire classes")]
fn expected() -> Vec<(MakeDesc, Expected)> {
    vec![
        (
            wire::sync_tree_directory_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.SyncTreeDirectory",
                uid: -2_855_304_993_780_573_704,
                flags: S,
                fields: &[
                    ('Z', "deleted", None),
                    ('Z', "hidden", None),
                    ('Z', "markAsAddon", None),
                    ('Z', "updated", None),
                    ('L', "list", LIST),
                    ('L', "name", STR),
                    (
                        'L',
                        "parent",
                        Some("Lfr/soe/a3s/domain/repository/SyncTreeDirectory;"),
                    ),
                ],
                super_class: None,
            },
        ),
        (
            wire::sync_tree_leaf_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.SyncTreeLeaf",
                uid: 8_849_248_143_660_225_239,
                flags: S,
                fields: &[
                    ('Z', "compressed", None),
                    ('J', "compressedSize", None),
                    ('Z', "deleted", None),
                    ('J', "size", None),
                    ('Z', "updated", None),
                    ('L', "name", STR),
                    (
                        'L',
                        "parent",
                        Some("Lfr/soe/a3s/domain/repository/SyncTreeDirectory;"),
                    ),
                    ('L', "sha1", STR),
                ],
                super_class: None,
            },
        ),
        (
            wire::server_info_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.ServerInfo",
                uid: 7_697_232_677_958_952_953,
                flags: S,
                fields: &[
                    ('Z', "compressedPboFilesOnly", None),
                    ('Z', "noPartialFileTransfer", None),
                    ('I', "numberOfConnections", None),
                    ('J', "numberOfFiles", None),
                    ('Z', "repositoryContentUpdated", None),
                    ('I', "revision", None),
                    ('J', "totalFilesSize", None),
                    ('L', "buildDate", Some("Ljava/util/Date;")),
                    ('L', "hiddenFolderPaths", Some("Ljava/util/Set;")),
                ],
                super_class: None,
            },
        ),
        (
            wire::changelogs_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.Changelogs",
                uid: 7_332_983_755_625_818_660,
                flags: S,
                fields: &[('L', "list", LIST)],
                super_class: None,
            },
        ),
        (
            wire::changelog_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.Changelog",
                uid: 749_128_678_979_864_247,
                flags: S,
                fields: &[
                    ('Z', "contentUpdated", None),
                    ('I', "revision", None),
                    ('L', "addons", LIST),
                    ('L', "buildDate", Some("Ljava/util/Date;")),
                    ('L', "deletedAddons", LIST),
                    ('L', "newAddons", LIST),
                    ('L', "updatedAddons", LIST),
                ],
                super_class: None,
            },
        ),
        (
            wire::events_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.Events",
                uid: -5_141_643_688_299_352_462,
                flags: S,
                fields: &[('L', "list", LIST)],
                super_class: None,
            },
        ),
        (
            wire::event_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.Event",
                uid: 7_456_226_002_765_813_117,
                flags: S,
                fields: &[
                    ('L', "addonNames", MAP),
                    ('L', "description", STR),
                    ('L', "dlcNames", MAP),
                    ('L', "name", STR),
                    ('L', "userconfigFolderNames", MAP),
                ],
                super_class: None,
            },
        ),
        (
            wire::auto_config_desc,
            Expected {
                name: "fr.soe.a3s.domain.repository.AutoConfig",
                uid: 2_706_280_725_619_197_755,
                flags: S,
                fields: &[
                    ('L', "favoriteServers", LIST),
                    (
                        'L',
                        "protocole",
                        Some("Lfr/soe/a3s/domain/AbstractProtocole;"),
                    ),
                    ('L', "repositoryName", STR),
                ],
                super_class: None,
            },
        ),
        (
            wire::abstract_protocole_desc,
            Expected {
                name: "fr.soe.a3s.domain.AbstractProtocole",
                uid: -886_357_032_287_815_203,
                flags: S,
                fields: &[
                    ('Z', "validateSSLCertificate", None),
                    ('L', "connectionTimeOut", STR),
                    (
                        'L',
                        "encryptionMode",
                        Some("Lfr/soe/a3s/constant/EncryptionMode;"),
                    ),
                    ('L', "login", STR),
                    ('L', "password", STR),
                    ('L', "port", STR),
                    (
                        'L',
                        "protocolType",
                        Some("Lfr/soe/a3s/constant/ProtocolType;"),
                    ),
                    ('L', "readTimeOut", STR),
                    ('L', "url", STR),
                ],
                super_class: None,
            },
        ),
        (
            wire::favorite_server_desc,
            Expected {
                name: "fr.soe.a3s.domain.configration.FavoriteServer",
                uid: 4_613_236_292_596_702_132,
                flags: S,
                fields: &[
                    ('I', "port", None),
                    ('Z', "selected", None),
                    ('L', "ipAddress", STR),
                    ('L', "modsetName", STR),
                    ('L', "name", STR),
                    ('L', "password", STR),
                    ('L', "repositoryName", STR),
                ],
                super_class: None,
            },
        ),
        (
            |b| wire::protocol_class_desc(b, wire::HTTP),
            protocol_subclass("fr.soe.a3s.domain.Http", -20),
        ),
        (
            |b| wire::protocol_class_desc(b, wire::FTP),
            protocol_subclass("fr.soe.a3s.domain.Ftp", -1_917_448_655_850_030_044),
        ),
        (
            |b| wire::protocol_class_desc(b, wire::WEBDAV),
            protocol_subclass("fr.soe.a3s.domain.Webdav", -30),
        ),
        (
            |b| wire::protocol_class_desc(b, wire::SOCKS),
            protocol_subclass("fr.soe.a3s.domain.Socks", -40),
        ),
        (
            |b| wire::protocol_class_desc(b, wire::SFTP),
            protocol_subclass("fr.soe.a3s.domain.Sftp", 1),
        ),
    ]
}

fn protocol_subclass(name: &'static str, uid: i64) -> Expected {
    Expected {
        name,
        uid,
        flags: S,
        fields: &[],
        super_class: Some("fr.soe.a3s.domain.AbstractProtocole"),
    }
}

fn type_code(t: FieldType) -> char {
    t.code() as char
}

#[test]
fn class_descriptors_are_locked() {
    let all = expected();
    assert_eq!(all.len(), 15, "every wire class must be listed");
    for (make, want) in all {
        let mut b = Builder::new();
        let id = make(&mut b);
        let graph = b.graph();
        let desc = graph.class_desc(id).unwrap();
        assert_eq!(desc.name, want.name);
        assert_eq!(desc.serial_version_uid, want.uid, "{}", want.name);
        assert_eq!(desc.flags, want.flags, "{}", want.name);
        let fields: Vec<Field> = desc
            .fields
            .iter()
            .map(|f| {
                let sig = f.class_name.map(|s| graph.string(s).unwrap().to_owned());
                (
                    type_code(f.field_type),
                    // Leak is fine in a test: compare against &'static str tuples.
                    &*Box::leak(f.name.clone().into_boxed_str()),
                    sig.map(|s| &*Box::leak(s.into_boxed_str())),
                )
            })
            .collect();
        assert_eq!(fields, want.fields, "{}", want.name);
        let super_name = desc
            .super_class
            .map(|s| graph.class_desc(s).unwrap().name.clone());
        assert_eq!(super_name.as_deref(), want.super_class, "{}", want.name);
    }
}

#[test]
fn enum_constants_are_locked() {
    use a3s_types::{EncryptionMode, ProtocolType};
    let names: Vec<&str> = ProtocolType::ALL.iter().map(|p| p.java_name()).collect();
    assert_eq!(
        names,
        [
            "FTP",
            "FTPS",
            "SFTP",
            "HTTP",
            "HTTPS",
            "A3S",
            "SOCKS4",
            "SOCKS5",
            "HTTP_WEBDAV",
            "HTTPS_WEBDAV"
        ]
    );
    for mode in [
        EncryptionMode::NoEncryption,
        EncryptionMode::ExplicitSsl,
        EncryptionMode::ImplicitSsl,
    ] {
        assert_eq!(EncryptionMode::from_java_name(mode.java_name()), Some(mode));
    }
    assert_eq!(EncryptionMode::NoEncryption.java_name(), "NO_ENCRYPTION");
    assert_eq!(EncryptionMode::ExplicitSsl.java_name(), "EXPLICIT_SSL");
    assert_eq!(EncryptionMode::ImplicitSsl.java_name(), "IMPLICIT_SSL");
    assert_eq!(a3s_types::SYNC_ROOT_NAME, "racine");
}
