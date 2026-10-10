//! Java class descriptors of the Arma3Sync domain, and the [`JavaCodec`] trait.
//!
//! Field lists are in Java wire order: primitives sorted by name, then objects sorted by name.

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{FieldType, Graph, NodeId, Value, flags};

use crate::{Error, Result};

/// Conversion between a domain type and a Java object graph.
pub trait JavaCodec: Sized {
    /// Decodes `value` (an object reference in `graph`).
    fn decode(graph: &Graph, value: Value) -> Result<Self>;

    /// Encodes `self` into the builder, returning the reference to the new object.
    fn encode(&self, builder: &mut Builder) -> Value;
}

/// Decodes a complete (uncompressed) serialization stream holding one `T`.
pub fn decode_stream<T: JavaCodec>(bytes: &[u8]) -> Result<T> {
    let graph = a3s_jserial::read(bytes)?;
    T::decode(&graph, graph.root_value()?)
}

/// Encodes `value` as a complete (uncompressed) serialization stream.
pub fn encode_stream<T: JavaCodec>(value: &T) -> Result<Vec<u8>> {
    let mut builder = Builder::new();
    let root = value.encode(&mut builder);
    Ok(a3s_jserial::write(&builder.finish(root))?)
}

// ---------------------------------------------------------------------------------------------
// Signatures

pub const SIG_STRING: &str = "Ljava/lang/String;";
pub const SIG_LIST: &str = "Ljava/util/List;";
pub const SIG_SET: &str = "Ljava/util/Set;";
pub const SIG_MAP: &str = "Ljava/util/Map;";
pub const SIG_DATE: &str = "Ljava/util/Date;";
pub const SIG_SYNC_TREE_DIRECTORY: &str = "Lfr/soe/a3s/domain/repository/SyncTreeDirectory;";
pub const SIG_ABSTRACT_PROTOCOLE: &str = "Lfr/soe/a3s/domain/AbstractProtocole;";
pub const SIG_ENCRYPTION_MODE: &str = "Lfr/soe/a3s/constant/EncryptionMode;";
pub const SIG_PROTOCOL_TYPE: &str = "Lfr/soe/a3s/constant/ProtocolType;";

// ---------------------------------------------------------------------------------------------
// Class names

pub const SYNC_TREE_DIRECTORY: &str = "fr.soe.a3s.domain.repository.SyncTreeDirectory";
pub const SYNC_TREE_LEAF: &str = "fr.soe.a3s.domain.repository.SyncTreeLeaf";
pub const SERVER_INFO: &str = "fr.soe.a3s.domain.repository.ServerInfo";
pub const CHANGELOGS: &str = "fr.soe.a3s.domain.repository.Changelogs";
pub const CHANGELOG: &str = "fr.soe.a3s.domain.repository.Changelog";
pub const EVENTS: &str = "fr.soe.a3s.domain.repository.Events";
pub const EVENT: &str = "fr.soe.a3s.domain.repository.Event";
pub const AUTO_CONFIG: &str = "fr.soe.a3s.domain.repository.AutoConfig";
pub const ABSTRACT_PROTOCOLE: &str = "fr.soe.a3s.domain.AbstractProtocole";
pub const HTTP: &str = "fr.soe.a3s.domain.Http";
pub const FTP: &str = "fr.soe.a3s.domain.Ftp";
pub const WEBDAV: &str = "fr.soe.a3s.domain.Webdav";
pub const SOCKS: &str = "fr.soe.a3s.domain.Socks";
pub const SFTP: &str = "fr.soe.a3s.domain.Sftp";
pub const FAVORITE_SERVER: &str = "fr.soe.a3s.domain.configration.FavoriteServer";
pub const PROTOCOL_TYPE: &str = "fr.soe.a3s.constant.ProtocolType";
pub const ENCRYPTION_MODE: &str = "fr.soe.a3s.constant.EncryptionMode";

// ---------------------------------------------------------------------------------------------
// Descriptors

const S: u8 = flags::SERIALIZABLE;

pub fn sync_tree_directory_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        SYNC_TREE_DIRECTORY,
        -2_855_304_993_780_573_704,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "deleted"),
            FieldSpec::prim(FieldType::Boolean, "hidden"),
            FieldSpec::prim(FieldType::Boolean, "markAsAddon"),
            FieldSpec::prim(FieldType::Boolean, "updated"),
            FieldSpec::object("list", SIG_LIST),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("parent", SIG_SYNC_TREE_DIRECTORY),
        ],
        None,
    )
}

pub fn sync_tree_leaf_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        SYNC_TREE_LEAF,
        8_849_248_143_660_225_239,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "compressed"),
            FieldSpec::prim(FieldType::Long, "compressedSize"),
            FieldSpec::prim(FieldType::Boolean, "deleted"),
            FieldSpec::prim(FieldType::Long, "size"),
            FieldSpec::prim(FieldType::Boolean, "updated"),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("parent", SIG_SYNC_TREE_DIRECTORY),
            FieldSpec::object("sha1", SIG_STRING),
        ],
        None,
    )
}

