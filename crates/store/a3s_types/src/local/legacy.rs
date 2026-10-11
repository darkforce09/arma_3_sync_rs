//! Lenient decoders for the Java client's local files.
//!
//! Input is an uncompressed (and, for repositories, decrypted) Java serialization stream of
//! `a3s.cfg`, `a3s.prefs`, `*.a3s.profile`, `*.a3s.repository` or `export.a3s.autoconfig`.
//! These files were written by many Java versions, so decoding is deliberately forgiving:
//!
//! * the stream is read with generous limits ([`a3s_jserial::read`]), not the current Java
//!   filter, because large SHA-1 caches exceed it and old clients wrote files before it existed;
//! * fields are looked up by name through the class hierarchy; UIDs, unknown classes and unknown
//!   fields are ignored;
//! * a **missing** field takes the Java zero value (`0`, `false`, `null`), not the field
//!   initializer: Java does not run initializers when deserializing these classes. Java's
//!   null-normalising getters are then applied;
//! * a field of an unexpected kind is treated as missing, and an unknown enum constant takes the
//!   getter default (or `None`); both record a warning. Java would fail the whole file here
//!   (a local-only difference from Java);
//! * null elements of lists and sets are skipped.
//!
//! Hard errors are limited to an unreadable stream, a wrong root class, and a null
//! `Repository.name`, `Repository.protocole` or `Profile.name`, and a profile, repository or
//! addon-group tree node that is referenced twice or cyclically (Java never writes those; decoding
//! them again would let a tiny hostile stream expand exponentially).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Display;

use a3s_jserial::{Graph, Node, NodeId, Value};

use super::wire as local_wire;
use super::{
    Acre2Options, AcreOptions, AutoConfigExport, BikeyExtractOptions, Configuration,
    ExternalApplication, FileAttributes, LauncherOptions, Preferences, Profile, Proxy, Repository,
    RptOptions, TfarOptions, TreeDirectory, TreeLeaf, TreeNode,
};
use crate::constants::{
    CheckRepositoriesFrequency, DEFAULT_PROFILE_NAME, GAME_VERSION_ARMA3, IconResize, LookAndFeel,
    MinimizationType, ModsetType, StartWithOs,
};
use crate::{EncryptionMode, Error, FavoriteServer, Protocol, ProtocolType, Result, wire};

/// Maximum nesting of addon groups accepted in a profile tree.
const MAX_TREE_DEPTH: usize = 1_000;

/// A decoded value plus the human-readable warnings collected while decoding it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy<T> {
    pub value: T,

    /// One entry per lenient fallback, naming the Java class and field.
    pub warnings: Vec<String>,
}

/// `a3s.cfg`: the configuration plus the deprecated fields that the first start of a new client
/// migrates into the active profile.
///
/// Java (`ui/main/MainPanel.java:538-569`) copies every search path into the active profile, and
/// copies only these launcher fields: `arma3ExePath`, `autoRestart`, `checkSignatures`,
/// `filePatching`, `noPause`, `showScriptErrors`, `windowMode`, `cpuCountSelection`,
/// `defaultWorld`, `enableHT`, `hugePages`, `exThreadsSelection`, `gameProfile`,
/// `mallocSelection`, `maxMemorySelection`, `nologs`, `noSplashScreen`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyConfiguration {
    pub configuration: Configuration,

    /// Deprecated `Configuration.addonSearchDirectoryPaths`; `None` when null or absent.
    pub addon_search_directory_paths: Option<Vec<String>>,

    /// Deprecated `Configuration.launcherOptions`; `None` when null or absent.
    pub launcher_options: Option<LauncherOptions>,
}

/// `*.a3s.repository`: the repository plus its SHA-1 caches.
///
/// Cache keys are absolute local paths. Entries with a null key or a null `sha1` are dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyRepository {
    pub repository: Repository,

    /// `mapFilesForSync`.
    pub files_for_sync: BTreeMap<String, FileAttributes>,

    /// `mapFilesForBuild`.
    pub files_for_build: BTreeMap<String, FileAttributes>,
}

/// A root type of a Java local file that can be decoded leniently.
pub trait LegacyDecode: Sized {
    /// Decodes the object `value` of `graph`, appending warnings.
    fn decode_legacy(graph: &Graph, value: Value, warnings: &mut Vec<String>) -> Result<Self>;
}

/// Decodes a complete (uncompressed, decrypted) serialization stream holding one `T`.
pub fn decode_legacy_stream<T: LegacyDecode>(bytes: &[u8]) -> Result<Legacy<T>> {
    let graph = a3s_jserial::read(bytes)?;
    let mut warnings = Vec::new();
    let value = T::decode_legacy(&graph, graph.root_value()?, &mut warnings)?;
    Ok(Legacy { value, warnings })
}

