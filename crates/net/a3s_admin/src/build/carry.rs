//! Carrying objects of a previous `.a3s` file over into a new one, with their instance sharing.
//!
//! Java reads the old `changelogs` and `events` files into objects and writes some of them again.
//! Deserialization re-creates the old stream's sharing (one `String` referenced from two places
//! stays one object), and the new stream reproduces it as back-references. The build therefore
//! keeps old files as raw [`Graph`]s ([`RawGraph`]), resolves the parts it reuses into the
//! `Old*` handles below while planning, and re-emits each old node through a [`Memo`]: every
//! reference to one old node becomes one new instance.

use std::collections::HashMap;

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{ClassData, Content, Graph, Node, NodeId, Value};
use a3s_types::JavaCodec;

/// A raw `.a3s` object graph, as read from a file.
///
/// `read_a3s::<RawGraph>` reads a file with the gzip size limit without interpreting it.
#[derive(Clone, Debug, PartialEq)]
pub struct RawGraph {
    /// The decoded stream.
    pub graph: Graph,

    /// The top-level object.
    pub root: Value,
}

impl JavaCodec for RawGraph {
    fn decode(graph: &Graph, value: Value) -> a3s_types::Result<Self> {
        Ok(Self {
            graph: graph.clone(),
            root: value,
        })
    }

    /// Re-emits the graph node by node (class descriptors, objects, strings, enum constants),
    /// which reproduces streams that hold no arrays, like `.a3s` files. Arrays and `Class`
    /// objects become `null`.
    fn encode(&self, b: &mut Builder) -> Value {
        let mut copier = Copier {
            graph: &self.graph,
            values: HashMap::new(),
            classes: HashMap::new(),
        };
        copier.value(b, self.root)
    }
}

/// Generic node-by-node copy of a [`Graph`] into a [`Builder`].
struct Copier<'g> {
    graph: &'g Graph,
    values: HashMap<NodeId, Value>,
    classes: HashMap<NodeId, Option<NodeId>>,
}

impl Copier<'_> {
    fn class(&mut self, b: &mut Builder, id: NodeId) -> Option<NodeId> {
        if let Some(&class) = self.classes.get(&id) {
            return class;
        }
        let graph = self.graph;
        let class = match graph.node(id) {
            Ok(Node::ClassDesc(desc)) => {
                let super_class = desc.super_class.and_then(|s| self.class(b, s));
                let fields: Vec<FieldSpec<'_>> = desc
                    .fields
                    .iter()
                    .map(|f| FieldSpec {
                        field_type: f.field_type,
                        name: &f.name,
                        signature: f.class_name.and_then(|s| graph.string(s).ok()),
                    })
                    .collect();
                Some(b.class_desc(
                    &desc.name,
                    desc.serial_version_uid,
                    desc.flags,
                    &fields,
                    super_class,
                ))
            }
            _ => None,
        };
        self.classes.insert(id, class);
        class
    }

    fn value(&mut self, b: &mut Builder, value: Value) -> Value {
        let Value::Ref(id) = value else {
            return value;
        };
        if let Some(&copy) = self.values.get(&id) {
            return copy;
        }
        let graph = self.graph;
        let copy = match graph.node(id) {
            Ok(Node::String(s)) => b.string(s),
            Ok(Node::Enum(e)) => match (graph.class_desc(e.class), graph.string(e.constant)) {
                (Ok(desc), Ok(constant)) => b.enum_value(&desc.name, constant),
                _ => Value::Null,
            },
            Ok(Node::Object(obj)) => {
                let Some(class) = self.class(b, obj.class) else {
                    return Value::Null;
                };
                let new_id = b.reserve_object(class);
                // Registered before the fields, so cycles (e.g. `parent`) become back-references.
                self.values.insert(id, Value::Ref(new_id));
                let class_data = obj
                    .class_data
                    .iter()
                    .map(|data| ClassData {
                        values: data.values.iter().map(|v| self.value(b, *v)).collect(),
                        annotations: data
                            .annotations
                            .iter()
                            .map(|c| match c {
                                Content::BlockData(bytes) => Content::BlockData(bytes.clone()),
                                Content::Value(v) => Content::Value(self.value(b, *v)),
                            })
                            .collect(),
                    })
                    .collect();
                b.set_object_data(new_id, class_data);
                Value::Ref(new_id)
            }
            _ => Value::Null,
        };
        self.values.insert(id, copy);
        copy
    }
}

/// A nullable `String` field or element of an old graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OldStr {
    Null,
    Node(NodeId, String),
}

impl OldStr {
    /// Resolves `value` in `graph`.
    pub fn resolve(graph: &Graph, value: Value) -> a3s_jserial::Result<Self> {
        Ok(match value {
            Value::Ref(id) => Self::Node(id, graph.string(id)?.to_owned()),
            _ => Self::Null,
        })
    }

