use std::collections::HashSet;

use a3s_jserial::jdk::Builder;
use a3s_jserial::{ClassData, Graph, NodeId, Value};
use serde::{Deserialize, Serialize};

use crate::wire::{self, JavaCodec};
use crate::{Error, Result};

/// Name of the root directory of every sync tree (French for "root").
pub const SYNC_ROOT_NAME: &str = "racine";

/// A directory of the repository content tree (`.a3s/sync`).
///
/// Mirrors `fr.soe.a3s.domain.repository.SyncTreeDirectory`. The Java `parent` back-reference
/// is implicit in the Rust tree structure and re-created on encoding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTreeDirectory {
    pub name: String,

    /// A directory containing an `addons` sub-directory (i.e. an Arma 3 mod).
    pub mark_as_addon: bool,
    pub updated: bool,
    pub deleted: bool,
    pub hidden: bool,

    /// Children, directories first, each group sorted by `compareToIgnoreCase`.
    pub children: Vec<SyncTreeNode>,
}

/// A file of the repository content tree.
///
/// Mirrors `fr.soe.a3s.domain.repository.SyncTreeLeaf`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTreeLeaf {
    pub name: String,

    /// Lowercase hex SHA-1 of the file, or `"0"` for an empty file.
    pub sha1: Option<String>,
    pub size: i64,

    /// Size of `<name>.zip` on the server, if [`Self::compressed`].
    pub compressed_size: i64,
    pub compressed: bool,
    pub updated: bool,
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncTreeNode {
    Directory(SyncTreeDirectory),
    Leaf(SyncTreeLeaf),
}

impl SyncTreeNode {
    pub fn name(&self) -> &str {
        match self {
            Self::Directory(d) => &d.name,
            Self::Leaf(l) => &l.name,
        }
    }
}

impl SyncTreeDirectory {
    /// An empty root directory (`racine`).
    pub fn new_root() -> Self {
        Self::new(SYNC_ROOT_NAME)
    }

    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Adds a child, keeping Java's order (`SyncTreeDirectory.addTreeNode`): directories first,
    /// then files, each sorted with `compareToIgnoreCase` (stable).
    pub fn add_child(&mut self, node: SyncTreeNode) {
        self.children.push(node);
        self.sort_children();
    }

    /// Re-sorts children into Java order.
    pub fn sort_children(&mut self) {
        self.children.sort_by(|a, b| {
            let kind = |n: &SyncTreeNode| matches!(n, SyncTreeNode::Leaf(_));
            kind(a)
                .cmp(&kind(b))
                .then_with(|| crate::java_string::compare_ignore_case(a.name(), b.name()))
        });
    }

    pub fn directories(&self) -> impl Iterator<Item = &Self> {
        self.children.iter().filter_map(|c| match c {
            SyncTreeNode::Directory(d) => Some(d),
            SyncTreeNode::Leaf(_) => None,
        })
    }

