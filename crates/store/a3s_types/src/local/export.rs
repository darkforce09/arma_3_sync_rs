use a3s_jserial::jdk::Builder;
use a3s_jserial::{ClassData, Graph, NodeId, Value};
use serde::{Deserialize, Serialize};

use super::legacy::LegacyDecode as _;
use super::wire as local_wire;
use super::{PROFILE_TREE_ROOT_NAME, Profile, Repository, TreeDirectory, TreeLeaf, TreeNode};
use crate::wire::JavaCodec;
use crate::{FavoriteServer, Protocol, Result};

/// Content of `export.a3s.autoconfig` (`fr.soe.a3s.domain.AutoConfig`): profiles, favorite
/// servers and repositories exchanged between clients.
///
/// Not to be confused with a repository's `.a3s/autoconfig` ([`crate::AutoConfig`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoConfigExport {
    pub profiles: Vec<Profile>,
    pub favorite_servers: Vec<FavoriteServer>,
    pub repositories: Vec<Repository>,
}

impl JavaCodec for AutoConfigExport {
    /// Lenient decode ([`super::legacy`]); warnings are dropped. Use
    /// [`super::decode_legacy_stream`] to get them.
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        Self::decode_legacy(graph, value, &mut Vec::new())
    }

    /// Encodes what Java's export writes:
    ///
    /// * every profile with `launcherOptions = null`;
    /// * every repository as `new Repository(name, protocol)`, i.e. all other fields at their
    ///   initializer values, whatever `self` holds;
    /// * favorite servers as they are.
    // Mirrors service/CommonService.java:54-81
    fn encode(&self, b: &mut Builder) -> Value {
        let class = local_wire::auto_config_export_desc(b);
        // Wire order: favoriteServers, profiles, repositories.
        let servers = self.favorite_servers.iter().map(|s| s.encode(b)).collect();
        let servers = b.array_list(servers);
        let profiles = self
            .profiles
            .iter()
            .map(|p| encode_profile_for_export(b, p))
            .collect();
        let profiles = b.array_list(profiles);
        let repositories = self
            .repositories
            .iter()
            .map(|r| encode_fresh_repository(b, &r.name, &r.protocol))
            .collect();
        let repositories = b.array_list(repositories);
        b.simple_object(class, vec![servers, profiles, repositories])
    }
}

/// A `Profile` with its launcher options cleared (`profile.setLauncherOptions(null)`).
// Mirrors service/CommonService.java:59-64
fn encode_profile_for_export(b: &mut Builder, profile: &Profile) -> Value {
    let class = local_wire::profile_desc(b);
    // Wire order: additionalParameters, addonNamesByPriority, addonSearchDirectories,
    // lastAddedAddonSearchDirectory, launcherOptions, name, tree.
    let additional = b.opt_string(profile.additional_parameters.as_deref());
    let priorities = string_array_list(b, &profile.addon_names_by_priority);
    let directories = string_array_list(b, &profile.addon_search_directories);
    let last_added = b.opt_string(profile.last_added_addon_search_directory.as_deref());
    let name = b.string(&profile.name);
    let tree = encode_tree_directory(b, &profile.tree, None);
    b.simple_object(
        class,
        vec![
            additional,
            priorities,
            directories,
            last_added,
            Value::Null,
            name,
            tree,
        ],
    )
}

fn string_array_list(b: &mut Builder, strings: &[String]) -> Value {
    let elements = strings.iter().map(|s| b.string(s)).collect();
    b.array_list(elements)
}

