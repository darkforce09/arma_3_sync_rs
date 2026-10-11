//! Java class names and descriptors of the client's local files.
//!
//! Descriptor functions exist only for the classes written to `export.a3s.autoconfig`. Their
//! field lists are in Java wire order (primitives sorted by name, then objects sorted by name)
//! and are locked by `tests/local_descriptor_lock.rs`. **Never** change them.

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{FieldType, NodeId, flags};

use crate::wire::{SIG_ABSTRACT_PROTOCOLE, SIG_LIST, SIG_MAP, SIG_SET, SIG_STRING};

// ---------------------------------------------------------------------------------------------
// Class names

/// Root of `export.a3s.autoconfig` (not the repository's `.a3s/autoconfig` class).
pub const AUTO_CONFIG_EXPORT: &str = "fr.soe.a3s.domain.AutoConfig";
pub const PROFILE: &str = "fr.soe.a3s.domain.Profile";
pub const TREE_DIRECTORY: &str = "fr.soe.a3s.domain.TreeDirectory";
pub const TREE_LEAF: &str = "fr.soe.a3s.domain.TreeLeaf";
pub const PREFERENCES: &str = "fr.soe.a3s.domain.Preferences";
pub const REPOSITORY: &str = "fr.soe.a3s.domain.repository.Repository";
pub const FILE_ATTRIBUTES: &str = "fr.soe.a3s.domain.repository.FileAttributes";
pub const CONFIGURATION: &str = "fr.soe.a3s.domain.configration.Configuration";
pub const LAUNCHER_OPTIONS: &str = "fr.soe.a3s.domain.configration.LauncherOptions";
pub const EXTERNAL_APPLICATION: &str = "fr.soe.a3s.domain.configration.ExternalApplication";
pub const ACRE_OPTIONS: &str = "fr.soe.a3s.domain.configration.AcreOptions";
pub const ACRE2_OPTIONS: &str = "fr.soe.a3s.domain.configration.Acre2Options";
pub const TFAR_OPTIONS: &str = "fr.soe.a3s.domain.configration.TfarOptions";
pub const RPT_OPTIONS: &str = "fr.soe.a3s.domain.configration.RptOptions";
pub const BIKEY_EXTRACT_OPTIONS: &str = "fr.soe.a3s.domain.configration.BikeyExtractOptions";
pub const AIA_OPTIONS: &str = "fr.soe.a3s.domain.configration.AiAOptions";
pub const PROXY: &str = "fr.soe.a3s.domain.configration.Proxy";

/// The older proxy class (UID `-50`), unused by current Java code.
pub const LEGACY_PROXY: &str = "fr.soe.a3s.domain.Proxy";

pub const MODSET_TYPE: &str = "fr.soe.a3s.constant.ModsetType";
pub const MINIMIZATION_TYPE: &str = "fr.soe.a3s.constant.MinimizationType";
pub const LOOK_AND_FEEL: &str = "fr.soe.a3s.constant.LookAndFeel";
pub const ICON_RESIZE: &str = "fr.soe.a3s.constant.IconResize";
pub const START_WITH_OS: &str = "fr.soe.a3s.constant.StartWithOS";
pub const CHECK_REPOSITORIES_FREQUENCY: &str = "fr.soe.a3s.constant.CheckRepositoriesFrequency";

// ---------------------------------------------------------------------------------------------
// Signatures

pub const SIG_TREE_DIRECTORY: &str = "Lfr/soe/a3s/domain/TreeDirectory;";
pub const SIG_MODSET_TYPE: &str = "Lfr/soe/a3s/constant/ModsetType;";
pub const SIG_LAUNCHER_OPTIONS: &str = "Lfr/soe/a3s/domain/configration/LauncherOptions;";

// ---------------------------------------------------------------------------------------------
// serialVersionUIDs

/// `fr.soe.a3s.domain.AutoConfig` declares no `serialVersionUID`; this is the JVM-computed
/// default (Serialization Specification 4.6), confirmed by the JDK harness
/// (`tests/fixtures/java_descriptors.txt`).
pub const AUTO_CONFIG_EXPORT_UID: i64 = 6_424_431_620_520_340_750;

/// JVM-computed default UID of `FileAttributes` (decode only, informational).
pub const FILE_ATTRIBUTES_UID: i64 = -2_500_913_557_576_506_383;

/// JVM-computed default UID of `configration.Proxy` (decode only, informational).
pub const PROXY_UID: i64 = -6_891_013_124_542_313_138;

// ---------------------------------------------------------------------------------------------
// Descriptors

const S: u8 = flags::SERIALIZABLE;

/// `fr.soe.a3s.domain.AutoConfig`.
// Mirrors domain/AutoConfig.java:10-16
pub fn auto_config_export_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        AUTO_CONFIG_EXPORT,
        AUTO_CONFIG_EXPORT_UID,
        S,
        &[
            FieldSpec::object("favoriteServers", SIG_LIST),
            FieldSpec::object("profiles", SIG_LIST),
            FieldSpec::object("repositories", SIG_LIST),
        ],
        None,
    )
}

