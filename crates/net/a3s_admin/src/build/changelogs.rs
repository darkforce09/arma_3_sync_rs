//! `.a3s/changelogs`: the build history (B28–B33, B44–B46).

use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, NodeId, Value};
use a3s_types::{Changelogs, JavaCodec, JavaDate, SyncTreeDirectory, SyncTreeLeaf, wire};

use super::carry::{Memo, OldDate, OldList, OldStr, RawGraph};
use super::java_hash::JavaHashMap;

/// An addon directory of a sync tree, in `getAddonsByName` order.
pub(crate) struct AddonDir<'t> {
    pub name: &'t str,

    /// `/`-joined path below `racine`.
    pub relpath: String,

    /// Directly below `racine`: its relative path is its name (the same `String` instance).
    pub top_level: bool,
    pub dir: &'t SyncTreeDirectory,
}

/// Addon directories: a depth-first search over directories in list order; an addon is not
/// searched further, so nested addons below it are not listed.
// Mirrors dao/repository/RepositoryBuildProcessor.java:394-421
pub(crate) fn addon_dirs(sync: &SyncTreeDirectory) -> Vec<AddonDir<'_>> {
    fn rec<'t>(dir: &'t SyncTreeDirectory, prefix: &str, out: &mut Vec<AddonDir<'t>>) {
        for child in dir.directories() {
            let relpath = if prefix.is_empty() {
                child.name.clone()
            } else {
                format!("{prefix}/{}", child.name)
            };
            if child.mark_as_addon {
                out.push(AddonDir {
                    name: &child.name,
                    relpath,
                    top_level: prefix.is_empty(),
                    dir: child,
                });
            } else {
                rec(child, &relpath, out);
            }
        }
    }
    let mut out = Vec::new();
    rec(sync, "", &mut out);
    out
}

/// Addon names (`getAddonsByName`): bare directory names, duplicates possible.
pub(crate) fn addon_names(sync: &SyncTreeDirectory) -> Vec<String> {
    addon_dirs(sync)
        .into_iter()
        .map(|a| a.name.to_owned())
        .collect()
}

/// Leaves of a subtree in deep-search order (`getDeepSearchLeafsList`).
pub(crate) fn deep_leaves(dir: &SyncTreeDirectory) -> Vec<&SyncTreeLeaf> {
    let mut out = Vec::new();
    dir.walk(&mut |_, node| {
        if let a3s_types::SyncTreeNode::Leaf(leaf) = node {
            out.push(leaf);
        }
    });
    out
}

/// A changelog of the previous file, carried over as is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OldChangelog {
    id: NodeId,
    content_updated: bool,
    revision: i32,
    addons: OldList,
    build_date: OldDate,
    deleted_addons: OldList,
    new_addons: OldList,
    updated_addons: OldList,
}

impl OldChangelog {
    fn resolve(graph: &Graph, id: NodeId) -> a3s_jserial::Result<Self> {
        a3s_jserial::jdk::expect_class(graph, id, wire::CHANGELOG)?;
        let object = |name| -> a3s_jserial::Result<Value> {
            Ok(graph.opt_field(id, name)?.unwrap_or(Value::Null))
        };
        let list = |name| OldList::resolve(graph, object(name)?);
        Ok(Self {
            id,
            content_updated: match graph.opt_field(id, "contentUpdated")? {
                Some(Value::Boolean(b)) => b,
                _ => false,
            },
            revision: match graph.opt_field(id, "revision")? {
                Some(Value::Int(r)) => r,
                _ => 0,
            },
            addons: list("addons")?,
            build_date: OldDate::resolve(graph, object("buildDate")?)?,
            deleted_addons: list("deletedAddons")?,
            new_addons: list("newAddons")?,
            updated_addons: list("updatedAddons")?,
        })
    }

    /// Re-emits the changelog with the current class descriptor.
    fn encode(&self, b: &mut Builder, memo: &mut Memo) -> Value {
        if let Some(v) = memo.object(self.id) {
            return v;
        }
        let class = wire::changelog_desc(b);
        let id = b.reserve_object(class);
        memo.set_object(self.id, Value::Ref(id));
        let values = vec![
            Value::Boolean(self.content_updated),
            Value::Int(self.revision),
            memo.list(b, &self.addons),
            memo.date(b, self.build_date),
            memo.list(b, &self.deleted_addons),
            memo.list(b, &self.new_addons),
            memo.list(b, &self.updated_addons),
        ];
        b.set_object_data(
            id,
            vec![a3s_jserial::ClassData {
                values,
                annotations: Vec::new(),
            }],
        );
        Value::Ref(id)
    }
}