/// A `TreeDirectory` whose children point back to it through `parent`.
// Mirrors domain/TreeDirectory.java:16-29
fn encode_tree_directory(b: &mut Builder, dir: &TreeDirectory, parent: Option<NodeId>) -> Value {
    let class = local_wire::tree_directory_desc(b);
    let id = b.reserve_object(class);
    let children = dir
        .children
        .iter()
        .map(|child| match child {
            TreeNode::Directory(d) => encode_tree_directory(b, d, Some(id)),
            TreeNode::Leaf(l) => encode_tree_leaf(b, l, id),
        })
        .collect();
    let list = b.array_list(children);
    let modset_repository_name = b.opt_string(dir.modset_repository_name.as_deref());
    let modset_type = dir.modset_type.map_or(Value::Null, |t| {
        b.enum_value(local_wire::MODSET_TYPE, t.java_name())
    });
    let name = if parent.is_none() && dir.name == PROFILE_TREE_ROOT_NAME {
        // A string literal in Java (`new TreeDirectory("racine", null)`).
        b.interned(PROFILE_TREE_ROOT_NAME)
    } else {
        b.string(&dir.name)
    };
    b.set_object_data(
        id,
        vec![ClassData {
            values: vec![
                Value::Boolean(dir.marked),
                Value::Boolean(dir.optional),
                Value::Boolean(dir.selected),
                list,
                modset_repository_name,
                modset_type,
                name,
                parent.map_or(Value::Null, Value::Ref),
            ],
            annotations: Vec::new(),
        }],
    );
    Value::Ref(id)
}

/// A `TreeLeaf`; the deprecated final `duplicate` is always `false`.
// Mirrors domain/TreeLeaf.java:11-19
fn encode_tree_leaf(b: &mut Builder, leaf: &TreeLeaf, parent: NodeId) -> Value {
    let class = local_wire::tree_leaf_desc(b);
    let dlc_app_id = b.opt_string(leaf.dlc_app_id.as_deref());
    let name = b.string(&leaf.name);
    let workshop_id = b.opt_string(leaf.workshop_id.as_deref());
    b.simple_object(
        class,
        vec![
            Value::Boolean(false),
            Value::Boolean(leaf.optional),
            Value::Boolean(leaf.selected),
            dlc_app_id,
            name,
            Value::Ref(parent),
            workshop_id,
        ],
    )
}

/// `new Repository(name, protocol)`: every other field at its initializer value, with fresh,
/// empty `HashSet`, `HashMap` and `ArrayList` instances.
// Mirrors service/CommonService.java:76, domain/repository/Repository.java:30-89
fn encode_fresh_repository(b: &mut Builder, name: &str, protocol: &Protocol) -> Value {
    let fresh = Repository::new(name, protocol.clone());
    let class = local_wire::repository_desc(b);
    let excluded_files = b.hash_set_str(&[]);
    let excluded_folders = b.hash_set_str(&[]);
    let favorite_servers = b.array_list(Vec::new());
    let hidden_folders = b.hash_set_str(&[]);
    let events_download_location = b.hash_map_str(&[]);
    let files_for_build = b.hash_map_str(&[]);
    let files_for_sync = b.hash_map_str(&[]);
    let name = b.string(&fresh.name);
    let protocol = fresh.protocol.encode(b);
    b.simple_object(
        class,
        vec![
            Value::Boolean(fresh.auto),
            Value::Boolean(fresh.compressed),
            Value::Boolean(fresh.exact_match),
            Value::Double(fresh.maximum_client_download_speed),
            Value::Boolean(fresh.no_auto_discover),
            Value::Boolean(fresh.no_partial_file_transfer),
            Value::Boolean(fresh.notify),
            Value::Int(fresh.number_of_client_connections),
            Value::Int(fresh.number_of_connections),
            Value::Int(fresh.number_of_repository_check_connections),
            // `outOfSynk`: deprecated, final, always false.
            Value::Boolean(false),
            Value::Int(fresh.parallel_upload_connections),
            Value::Int(fresh.revision),
            Value::Boolean(fresh.upload_compressed_pbo_files_only),
            Value::Null, // autoConfigURL
            Value::Null, // bitTorrentProtocole
            Value::Null, // defaultDownloadLocation
            excluded_files,
            excluded_folders,
            favorite_servers,
            hidden_folders,
            events_download_location,
            files_for_build,
            files_for_sync,
            name,
            Value::Null, // path
            protocol,
            Value::Null, // uploadProtocole
        ],
    )
}