impl LegacyDecode for LegacyConfiguration {
    fn decode_legacy(graph: &Graph, value: Value, warnings: &mut Vec<String>) -> Result<Self> {
        let id = wire::expect_object(graph, value, local_wire::CONFIGURATION)?;
        Lenient::new(graph, warnings).configuration(id)
    }
}

impl LegacyDecode for Preferences {
    fn decode_legacy(graph: &Graph, value: Value, warnings: &mut Vec<String>) -> Result<Self> {
        let id = wire::expect_object(graph, value, local_wire::PREFERENCES)?;
        Lenient::new(graph, warnings).preferences(id)
    }
}

impl LegacyDecode for Profile {
    fn decode_legacy(graph: &Graph, value: Value, warnings: &mut Vec<String>) -> Result<Self> {
        let id = wire::expect_object(graph, value, local_wire::PROFILE)?;
        Lenient::new(graph, warnings).profile(id)
    }
}

impl LegacyDecode for LegacyRepository {
    fn decode_legacy(graph: &Graph, value: Value, warnings: &mut Vec<String>) -> Result<Self> {
        let id = wire::expect_object(graph, value, local_wire::REPOSITORY)?;
        Lenient::new(graph, warnings).repository(id)
    }
}

impl LegacyDecode for AutoConfigExport {
    fn decode_legacy(graph: &Graph, value: Value, warnings: &mut Vec<String>) -> Result<Self> {
        let id = wire::expect_object(graph, value, local_wire::AUTO_CONFIG_EXPORT)?;
        Lenient::new(graph, warnings).auto_config_export(id)
    }
}

/// Lenient field access over one graph, collecting warnings.
struct Lenient<'g, 'w> {
    graph: &'g Graph,
    warnings: &'w mut Vec<String>,

    /// Profiles, repositories and tree nodes decoded so far. Java-built files never share them,
    /// so a second reference (or a cycle) is rejected instead of being decoded again, which
    /// would let a small hostile stream expand exponentially.
    seen: HashSet<NodeId>,
}

impl<'g, 'w> Lenient<'g, 'w> {
    fn new(graph: &'g Graph, warnings: &'w mut Vec<String>) -> Self {
        Self {
            graph,
            warnings,
            seen: HashSet::new(),
        }
    }

    /// Marks `id` as decoded; a node reached twice is an error (see [`Self::seen`]).
    fn visit(&mut self, id: NodeId, what: &'static str) -> Result<()> {
        if self.seen.insert(id) {
            Ok(())
        } else {
            Err(a3s_jserial::Error::LimitExceeded(what).into())
        }
    }
    // -----------------------------------------------------------------------------------------
    // Primitive access

    fn warn(&mut self, id: NodeId, field: &str, what: impl Display) {
        let class = self.class_of(id).unwrap_or("?");
        self.warnings.push(format!("{class}.{field}: {what}"));
    }