pub fn server_info_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        SERVER_INFO,
        7_697_232_677_958_952_953,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "compressedPboFilesOnly"),
            FieldSpec::prim(FieldType::Boolean, "noPartialFileTransfer"),
            FieldSpec::prim(FieldType::Int, "numberOfConnections"),
            FieldSpec::prim(FieldType::Long, "numberOfFiles"),
            FieldSpec::prim(FieldType::Boolean, "repositoryContentUpdated"),
            FieldSpec::prim(FieldType::Int, "revision"),
            FieldSpec::prim(FieldType::Long, "totalFilesSize"),
            FieldSpec::object("buildDate", SIG_DATE),
            FieldSpec::object("hiddenFolderPaths", SIG_SET),
        ],
        None,
    )
}

pub fn changelogs_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        CHANGELOGS,
        7_332_983_755_625_818_660,
        S,
        &[FieldSpec::object("list", SIG_LIST)],
        None,
    )
}

pub fn changelog_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        CHANGELOG,
        749_128_678_979_864_247,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "contentUpdated"),
            FieldSpec::prim(FieldType::Int, "revision"),
            FieldSpec::object("addons", SIG_LIST),
            FieldSpec::object("buildDate", SIG_DATE),
            FieldSpec::object("deletedAddons", SIG_LIST),
            FieldSpec::object("newAddons", SIG_LIST),
            FieldSpec::object("updatedAddons", SIG_LIST),
        ],
        None,
    )
}

pub fn events_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        EVENTS,
        -5_141_643_688_299_352_462,
        S,
        &[FieldSpec::object("list", SIG_LIST)],
        None,
    )
}

pub fn event_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        EVENT,
        7_456_226_002_765_813_117,
        S,
        &[
            FieldSpec::object("addonNames", SIG_MAP),
            FieldSpec::object("description", SIG_STRING),
            FieldSpec::object("dlcNames", SIG_MAP),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("userconfigFolderNames", SIG_MAP),
        ],
        None,
    )
}

pub fn auto_config_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        AUTO_CONFIG,
        2_706_280_725_619_197_755,
        S,
        &[
            FieldSpec::object("favoriteServers", SIG_LIST),
            FieldSpec::object("protocole", SIG_ABSTRACT_PROTOCOLE),
            FieldSpec::object("repositoryName", SIG_STRING),
        ],
        None,
    )
}

pub fn abstract_protocole_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        ABSTRACT_PROTOCOLE,
        -886_357_032_287_815_203,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "validateSSLCertificate"),
            FieldSpec::object("connectionTimeOut", SIG_STRING),
            FieldSpec::object("encryptionMode", SIG_ENCRYPTION_MODE),
            FieldSpec::object("login", SIG_STRING),
            FieldSpec::object("password", SIG_STRING),
            FieldSpec::object("port", SIG_STRING),
            FieldSpec::object("protocolType", SIG_PROTOCOL_TYPE),
            FieldSpec::object("readTimeOut", SIG_STRING),
            FieldSpec::object("url", SIG_STRING),
        ],
        None,
    )
}

/// Descriptor of a concrete protocol class (`Http`, `Ftp`, …), which has no own fields.
pub fn protocol_class_desc(b: &mut Builder, class: &str) -> NodeId {
    let uid = match class {
        HTTP => -20,
        FTP => -1_917_448_655_850_030_044,
        WEBDAV => -30,
        SOCKS => -40,
        SFTP => 1,
        _ => unreachable!("unknown protocol class {class}"),
    };
    let base = abstract_protocole_desc(b);
    b.class_desc(class, uid, S, &[], Some(base))
}

pub fn favorite_server_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        FAVORITE_SERVER,
        4_613_236_292_596_702_132,
        S,
        &[
            FieldSpec::prim(FieldType::Int, "port"),
            FieldSpec::prim(FieldType::Boolean, "selected"),
            FieldSpec::object("ipAddress", SIG_STRING),
            FieldSpec::object("modsetName", SIG_STRING),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("password", SIG_STRING),
            FieldSpec::object("repositoryName", SIG_STRING),
        ],
        None,
    )
}

// ---------------------------------------------------------------------------------------------
// Decoding helpers

/// Returns the object id of `value`, checking it is an instance of `class`.
pub(crate) fn expect_object(graph: &Graph, value: Value, class: &'static str) -> Result<NodeId> {
    let id = value.as_ref().ok_or(Error::NullValue(class))?;
    let found = graph.class_name_of(id)?;
    if found == class {
        Ok(id)
    } else {
        Err(Error::UnexpectedClass {
            expected: class,
            found: found.to_owned(),
        })
    }
}

/// Decodes a nullable `List<String>`; null becomes empty.
pub(crate) fn string_list(graph: &Graph, value: Value) -> Result<Vec<String>> {
    let elements = a3s_jserial::jdk::list(graph, value)?.unwrap_or_default();
    elements
        .into_iter()
        .filter_map(|v| graph.opt_string(v).transpose())
        .map(|s| s.map(str::to_owned).map_err(Error::from))
        .collect()
}

/// Decodes a nullable list of `T`; null becomes empty, null elements are skipped.
pub(crate) fn object_list<T: JavaCodec>(graph: &Graph, value: Value) -> Result<Vec<T>> {
    let elements = a3s_jserial::jdk::list(graph, value)?.unwrap_or_default();
    elements
        .into_iter()
        .filter(|v| *v != Value::Null)
        .map(|v| T::decode(graph, v))
        .collect()
}

/// Encodes strings as a `java.util.ArrayList<String>`.
pub(crate) fn encode_string_list(b: &mut Builder, items: &[String]) -> Value {
    let values = items.iter().map(|s| b.string(s)).collect();
    b.array_list(values)
}