/// An `updatedAddons` entry: the lowercase relative path used as `HashMap` key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdatedAddon {
    /// `toLowerCase` returned the top-level addon's own name: the same instance as `addons[i]`.
    Addon(usize),

    /// A new string.
    Text(String),
}

/// The changelog written by this build.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NewChangelog {
    revision: i32,
    build_date: JavaDate,
    content_updated: bool,
    addons: Vec<String>,

    /// Indices into `addons` (the same instances).
    new_addons: Vec<usize>,

    /// Entries of the previous changelog's `addons` (the same instances).
    deleted_addons: Vec<OldStr>,
    updated_addons: Vec<UpdatedAddon>,
}

impl NewChangelog {
    fn encode(&self, b: &mut Builder, memo: &mut Memo) -> Value {
        let class = wire::changelog_desc(b);
        // Every name is a new `String` (`File.getName()`), even for duplicate names.
        let addons: Vec<Value> = self.addons.iter().map(|a| b.string(a)).collect();
        let addon_list = b.array_list(addons.clone());
        let build_date = b.date(self.build_date);
        let deleted = self
            .deleted_addons
            .iter()
            .map(|s| memo.string(b, s))
            .collect();
        let deleted = b.array_list(deleted);
        let new = self.new_addons.iter().map(|&i| addons[i]).collect();
        let new = b.array_list(new);
        let updated = self
            .updated_addons
            .iter()
            .map(|u| match u {
                UpdatedAddon::Addon(i) => addons[*i],
                UpdatedAddon::Text(s) => b.string(s),
            })
            .collect();
        let updated = b.array_list(updated);
        b.simple_object(
            class,
            vec![
                Value::Boolean(self.content_updated),
                Value::Int(self.revision),
                addon_list,
                build_date,
                deleted,
                new,
                updated,
            ],
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Null,
    Old(OldChangelog),
    New(NewChangelog),
}

/// The `Changelogs` object written by a build. Write-only: its `decode` always fails; read the
/// written file as [`Changelogs`].
#[derive(Clone, Debug, PartialEq)]
pub struct ChangelogsEmit {
    entries: Vec<Entry>,
}

impl JavaCodec for ChangelogsEmit {
    fn decode(_graph: &Graph, _value: Value) -> a3s_types::Result<Self> {
        Err(a3s_jserial::Error::Unsupported("build output is write-only").into())
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::changelogs_desc(b);
        let mut memo = Memo::default();
        let items = self
            .entries
            .iter()
            .map(|e| match e {
                Entry::Null => Value::Null,
                Entry::Old(old) => old.encode(b, &mut memo),
                Entry::New(new) => new.encode(b, &mut memo),
            })
            .collect();
        let list = b.array_list(items);
        b.simple_object(class, vec![list])
    }
}

/// Resolves the old changelog list (null list or no file: empty).
fn old_entries(old: Option<&RawGraph>) -> a3s_jserial::Result<Vec<Option<OldChangelog>>> {
    let Some(old) = old else {
        return Ok(Vec::new());
    };
    let graph = &old.graph;
    let root = old
        .root
        .as_ref()
        .ok_or(a3s_jserial::Error::UnexpectedNull)?;
    a3s_jserial::jdk::expect_class(graph, root, wire::CHANGELOGS)?;
    let elements = a3s_jserial::jdk::list(graph, graph.field(root, "list")?)?.unwrap_or_default();
    elements
        .into_iter()
        .map(|e| match e {
            Value::Ref(id) => OldChangelog::resolve(graph, id).map(Some),
            _ => Ok(None),
        })
        .collect()
}

/// Computes the new changelog list (B29–B33).
///
/// `old` is the previous `changelogs` file, `old_sync` the previous `sync`.
// Mirrors dao/repository/RepositoryBuildProcessor.java:178-243
pub fn plan_changelogs(
    old: Option<&RawGraph>,
    old_sync: Option<&SyncTreeDirectory>,
    sync: &SyncTreeDirectory,
    revision: i32,
    build_date: JavaDate,
    content_updated: bool,
) -> a3s_jserial::Result<ChangelogsEmit> {
    let old_list = old_entries(old)?;
    let carried = |list: Vec<Option<OldChangelog>>| -> Vec<Entry> {
        list.into_iter()
            .map(|e| e.map_or(Entry::Null, Entry::Old))
            .collect()
    };
    let dirs = addon_dirs(sync);
    let addons: Vec<String> = dirs.iter().map(|a| a.name.to_owned()).collect();

    if old_list.is_empty() {
        // Case A: first changelog (also when content did not change).
        let new_addons = if revision == 1 {
            (0..addons.len()).collect()
        } else {
            Vec::new()
        };
        let c = NewChangelog {
            revision,
            build_date,
            content_updated,
            addons,
            new_addons,
            deleted_addons: Vec::new(),
            updated_addons: Vec::new(),
        };
        return Ok(ChangelogsEmit {
            entries: vec![Entry::New(c)],
        });
    }
    if !content_updated {
        // Case C: the old list, unchanged.
        return Ok(ChangelogsEmit {
            entries: carried(old_list),
        });
    }

    // Case B: append, then drop only the first entry if there are more than 10.
    let mut entries = carried(old_list);
    entries.push(Entry::Null); // placeholder for the new changelog
    if entries.len() > Changelogs::MAX_ENTRIES {
        entries.remove(0);
    }
    let previous: &[OldStr] = match &entries[entries.len() - 2] {
        Entry::Old(prev) => prev.addons.elements(),
        Entry::Null | Entry::New(_) => &[],
    };
    let new_addons = addons
        .iter()
        .enumerate()
        .filter(|(_, a)| !previous.iter().any(|p| p.as_str() == Some(a.as_str())))
        .map(|(i, _)| i)
        .collect();
    let deleted_addons = previous
        .iter()
        .filter(|p| !addons.iter().any(|a| p.as_str() == Some(a.as_str())))
        .cloned()
        .collect();
    let updated_addons = match old_sync {
        Some(old_sync) => updated_addons(&dirs, old_sync),
        None => Vec::new(),
    };
    let last = entries.len() - 1;
    entries[last] = Entry::New(NewChangelog {
        revision,
        build_date,
        content_updated,
        addons,
        new_addons,
        deleted_addons,
        updated_addons,
    });
    Ok(ChangelogsEmit { entries })
}

/// Key of an addon in `getAddons`'s map: its lowercase relative path.
// Mirrors dao/repository/RepositoryBuildProcessor.java:459-466
fn addon_key(addon: &AddonDir<'_>) -> String {
    addon.relpath.to_lowercase()
}

/// Deep leaf list stably sorted by `compareToIgnoreCase` of the names (`Collections.sort`).
fn sorted_leaves(dir: &SyncTreeDirectory) -> Vec<&SyncTreeLeaf> {
    let mut leaves = deep_leaves(dir);
    leaves.sort_by(|a, b| a3s_types::java_string::compare_ignore_case(&a.name, &b.name));
    leaves
}

/// Addons present in both trees whose files changed (B31), in `HashMap` iteration order.
// Mirrors dao/repository/RepositoryBuildProcessor.java:214-240
fn updated_addons(dirs: &[AddonDir<'_>], old_sync: &SyncTreeDirectory) -> Vec<UpdatedAddon> {
    let mut old_map: JavaHashMap<(), &SyncTreeDirectory> = JavaHashMap::new();
    for addon in addon_dirs(old_sync) {
        old_map.put(addon_key(&addon), (), addon.dir);
    }
    let mut map: JavaHashMap<UpdatedAddon, &SyncTreeDirectory> = JavaHashMap::new();
    for (i, addon) in dirs.iter().enumerate() {
        let key = addon_key(addon);
        // `toLowerCase` returns the string itself when nothing changes, and a top-level addon's
        // relative path is its name instance, which is also `addons[i]`.
        let payload = if addon.top_level && key == addon.name {
            UpdatedAddon::Addon(i)
        } else {
            UpdatedAddon::Text(key.clone())
        };
        map.put(key, payload, addon.dir);
    }
    let mut updated = Vec::new();
    for entry in map.iter() {
        let Some(old) = old_map.get(&entry.key) else {
            continue;
        };
        let new_leaves = sorted_leaves(entry.value);
        let old_leaves = sorted_leaves(old.value);
        // Java's `getSha1().equals` would throw on a null new SHA-1 (file deleted mid-build);
        // comparing `Option`s instead.
        let changed = new_leaves.len() != old_leaves.len()
            || new_leaves
                .iter()
                .zip(&old_leaves)
                .any(|(n, o)| n.sha1 != o.sha1);
        if changed {
            updated.push(entry.key_payload.clone());
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::addon_names;
    use a3s_types::{SyncTreeDirectory, SyncTreeNode};

    fn dir(name: &str, addon: bool, children: Vec<SyncTreeNode>) -> SyncTreeNode {
        SyncTreeNode::Directory(SyncTreeDirectory {
            name: name.to_owned(),
            mark_as_addon: addon,
            children,
            ..Default::default()
        })
    }

    #[test]
    fn nested_addons_below_an_addon_are_not_listed() {
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![
            dir("@a", true, vec![dir("@inner", true, vec![])]),
            dir(
                "group",
                false,
                vec![dir("@b", true, vec![]), dir("@a", true, vec![])],
            ),
        ];
        assert_eq!(addon_names(&root), ["@a", "@b", "@a"]);
    }
}