    pub fn leaves(&self) -> impl Iterator<Item = &SyncTreeLeaf> {
        self.children.iter().filter_map(|c| match c {
            SyncTreeNode::Leaf(l) => Some(l),
            SyncTreeNode::Directory(_) => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&SyncTreeNode> {
        self.children.iter().find(|c| c.name() == name)
    }

    /// Visits every node depth-first with its `/`-joined path relative to this directory.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&str, &'a SyncTreeNode)) {
        fn rec<'a>(
            dir: &'a SyncTreeDirectory,
            prefix: &str,
            f: &mut impl FnMut(&str, &'a SyncTreeNode),
        ) {
            for child in &dir.children {
                let path = if prefix.is_empty() {
                    child.name().to_owned()
                } else {
                    format!("{prefix}/{}", child.name())
                };
                f(&path, child);
                if let SyncTreeNode::Directory(d) = child {
                    rec(d, &path, f);
                }
            }
        }
        rec(self, "", f);
    }

    /// Number of leaves in the whole subtree.
    pub fn leaf_count(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |_, node| {
            if matches!(node, SyncTreeNode::Leaf(_)) {
                n += 1;
            }
        });
        n
    }

    /// Decodes a directory. `seen` holds every node decoded so far: a Java-built tree never
    /// shares nodes, so a node referenced twice (or a cycle) is rejected instead of being
    /// expanded again, which would let a crafted file blow up exponentially.
    fn decode_node(
        graph: &Graph,
        id: NodeId,
        depth: usize,
        seen: &mut HashSet<NodeId>,
    ) -> Result<Self> {
        if !seen.insert(id) {
            return Err(
                a3s_jserial::Error::LimitExceeded("shared or cyclic sync tree node").into(),
            );
        }
        if depth > 1_000 {
            return Err(a3s_jserial::Error::LimitExceeded("sync tree depth").into());
        }
        let elements = a3s_jserial::jdk::list(graph, graph.field(id, "list")?)?.unwrap_or_default();
        let mut children = Vec::with_capacity(elements.len());
        for element in elements {
            let Value::Ref(child) = element else {
                continue;
            };
            match graph.class_name_of(child)? {
                wire::SYNC_TREE_DIRECTORY => children.push(SyncTreeNode::Directory(
                    Self::decode_node(graph, child, depth + 1, seen)?,
                )),
                wire::SYNC_TREE_LEAF => {
                    if !seen.insert(child) {
                        return Err(a3s_jserial::Error::LimitExceeded(
                            "shared or cyclic sync tree node",
                        )
                        .into());
                    }
                    children.push(SyncTreeNode::Leaf(SyncTreeLeaf::decode(graph, element)?));
                }
                other => {
                    return Err(Error::UnexpectedClass {
                        expected: wire::SYNC_TREE_LEAF,
                        found: other.to_owned(),
                    });
                }
            }
        }
        Ok(Self {
            name: graph.field_string(id, "name")?.unwrap_or_default(),
            mark_as_addon: graph.field_bool(id, "markAsAddon")?,
            updated: graph.field_bool(id, "updated")?,
            deleted: graph.field_bool(id, "deleted")?,
            hidden: graph.field_bool(id, "hidden")?,
            children,
        })
    }

    fn encode_node(&self, b: &mut Builder, parent: Option<NodeId>) -> Value {
        let class = wire::sync_tree_directory_desc(b);
        let id = b.reserve_object(class);
        let children = self
            .children
            .iter()
            .map(|child| match child {
                SyncTreeNode::Directory(d) => d.encode_node(b, Some(id)),
                SyncTreeNode::Leaf(l) => l.encode_with_parent(b, id),
            })
            .collect();
        let list = b.array_list(children);
        let name = if self.name == SYNC_ROOT_NAME && parent.is_none() {
            // A string literal in Java.
            b.interned(SYNC_ROOT_NAME)
        } else {
            b.string(&self.name)
        };
        b.set_object_data(
            id,
            vec![ClassData {
                values: vec![
                    Value::Boolean(self.deleted),
                    Value::Boolean(self.hidden),
                    Value::Boolean(self.mark_as_addon),
                    Value::Boolean(self.updated),
                    list,
                    name,
                    parent.map_or(Value::Null, Value::Ref),
                ],
                annotations: Vec::new(),
            }],
        );
        Value::Ref(id)
    }
}

/// SHA-1 of an empty file (`FileAccessMethods.computeSHA1`).
const EMPTY_FILE_SHA1: &str = "0";

/// Encodes a leaf's `sha1`. The empty-file value `"0"` is a Java string literal returned by
/// `computeSHA1`, so every empty leaf shares one instance; other hashes are separate strings.
// Mirrors dao/FileAccessMethods.java:238-240
fn encode_sha1(b: &mut Builder, sha1: Option<&str>) -> Value {
    match sha1 {
        Some(EMPTY_FILE_SHA1) => b.interned(EMPTY_FILE_SHA1),
        other => b.opt_string(other),
    }
}

impl SyncTreeLeaf {
    fn encode_with_parent(&self, b: &mut Builder, parent: NodeId) -> Value {
        let class = wire::sync_tree_leaf_desc(b);
        let name = b.string(&self.name);
        let sha1 = encode_sha1(b, self.sha1.as_deref());
        b.simple_object(
            class,
            vec![
                Value::Boolean(self.compressed),
                Value::Long(self.compressed_size),
                Value::Boolean(self.deleted),
                Value::Long(self.size),
                Value::Boolean(self.updated),
                name,
                Value::Ref(parent),
                sha1,
            ],
        )
    }
}

impl JavaCodec for SyncTreeLeaf {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::SYNC_TREE_LEAF)?;
        Ok(Self {
            name: graph.field_string(id, "name")?.unwrap_or_default(),
            sha1: graph.field_string(id, "sha1")?,
            size: graph.field_i64(id, "size")?,
            compressed_size: graph.field_i64(id, "compressedSize")?,
            compressed: graph.field_bool(id, "compressed")?,
            updated: graph.field_bool(id, "updated")?,
            deleted: graph.field_bool(id, "deleted")?,
        })
    }

    /// Encodes a leaf without parent. Leaves normally live in a tree; see
    /// [`SyncTreeDirectory`]'s codec.
    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::sync_tree_leaf_desc(b);
        let name = b.string(&self.name);
        let sha1 = encode_sha1(b, self.sha1.as_deref());
        b.simple_object(
            class,
            vec![
                Value::Boolean(self.compressed),
                Value::Long(self.compressed_size),
                Value::Boolean(self.deleted),
                Value::Long(self.size),
                Value::Boolean(self.updated),
                name,
                Value::Null,
                sha1,
            ],
        )
    }
}

impl JavaCodec for SyncTreeDirectory {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::SYNC_TREE_DIRECTORY)?;
        Self::decode_node(graph, id, 0, &mut HashSet::new())
    }

    fn encode(&self, b: &mut Builder) -> Value {
        self.encode_node(b, None)
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};
    use crate::wire::{decode_stream, encode_stream};

    fn leaf(name: &str, sha1: &str) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            sha1: Some(sha1.to_owned()),
            ..Default::default()
        })
    }

    /// All empty-file leaves share the literal `"0"`, other hashes are separate strings.
    #[test]
    fn empty_file_sha1_is_one_shared_instance() {
        let mut sub = SyncTreeDirectory::new("sub");
        sub.children = vec![leaf("c", "0"), leaf("d", "abc")];
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![
            SyncTreeNode::Directory(sub),
            leaf("a", "0"),
            leaf("b", "abc"),
        ];
        let bytes = encode_stream(&root).unwrap();
        assert_eq!(decode_stream::<SyncTreeDirectory>(&bytes).unwrap(), root);

        let graph = a3s_jserial::read(&bytes).unwrap();
        let mut sha1s = Vec::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            let id = a3s_jserial::NodeId(u32::try_from(index).unwrap());
            if matches!(node, a3s_jserial::Node::Object(_))
                && graph.class_name_of(id).unwrap() == crate::wire::SYNC_TREE_LEAF
            {
                sha1s.push(graph.field(id, "sha1").unwrap());
            }
        }
        // Leaves in stream order: c, d, a, b.
        assert_eq!(sha1s.len(), 4);
        assert_eq!(sha1s[0], sha1s[2], "the two \"0\" are one instance");
        assert_ne!(sha1s[1], sha1s[3], "equal non-empty hashes stay separate");
    }
}