    /// The text, or `None` for null.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Null => None,
            Self::Node(_, s) => Some(s),
        }
    }
}

/// A nullable `java.lang.Boolean` of an old graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OldBool {
    Null,
    Node(NodeId, bool),
}

impl OldBool {
    /// Resolves `value` in `graph`.
    pub fn resolve(graph: &Graph, value: Value) -> a3s_jserial::Result<Self> {
        Ok(match (value, a3s_jserial::jdk::boolean(graph, value)?) {
            (Value::Ref(id), Some(b)) => Self::Node(id, b),
            _ => Self::Null,
        })
    }
}

/// A nullable `java.util.Date` of an old graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OldDate {
    Null,
    Node(NodeId, i64),
}

impl OldDate {
    /// Resolves `value` in `graph`.
    pub fn resolve(graph: &Graph, value: Value) -> a3s_jserial::Result<Self> {
        Ok(match (value, a3s_jserial::jdk::date(graph, value)?) {
            (Value::Ref(id), Some(millis)) => Self::Node(id, millis),
            _ => Self::Null,
        })
    }
}

/// A nullable `List<String>` of an old graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OldList {
    Null,
    Node(NodeId, Vec<OldStr>),
}

impl OldList {
    /// Resolves `value` in `graph`; elements must be strings or null.
    pub fn resolve(graph: &Graph, value: Value) -> a3s_jserial::Result<Self> {
        let Value::Ref(id) = value else {
            return Ok(Self::Null);
        };
        let elements = a3s_jserial::jdk::list(graph, value)?.unwrap_or_default();
        let elements = elements
            .into_iter()
            .map(|e| OldStr::resolve(graph, e))
            .collect::<a3s_jserial::Result<_>>()?;
        Ok(Self::Node(id, elements))
    }

    /// The elements (none for null).
    pub fn elements(&self) -> &[OldStr] {
        match self {
            Self::Null => &[],
            Self::Node(_, elements) => elements,
        }
    }
}

/// Re-emission memo: one new instance per old node.
#[derive(Default)]
pub struct Memo {
    values: HashMap<NodeId, Value>,
}

impl Memo {
    fn get_or(&mut self, id: NodeId, make: impl FnOnce() -> Value) -> Value {
        *self.values.entry(id).or_insert_with(make)
    }

    /// The new instance of an old string.
    pub fn string(&mut self, b: &mut Builder, s: &OldStr) -> Value {
        match s {
            OldStr::Null => Value::Null,
            OldStr::Node(id, text) => self.get_or(*id, || b.string(text)),
        }
    }

    /// The new instance of an old `Boolean`. Booleans are canonical (`Boolean.valueOf`), so two
    /// old `Boolean` objects with the same value become one (Q11).
    pub fn boolean(&mut self, b: &mut Builder, v: OldBool) -> Value {
        match v {
            OldBool::Null => Value::Null,
            OldBool::Node(id, value) => self.get_or(id, || b.boolean(value)),
        }
    }

    /// The new instance of an old `Date`.
    pub fn date(&mut self, b: &mut Builder, d: OldDate) -> Value {
        match d {
            OldDate::Null => Value::Null,
            OldDate::Node(id, millis) => self.get_or(id, || b.date(millis)),
        }
    }

    /// The new instance of an old `ArrayList<String>`, with its elements re-emitted.
    pub fn list(&mut self, b: &mut Builder, list: &OldList) -> Value {
        match list {
            OldList::Null => Value::Null,
            OldList::Node(id, elements) => {
                if let Some(&v) = self.values.get(id) {
                    return v;
                }
                let values = elements.iter().map(|e| self.string(b, e)).collect();
                let v = b.array_list(values);
                self.values.insert(*id, v);
                v
            }
        }
    }

    /// An already re-emitted object, if any (for objects that appear twice in a list).
    pub fn object(&self, id: NodeId) -> Option<Value> {
        self.values.get(&id).copied()
    }

    /// Records the re-emitted instance of an old object.
    pub fn set_object(&mut self, id: NodeId, value: Value) {
        self.values.insert(id, value);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::path::Path;

    use super::RawGraph;

    fn gunzip(path: &Path) -> Vec<u8> {
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
            .read_to_end(&mut raw)
            .unwrap();
        raw
    }

    /// The generic copy reproduces Java-written `.a3s` streams byte for byte.
    #[test]
    fn raw_graph_copy_reproduces_java_streams() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/java_repos/wog_ftp");
        for name in ["sync", "serverinfo", "changelogs", "autoconfig", "events"] {
            let path = dir.join(name);
            let raw: RawGraph = a3s_repo_files::read_a3s(&path).unwrap();
            assert_eq!(
                a3s_types::encode_stream(&raw).unwrap(),
                gunzip(&path),
                "{name}"
            );
        }
    }
}