/// `fr.soe.a3s.domain.Profile`.
// Mirrors domain/Profile.java:14-21
pub fn profile_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        PROFILE,
        -4_395_099_790_449_721_021,
        S,
        &[
            FieldSpec::object("additionalParameters", SIG_STRING),
            FieldSpec::object("addonNamesByPriority", SIG_LIST),
            FieldSpec::object("addonSearchDirectories", SIG_LIST),
            FieldSpec::object("lastAddedAddonSearchDirectory", SIG_STRING),
            FieldSpec::object("launcherOptions", SIG_LAUNCHER_OPTIONS),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("tree", SIG_TREE_DIRECTORY),
        ],
        None,
    )
}

/// `fr.soe.a3s.domain.TreeDirectory` (`updated` is transient).
// Mirrors domain/TreeDirectory.java:15-24
pub fn tree_directory_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        TREE_DIRECTORY,
        -3_917_102_856_123_611_985,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "marked"),
            FieldSpec::prim(FieldType::Boolean, "optional"),
            FieldSpec::prim(FieldType::Boolean, "selected"),
            FieldSpec::object("list", SIG_LIST),
            FieldSpec::object("modsetRepositoryName", SIG_STRING),
            FieldSpec::object("modsetType", SIG_MODSET_TYPE),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("parent", SIG_TREE_DIRECTORY),
        ],
        None,
    )
}

/// `fr.soe.a3s.domain.TreeLeaf` (`missing` is transient).
// Mirrors domain/TreeLeaf.java:10-19
pub fn tree_leaf_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        TREE_LEAF,
        8_681_722_025_933_532_378,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "duplicate"),
            FieldSpec::prim(FieldType::Boolean, "optional"),
            FieldSpec::prim(FieldType::Boolean, "selected"),
            FieldSpec::object("dlcAppId", SIG_STRING),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("parent", SIG_TREE_DIRECTORY),
            FieldSpec::object("workshopId", SIG_STRING),
        ],
        None,
    )
}

/// `fr.soe.a3s.constant.ModsetType`, an enum (UID 0, super `java.lang.Enum`), exactly as
/// [`Builder::enum_value`] creates it.
// Mirrors constant/ModsetType.java:3-5
pub fn modset_type_desc(b: &mut Builder) -> NodeId {
    let enum_flags = flags::SERIALIZABLE | flags::ENUM;
    let base = b.class_desc(a3s_jserial::jdk::ENUM, 0, enum_flags, &[], None);
    b.class_desc(MODSET_TYPE, 0, enum_flags, &[], Some(base))
}

/// `fr.soe.a3s.domain.repository.Repository` (13 transient fields omitted).
// Mirrors domain/repository/Repository.java:27-84
pub fn repository_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        REPOSITORY,
        -8_142_021_113_361_619_970,
        S,
        &[
            FieldSpec::prim(FieldType::Boolean, "auto"),
            FieldSpec::prim(FieldType::Boolean, "compressed"),
            FieldSpec::prim(FieldType::Boolean, "exactMatch"),
            FieldSpec::prim(FieldType::Double, "maximumClientDownloadSpeed"),
            FieldSpec::prim(FieldType::Boolean, "noAutoDiscover"),
            FieldSpec::prim(FieldType::Boolean, "noPartialFileTransfer"),
            FieldSpec::prim(FieldType::Boolean, "notify"),
            FieldSpec::prim(FieldType::Int, "numberOfClientConnections"),
            FieldSpec::prim(FieldType::Int, "numberOfConnections"),
            FieldSpec::prim(FieldType::Int, "numberOfRepositoryCheckConnections"),
            FieldSpec::prim(FieldType::Boolean, "outOfSynk"),
            FieldSpec::prim(FieldType::Int, "parallelUploadConnections"),
            FieldSpec::prim(FieldType::Int, "revision"),
            FieldSpec::prim(FieldType::Boolean, "uploadCompressedPboFilesOnly"),
            FieldSpec::object("autoConfigURL", SIG_STRING),
            FieldSpec::object("bitTorrentProtocole", SIG_ABSTRACT_PROTOCOLE),
            FieldSpec::object("defaultDownloadLocation", SIG_STRING),
            FieldSpec::object("excludedFilesFromBuild", SIG_SET),
            FieldSpec::object("excludedFoldersFromSync", SIG_SET),
            FieldSpec::object("favoriteServersSetToAutoconfig", SIG_LIST),
            FieldSpec::object("hidddenFolderPaths", SIG_SET),
            FieldSpec::object("mapEventsDownloadLocation", SIG_MAP),
            FieldSpec::object("mapFilesForBuild", SIG_MAP),
            FieldSpec::object("mapFilesForSync", SIG_MAP),
            FieldSpec::object("name", SIG_STRING),
            FieldSpec::object("path", SIG_STRING),
            FieldSpec::object("protocole", SIG_ABSTRACT_PROTOCOLE),
            FieldSpec::object("uploadProtocole", SIG_ABSTRACT_PROTOCOLE),
        ],
        None,
    )
}