    /// Class name of an object or enum constant; `None` for strings, arrays and bad handles.
    fn class_of(&self, id: NodeId) -> Option<&'g str> {
        match self.graph.node(id).ok()? {
            Node::Object(_) | Node::Enum(_) => self.graph.class_name_of(id).ok(),
            _ => None,
        }
    }

    /// The raw value of `field`, `None` if the class (hierarchy) has no such field.
    fn raw(&self, id: NodeId, field: &str) -> Result<Option<Value>> {
        Ok(self.graph.opt_field(id, field)?)
    }

    fn bool(&mut self, id: NodeId, field: &str) -> Result<bool> {
        Ok(match self.raw(id, field)? {
            None => false,
            Some(Value::Boolean(v)) => v,
            Some(other) => {
                self.warn(
                    id,
                    field,
                    format_args!("expected a boolean, found {other:?}"),
                );
                false
            }
        })
    }

    fn int(&mut self, id: NodeId, field: &str) -> Result<i32> {
        Ok(match self.raw(id, field)? {
            None => 0,
            Some(Value::Int(v)) => v,
            Some(other) => {
                self.warn(id, field, format_args!("expected an int, found {other:?}"));
                0
            }
        })
    }

    fn long(&mut self, id: NodeId, field: &str) -> Result<i64> {
        Ok(match self.raw(id, field)? {
            None => 0,
            Some(Value::Long(v)) => v,
            Some(other) => {
                self.warn(id, field, format_args!("expected a long, found {other:?}"));
                0
            }
        })
    }

    fn double(&mut self, id: NodeId, field: &str) -> Result<f64> {
        Ok(match self.raw(id, field)? {
            None => 0.0,
            Some(Value::Double(v)) => v,
            Some(other) => {
                self.warn(
                    id,
                    field,
                    format_args!("expected a double, found {other:?}"),
                );
                0.0
            }
        })
    }

    /// Interprets `value` as a nullable string; anything else warns and reads as null.
    fn string_value(&mut self, id: NodeId, field: &str, value: Value) -> Option<String> {
        match self.graph.opt_string(value) {
            Ok(s) => s.map(str::to_owned),
            Err(_err) => {
                self.warn(id, field, "expected a string");
                None
            }
        }
    }

    fn string(&mut self, id: NodeId, field: &str) -> Result<Option<String>> {
        Ok(match self.raw(id, field)? {
            None => None,
            Some(value) => self.string_value(id, field, value),
        })
    }

    /// A string field that the Rust type requires, with a warning when it is null.
    fn string_or(&mut self, id: NodeId, field: &str, default: &str) -> Result<String> {
        if let Some(s) = self.string(id, field)? {
            return Ok(s);
        }
        self.warn(id, field, format_args!("null, using {default:?}"));
        Ok(default.to_owned())
    }

    /// An enum field: `None` when null or missing; an unknown constant or a non-enum value warns
    /// and reads as `None`.
    fn java_enum<T: std::fmt::Debug>(
        &mut self,
        id: NodeId,
        field: &str,
        parse: fn(&str) -> Option<T>,
    ) -> Result<Option<T>> {
        let Some(value) = self.raw(id, field)? else {
            return Ok(None);
        };
        let name = match self.graph.enum_name(value) {
            Ok(name) => name,
            Err(_err) => {
                self.warn(id, field, "expected an enum constant");
                return Ok(None);
            }
        };
        let Some(name) = name else {
            return Ok(None);
        };
        let parsed = parse(name);
        if parsed.is_none() {
            self.warn(id, field, format_args!("unknown enum constant `{name}`"));
        }
        Ok(parsed)
    }

    /// An enum field with a non-null Rust default (Java getter normalisation).
    fn java_enum_or<T: std::fmt::Debug + Copy>(
        &mut self,
        id: NodeId,
        field: &str,
        parse: fn(&str) -> Option<T>,
        default: T,
    ) -> Result<T> {
        Ok(self.java_enum(id, field, parse)?.unwrap_or(default))
    }

    // -----------------------------------------------------------------------------------------
    // Objects and collections

    /// An object field whose class is one of `classes`. Null or missing gives `None`; any other
    /// value warns and gives `None`.
    fn object(&mut self, id: NodeId, field: &str, classes: &[&str]) -> Result<Option<NodeId>> {
        let Some(value) = self.raw(id, field)? else {
            return Ok(None);
        };
        Ok(self.object_value(id, field, value, classes))
    }

    fn object_value(
        &mut self,
        id: NodeId,
        field: &str,
        value: Value,
        classes: &[&str],
    ) -> Option<NodeId> {
        match value {
            Value::Null => None,
            Value::Ref(child) => match self.class_of(child) {
                Some(class) if classes.contains(&class) => Some(child),
                found => {
                    let found = found.unwrap_or("a non-object");
                    self.warn(id, field, format_args!("unexpected {found}, ignored"));
                    None
                }
            },
            other => {
                self.warn(
                    id,
                    field,
                    format_args!("expected an object, found {other:?}"),
                );
                None
            }
        }
    }

    /// Elements of a `List` field (null elements skipped); null or missing is empty.
    fn list(&mut self, id: NodeId, field: &str) -> Result<Vec<Value>> {
        let Some(value) = self.raw(id, field)? else {
            return Ok(Vec::new());
        };
        match a3s_jserial::jdk::list(self.graph, value) {
            Ok(elements) => Ok(non_null(elements.unwrap_or_default())),
            Err(err) => {
                self.warn(id, field, format_args!("not a readable list ({err})"));
                Ok(Vec::new())
            }
        }
    }

    /// Elements of a `Set` field (null elements skipped); null or missing is `None`.
    fn opt_set(&mut self, id: NodeId, field: &str) -> Result<Option<Vec<Value>>> {
        let Some(value) = self.raw(id, field)? else {
            return Ok(None);
        };
        match a3s_jserial::jdk::set(self.graph, value) {
            Ok(elements) => Ok(elements.map(non_null)),
            Err(err) => {
                self.warn(id, field, format_args!("not a readable set ({err})"));
                Ok(None)
            }
        }
    }

    /// Entries of a `Map` field; null or missing is empty.
    fn map(&mut self, id: NodeId, field: &str) -> Result<Vec<(Value, Value)>> {
        let Some(value) = self.raw(id, field)? else {
            return Ok(Vec::new());
        };
        match a3s_jserial::jdk::map(self.graph, value) {
            Ok(entries) => Ok(entries.unwrap_or_default()),
            Err(err) => {
                self.warn(id, field, format_args!("not a readable map ({err})"));
                Ok(Vec::new())
            }
        }
    }

    /// Converts collection elements to strings; non-strings warn and are skipped.
    fn strings(&mut self, id: NodeId, field: &str, elements: Vec<Value>) -> Vec<String> {
        elements
            .into_iter()
            .filter_map(|v| self.string_value(id, field, v))
            .collect()
    }

    fn string_list(&mut self, id: NodeId, field: &str) -> Result<Vec<String>> {
        let elements = self.list(id, field)?;
        Ok(self.strings(id, field, elements))
    }

    fn string_set(&mut self, id: NodeId, field: &str) -> Result<BTreeSet<String>> {
        let elements = self.opt_set(id, field)?.unwrap_or_default();
        Ok(self.strings(id, field, elements).into_iter().collect())
    }

    /// A `List` of objects of `class`, each decoded with `decode`; other elements warn.
    fn object_list<T>(
        &mut self,
        id: NodeId,
        field: &str,
        class: &str,
        decode: fn(&mut Self, NodeId) -> Result<T>,
    ) -> Result<Vec<T>> {
        let elements = self.list(id, field)?;
        let mut out = Vec::with_capacity(elements.len());
        for element in elements {
            if let Some(child) = self.object_value(id, field, element, &[class]) {
                out.push(decode(self, child)?);
            }
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------------------------
    // Protocols

    fn is_protocol(&self, id: NodeId) -> Result<bool> {
        let Ok(object) = self.graph.object(id) else {
            return Ok(false);
        };
        for class in self.graph.hierarchy(object.class)? {
            if self.graph.class_desc(class)?.name == wire::ABSTRACT_PROTOCOLE {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A nullable protocol field; anything that is not a decodable protocol warns.
    fn opt_protocol(&mut self, id: NodeId, field: &str) -> Result<Option<Protocol>> {
        match self.raw(id, field)? {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Ref(child)) if self.is_protocol(child)? => {
                let protocol = self.protocol(child)?;
                if protocol.is_none() {
                    self.warn(id, field, "protocol type unknown, ignored");
                }
                Ok(protocol)
            }
            Some(_) => {
                self.warn(id, field, "not an AbstractProtocole, ignored");
                Ok(None)
            }
        }
    }

    /// A protocol field the Rust type requires (`Repository.protocole`).
    fn required_protocol(&mut self, id: NodeId, field: &'static str) -> Result<Protocol> {
        let child = match self.raw(id, field)? {
            None | Some(Value::Null) => return Err(Error::NullValue(field)),
            Some(Value::Ref(child)) => child,
            Some(other) => {
                return Err(Error::UnexpectedClass {
                    expected: wire::ABSTRACT_PROTOCOLE,
                    found: format!("{other:?}"),
                });
            }
        };
        let protocol = if self.is_protocol(child)? {
            self.protocol(child)?
        } else {
            None
        };
        protocol.ok_or_else(|| Error::UnexpectedClass {
            expected: wire::ABSTRACT_PROTOCOLE,
            found: self.class_of(child).unwrap_or("a non-object").to_owned(),
        })
    }

    /// Decodes an `AbstractProtocole` subclass instance; `None` if its type can't be resolved.
    fn protocol(&mut self, id: NodeId) -> Result<Option<Protocol>> {
        let fallback = self.class_of(id).and_then(protocol_type_for_class);
        let protocol_type =
            match self.java_enum(id, "protocolType", ProtocolType::from_java_name)? {
                Some(t) => Some(t),
                None => fallback,
            };
        let Some(protocol_type) = protocol_type else {
            return Ok(None);
        };
        Ok(Some(Protocol {
            protocol_type,
            url: self.string(id, "url")?.unwrap_or_default(),
            port: self.string(id, "port")?.unwrap_or_default(),
            login: self.string(id, "login")?,
            password: self.string(id, "password")?,
            connection_timeout: self.string(id, "connectionTimeOut")?,
            read_timeout: self.string(id, "readTimeOut")?,
            validate_ssl_certificate: self.bool(id, "validateSSLCertificate")?,
            encryption_mode: self.java_enum(
                id,
                "encryptionMode",
                EncryptionMode::from_java_name,
            )?,
        }))
    }

    // -----------------------------------------------------------------------------------------
    // Configuration

    // Mirrors domain/configration/Configuration.java:16-147
    fn configuration(&mut self, id: NodeId) -> Result<LegacyConfiguration> {
        let mut configuration = Configuration {
            height: self.int(id, "height")?,
            width: self.int(id, "width")?,
            profile_name: self.string_or(id, "profileName", DEFAULT_PROFILE_NAME)?,
            game_version: self.string_or(id, "gameVersion", GAME_VERSION_ARMA3)?,
            server_name: self.string(id, "serverName")?,
            default_modset: self.string(id, "defaultModset")?,
            view_mode_tree: self.bool(id, "viewModeTree")?,
            favorite_servers: self.object_list(
                id,
                "favoriteServers",
                wire::FAVORITE_SERVER,
                Self::favorite_server,
            )?,
            external_applications: self.object_list(
                id,
                "externalApplications",
                local_wire::EXTERNAL_APPLICATION,
                Self::external_application,
            )?,
            acre_options: self
                .opt_child(
                    id,
                    "acreOptions",
                    local_wire::ACRE_OPTIONS,
                    Self::acre_options,
                )?
                .unwrap_or_default(),
            acre2_options: self
                .opt_child(
                    id,
                    "acre2Options",
                    local_wire::ACRE2_OPTIONS,
                    Self::acre2_options,
                )?
                .unwrap_or_default(),
            tfar_options: self
                .opt_child(
                    id,
                    "tfarOptions",
                    local_wire::TFAR_OPTIONS,
                    Self::tfar_options,
                )?
                .unwrap_or_default(),
            rpt_options: self
                .opt_child(id, "rptOptions", local_wire::RPT_OPTIONS, Self::rpt_options)?
                .unwrap_or_default(),
            bikey_extract_options: self
                .opt_child(
                    id,
                    "bikeyExtractOptions",
                    local_wire::BIKEY_EXTRACT_OPTIONS,
                    Self::bikey_extract_options,
                )?
                .unwrap_or_default(),
            proxy: self.proxy(id)?,
        };
        // Java sorts on every `getFavoriteServers()`, so this is the order it observes.
        configuration.sort_favorite_servers();
        let addon_search_directory_paths = self
            .opt_set(id, "addonSearchDirectoryPaths")?
            .map(|elements| self.strings(id, "addonSearchDirectoryPaths", elements));
        let launcher_options = self.opt_child(
            id,
            "launcherOptions",
            local_wire::LAUNCHER_OPTIONS,
            Self::launcher_options,
        )?;
        Ok(LegacyConfiguration {
            configuration,
            addon_search_directory_paths,
            launcher_options,
        })
    }

    /// Decodes an optional child object of `class`.
    fn opt_child<T>(
        &mut self,
        id: NodeId,
        field: &str,
        class: &str,
        decode: fn(&mut Self, NodeId) -> Result<T>,
    ) -> Result<Option<T>> {
        match self.object(id, field, &[class])? {
            Some(child) => decode(self, child).map(Some),
            None => Ok(None),
        }
    }

    // Mirrors domain/configration/FavoriteServer.java (fields; old files lack the last two)
    fn favorite_server(&mut self, id: NodeId) -> Result<FavoriteServer> {
        Ok(FavoriteServer {
            name: self.string(id, "name")?,
            ip_address: self.string(id, "ipAddress")?,
            port: self.int(id, "port")?,
            password: self.string(id, "password")?,
            selected: self.bool(id, "selected")?,
            modset_name: self.string(id, "modsetName")?,
            repository_name: self.string(id, "repositoryName")?,
        })
    }

    // Mirrors domain/configration/ExternalApplication.java:8-11
    fn external_application(&mut self, id: NodeId) -> Result<ExternalApplication> {
        Ok(ExternalApplication {
            name: self.string(id, "name")?,
            executable_path: self.string(id, "executablePath")?,
            parameters: self.string(id, "parameters")?,
            enable: self.bool(id, "enable")?,
        })
    }

    fn acre_options(&mut self, id: NodeId) -> Result<AcreOptions> {
        Ok(AcreOptions {
            ts3_path: self.string(id, "ts3Path")?,
            acre_plugin_path: self.string(id, "acrePluginPath")?,
            acre_userconfig_path: self.string(id, "acreUserconfigPath")?,
        })
    }

    fn acre2_options(&mut self, id: NodeId) -> Result<Acre2Options> {
        Ok(Acre2Options {
            ts3_path: self.string(id, "ts3Path")?,
            acre2_plugin_path: self.string(id, "acre2PluginPath")?,
        })
    }

    fn tfar_options(&mut self, id: NodeId) -> Result<TfarOptions> {
        Ok(TfarOptions {
            ts3_path: self.string(id, "ts3Path")?,
            tfar_plugin_path: self.string(id, "tfarPluginPath")?,
            tfar_userconfig_path: self.string(id, "tfarUserconfigPath")?,
        })
    }

    fn rpt_options(&mut self, id: NodeId) -> Result<RptOptions> {
        Ok(RptOptions {
            rpt_path: self.string(id, "rptPath")?,
        })
    }

    fn bikey_extract_options(&mut self, id: NodeId) -> Result<BikeyExtractOptions> {
        Ok(BikeyExtractOptions {
            source_directory_path: self.string(id, "sourceDirectoryPath")?,
            target_directory_path: self.string(id, "targetDirectoryPath")?,
        })
    }

    /// `Configuration.proxy`, accepting the current `configration.Proxy` and the old
    /// `domain.Proxy` (`proxyEnabled`).
    // Mirrors domain/configration/Proxy.java:9-11, domain/Proxy.java:7-9
    fn proxy(&mut self, id: NodeId) -> Result<Proxy> {
        let classes = [local_wire::PROXY, local_wire::LEGACY_PROXY];
        let Some(child) = self.object(id, "proxy", &classes)? else {
            return Ok(Proxy::default());
        };
        let enable_field = if self.class_of(child) == Some(local_wire::LEGACY_PROXY) {
            "proxyEnabled"
        } else {
            "enableProxy"
        };
        Ok(Proxy {
            enable_proxy: self.bool(child, enable_field)?,
            proxy_protocol: self.opt_protocol(child, "proxyProtocol")?,
        })
    }

    // Mirrors domain/configration/LauncherOptions.java:10-38
    fn launcher_options(&mut self, id: NodeId) -> Result<LauncherOptions> {
        Ok(LauncherOptions {
            game_profile: self.string(id, "gameProfile")?,
            show_script_errors: self.bool(id, "showScriptErrors")?,
            no_pause: self.bool(id, "noPause")?,
            file_patching: self.bool(id, "filePatching")?,
            window_mode: self.bool(id, "windowMode")?,
            check_signatures: self.bool(id, "checkSignatures")?,
            auto_restart: self.bool(id, "autoRestart")?,
            launch_arma3_as_administrator: self.bool(id, "launchArma3AsAdministrator")?,
            mission_file: self.bool(id, "missionFile")?,
            mission_file_path: self.string(id, "missionFilePath")?,
            max_memory_selection: self.string(id, "maxMemorySelection")?,
            cpu_count_selection: self.int(id, "cpuCountSelection")?,
            ex_threads_selection: self.string(id, "exThreadsSelection")?,
            malloc_selection: self.string(id, "mallocSelection")?,
            enable_ht: self.bool(id, "enableHT")?,
            huge_pages: self.bool(id, "hugePages")?,
            no_splash_screen: self.bool(id, "noSplashScreen")?,
            default_world: self.bool(id, "defaultWorld")?,
            nologs: self.bool(id, "nologs")?,
            arma3_exe_path: self.string(id, "arma3ExePath")?,
            steam_exe_path: self.string(id, "steamExePath")?,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Preferences

    /// Applies the null-normalising getters; null minimization types read as `TaskBar`.
    // Mirrors domain/Preferences.java:18-83
    fn preferences(&mut self, id: NodeId) -> Result<Preferences> {
        let prefer_github_updates = match self.raw(id, "preferGitHubUpdates")? {
            None => None,
            Some(value) => match a3s_jserial::jdk::boolean(self.graph, value) {
                Ok(v) => v,
                Err(_err) => {
                    self.warn(id, "preferGitHubUpdates", "expected a java.lang.Boolean");
                    None
                }
            },
        };
        Ok(Preferences {
            launch_panel_game_launch: self.java_enum_or(
                id,
                "launchPanelGameLaunch",
                MinimizationType::from_java_name,
                MinimizationType::TaskBar,
            )?,
            launch_panel_minimized: self.java_enum_or(
                id,
                "launchPanelMinimized",
                MinimizationType::from_java_name,
                MinimizationType::TaskBar,
            )?,
            look_and_feel: self.java_enum_or(
                id,
                "lookAndFeel",
                LookAndFeel::from_java_name,
                LookAndFeel::LafDefault,
            )?,
            // The getter normalises null to AUTO, not to the initializer NONE.
            icon_resize_size: self.java_enum_or(
                id,
                "iconResizeSize",
                IconResize::from_java_name,
                IconResize::Auto,
            )?,
            start_with_os: self.java_enum_or(
                id,
                "startWithOS",
                StartWithOs::from_java_name,
                StartWithOs::Disabled,
            )?,
            check_repositories_frequency: self.java_enum_or(
                id,
                "checkRepositoriesFrequency",
                CheckRepositoriesFrequency::from_java_name,
                CheckRepositoriesFrequency::Freq3,
            )?,
            prefer_github_updates: prefer_github_updates.unwrap_or(true),
        })
    }

    // -----------------------------------------------------------------------------------------
    // Profile

    // Mirrors domain/Profile.java:15-70
    fn profile(&mut self, id: NodeId) -> Result<Profile> {
        self.visit(id, "shared profile")?;
        let name = self.string(id, "name")?.ok_or(Error::NullValue("name"))?;
        let tree = if let Some(tree) = self.object(id, "tree", &[local_wire::TREE_DIRECTORY])? {
            self.tree_directory(tree, 0)?
        } else {
            self.warn(id, "tree", "null, using an empty tree");
            TreeDirectory::new_root()
        };
        Ok(Profile {
            name,
            tree,
            additional_parameters: self.string(id, "additionalParameters")?,
            addon_names_by_priority: self.string_list(id, "addonNamesByPriority")?,
            launcher_options: self
                .opt_child(
                    id,
                    "launcherOptions",
                    local_wire::LAUNCHER_OPTIONS,
                    Self::launcher_options,
                )?
                .unwrap_or_default(),
            addon_search_directories: self.string_list(id, "addonSearchDirectories")?,
            last_added_addon_search_directory: self.string(id, "lastAddedAddonSearchDirectory")?,
        })
    }

    /// Keeps the stream order of the children (no re-sorting).
    // Mirrors domain/TreeDirectory.java:16-24
    fn tree_directory(&mut self, id: NodeId, depth: usize) -> Result<TreeDirectory> {
        self.visit(id, "shared or cyclic profile tree node")?;
        if depth > MAX_TREE_DEPTH {
            return Err(a3s_jserial::Error::LimitExceeded("profile tree depth").into());
        }
        let mut children = Vec::new();
        for element in self.list(id, "list")? {
            let classes = [local_wire::TREE_DIRECTORY, local_wire::TREE_LEAF];
            let Some(child) = self.object_value(id, "list", element, &classes) else {
                continue;
            };
            if self.class_of(child) == Some(local_wire::TREE_DIRECTORY) {
                children.push(TreeNode::Directory(self.tree_directory(child, depth + 1)?));
            } else {
                children.push(TreeNode::Leaf(self.tree_leaf(child)?));
            }
        }
        Ok(TreeDirectory {
            name: self.string_or(id, "name", "")?,
            selected: self.bool(id, "selected")?,
            marked: self.bool(id, "marked")?,
            optional: self.bool(id, "optional")?,
            modset_type: self.java_enum(id, "modsetType", ModsetType::from_java_name)?,
            modset_repository_name: self.string(id, "modsetRepositoryName")?,
            children,
            updated: false,
        })
    }

    // Mirrors domain/TreeLeaf.java:11-19
    fn tree_leaf(&mut self, id: NodeId) -> Result<TreeLeaf> {
        self.visit(id, "shared or cyclic profile tree node")?;
        Ok(TreeLeaf {
            name: self.string_or(id, "name", "")?,
            selected: self.bool(id, "selected")?,
            optional: self.bool(id, "optional")?,
            workshop_id: self.string(id, "workshopId")?,
            dlc_app_id: self.string(id, "dlcAppId")?,
            missing: false,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Repository

    /// Connection counts stay raw (a missing field reads `0`; the getters return the defaults).
    // Mirrors domain/repository/Repository.java:30-89,171-258
    fn repository(&mut self, id: NodeId) -> Result<LegacyRepository> {
        self.visit(id, "shared repository")?;
        let name = self.string(id, "name")?.ok_or(Error::NullValue("name"))?;
        let protocol = self.required_protocol(id, "protocole")?;
        let mut events_download_location = BTreeMap::new();
        for (key, value) in self.map(id, "mapEventsDownloadLocation")? {
            let key = self.string_value(id, "mapEventsDownloadLocation", key);
            let value = self.string_value(id, "mapEventsDownloadLocation", value);
            if let (Some(key), Some(value)) = (key, value) {
                events_download_location.insert(key, value);
            }
        }
        let repository = Repository {
            name,
            protocol,
            bit_torrent_protocol: self.opt_protocol(id, "bitTorrentProtocole")?,
            revision: self.int(id, "revision")?,
            notify: self.bool(id, "notify")?,
            auto: self.bool(id, "auto")?,
            hidden_folder_paths: self.string_set(id, "hidddenFolderPaths")?,
            no_auto_discover: self.bool(id, "noAutoDiscover")?,
            exact_match: self.bool(id, "exactMatch")?,
            default_download_location: self.string(id, "defaultDownloadLocation")?,
            events_download_location,
            number_of_client_connections: self.int(id, "numberOfClientConnections")?,
            number_of_repository_check_connections: self
                .int(id, "numberOfRepositoryCheckConnections")?,
            maximum_client_download_speed: self.double(id, "maximumClientDownloadSpeed")?,
            upload_protocol: self.opt_protocol(id, "uploadProtocole")?,
            upload_compressed_pbo_files_only: self.bool(id, "uploadCompressedPboFilesOnly")?,
            parallel_upload_connections: self.int(id, "parallelUploadConnections")?,
            path: self.string(id, "path")?,
            number_of_connections: self.int(id, "numberOfConnections")?,
            compressed: self.bool(id, "compressed")?,
            no_partial_file_transfer: self.bool(id, "noPartialFileTransfer")?,
            auto_config_url: self.string(id, "autoConfigURL")?,
            favorite_servers_set_to_autoconfig: self.object_list(
                id,
                "favoriteServersSetToAutoconfig",
                wire::FAVORITE_SERVER,
                Self::favorite_server,
            )?,
            excluded_files_from_build: self.string_set(id, "excludedFilesFromBuild")?,
            excluded_folders_from_sync: self.string_set(id, "excludedFoldersFromSync")?,
        };
        Ok(LegacyRepository {
            repository,
            files_for_sync: self.file_cache(id, "mapFilesForSync")?,
            files_for_build: self.file_cache(id, "mapFilesForBuild")?,
        })
    }

    /// A `Map<String, FileAttributes>`; entries with a null key, value or `sha1` are dropped.
    fn file_cache(&mut self, id: NodeId, field: &str) -> Result<BTreeMap<String, FileAttributes>> {
        let mut cache = BTreeMap::new();
        for (key, value) in self.map(id, field)? {
            let Some(key) = self.string_value(id, field, key) else {
                continue;
            };
            let Some(entry) = self.object_value(id, field, value, &[local_wire::FILE_ATTRIBUTES])
            else {
                continue;
            };
            let Some(sha1) = self.string(entry, "sha1")? else {
                continue;
            };
            let last_modified = self.long(entry, "lastModified")?;
            cache.insert(
                key,
                FileAttributes {
                    sha1,
                    last_modified,
                },
            );
        }
        Ok(cache)
    }

    // -----------------------------------------------------------------------------------------
    // export.a3s.autoconfig

    /// Elements that fail a hard check (null profile or repository name, missing protocol) are
    /// skipped with a warning instead of failing the import.
    // Mirrors domain/AutoConfig.java:12-16, service/CommonService.java:84-108
    fn auto_config_export(&mut self, id: NodeId) -> Result<AutoConfigExport> {
        let profiles = self.object_list(id, "profiles", local_wire::PROFILE, |this, child| {
            this.skippable(child, "(AutoConfig.profiles element)", Self::profile)
        })?;
        let favorite_servers = self.object_list(
            id,
            "favoriteServers",
            wire::FAVORITE_SERVER,
            Self::favorite_server,
        )?;
        let repositories =
            self.object_list(id, "repositories", local_wire::REPOSITORY, |this, child| {
                this.skippable(child, "(AutoConfig.repositories element)", |this, child| {
                    this.repository(child).map(|r| r.repository)
                })
            })?;
        Ok(AutoConfigExport {
            profiles: profiles.into_iter().flatten().collect(),
            favorite_servers,
            repositories: repositories.into_iter().flatten().collect(),
        })
    }

    /// Runs `decode`, turning a hard element error into a warning and `None`.
    fn skippable<T>(
        &mut self,
        id: NodeId,
        field: &str,
        decode: impl FnOnce(&mut Self, NodeId) -> Result<T>,
    ) -> Result<Option<T>> {
        match decode(self, id) {
            Ok(value) => Ok(Some(value)),
            Err(err @ (Error::NullValue(_) | Error::UnexpectedClass { .. })) => {
                self.warn(id, field, format_args!("element skipped: {err}"));
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }
}

fn non_null(values: Vec<Value>) -> Vec<Value> {
    values.into_iter().filter(|v| *v != Value::Null).collect()
}

/// Protocol type implied by the concrete class when `protocolType` is null (Java getters).
// Mirrors domain/Ftp.java:21-27, Http.java:19-25, Sftp.java:18-24, Socks.java:18-24,
// Webdav.java:19-25
fn protocol_type_for_class(class: &str) -> Option<ProtocolType> {
    Some(match class {
        wire::FTP => ProtocolType::Ftp,
        wire::HTTP => ProtocolType::Http,
        wire::SFTP => ProtocolType::Sftp,
        wire::SOCKS => ProtocolType::Socks4,
        wire::WEBDAV => ProtocolType::HttpWebdav,
        _ => return None,
    })
}
