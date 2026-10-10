//! Knowledge about JDK classes: decoding their `writeObject` layouts, and building them.
//!
//! The descriptors here (`serialVersionUID`s, flags, fields) are those of `OpenJDK` and must not
//! change, otherwise Java readers reject the stream.

use std::collections::HashMap;

use crate::constants::MAX_BLOCK_SIZE;
use crate::model::{
    ClassData, ClassDesc, Content, EnumConst, FieldDesc, FieldType, Graph, Node, NodeId, Object,
    Value,
};
use crate::{Error, Result, flags};

pub const ARRAY_LIST: &str = "java.util.ArrayList";
pub const LINKED_LIST: &str = "java.util.LinkedList";
pub const HASH_MAP: &str = "java.util.HashMap";
pub const LINKED_HASH_MAP: &str = "java.util.LinkedHashMap";
pub const TREE_MAP: &str = "java.util.TreeMap";
pub const HASHTABLE: &str = "java.util.Hashtable";
pub const PROPERTIES: &str = "java.util.Properties";
pub const HASH_SET: &str = "java.util.HashSet";
pub const LINKED_HASH_SET: &str = "java.util.LinkedHashSet";
pub const TREE_SET: &str = "java.util.TreeSet";
pub const DATE: &str = "java.util.Date";
pub const BOOLEAN: &str = "java.lang.Boolean";
pub const INTEGER: &str = "java.lang.Integer";
pub const LONG: &str = "java.lang.Long";
pub const NUMBER: &str = "java.lang.Number";
pub const ENUM: &str = "java.lang.Enum";

/// `HashMap`'s default load factor.
const LOAD_FACTOR: f32 = 0.75;

/// `HashMap.DEFAULT_INITIAL_CAPACITY`.
const DEFAULT_CAPACITY: i32 = 16;

// ---------------------------------------------------------------------------------------------
// Decoding

/// Sequential reader over `writeObject` data: block data bytes interleaved with objects.
pub struct AnnotationReader<'a> {
    contents: &'a [Content],
    index: usize,
    offset: usize,
    class: &'a str,
}

impl<'a> AnnotationReader<'a> {
    pub fn new(contents: &'a [Content], class: &'a str) -> Self {
        Self {
            contents,
            index: 0,
            offset: 0,
            class,
        }
    }

    fn malformed(&self, reason: &'static str) -> Error {
        Error::MalformedData {
            class: self.class.to_owned(),
            reason,
        }
    }

    /// Reads `N` bytes of block data, possibly spanning several chunks.
    pub fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        let mut filled = 0;
        while filled < N {
            match self.contents.get(self.index) {
                Some(Content::BlockData(block)) => {
                    let available = &block[self.offset.min(block.len())..];
                    let n = available.len().min(N - filled);
                    out[filled..filled + n].copy_from_slice(&available[..n]);
                    filled += n;
                    self.offset += n;
                    if self.offset >= block.len() {
                        self.index += 1;
                        self.offset = 0;
                    }
                }
                _ => return Err(self.malformed("expected block data")),
            }
        }
        Ok(out)
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.bytes()?))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.bytes()?))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(u32::from_be_bytes(self.bytes()?)))
    }

    pub fn object(&mut self) -> Result<Value> {
        // Skip exhausted (or empty) block chunks.
        while let Some(Content::BlockData(block)) = self.contents.get(self.index) {
            if self.offset < block.len() {
                return Err(self.malformed("expected an object, found block data"));
            }
            self.index += 1;
            self.offset = 0;
        }
        match self.contents.get(self.index) {
            Some(Content::Value(value)) => {
                self.index += 1;
                Ok(*value)
            }
            _ => Err(self.malformed("missing object")),
        }
    }
}

fn annotations_of<'g>(
    graph: &'g Graph,
    id: NodeId,
    class: &'g str,
) -> Result<AnnotationReader<'g>> {
    let data = graph.class_data_of(id, class)?;
    Ok(AnnotationReader::new(&data.annotations, class))
}

fn checked_len(len: i32, class: &str) -> Result<usize> {
    usize::try_from(len).map_err(|_err| Error::MalformedData {
        class: class.to_owned(),
        reason: "negative size",
    })
}

/// Elements of a `java.util.List`, or `None` for null.
pub fn list(graph: &Graph, value: Value) -> Result<Option<Vec<Value>>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    let class = graph.class_name_of(id)?;
    let elements = match class {
        ARRAY_LIST | LINKED_LIST => {
            let mut r = annotations_of(graph, id, class)?;
            let size = checked_len(r.i32()?, class)?;
            let size = if class == ARRAY_LIST {
                checked_len(graph.field_i32(id, "size")?, class)?
            } else {
                size
            };
            (0..size).map(|_| r.object()).collect::<Result<_>>()?
        }
        "java.util.Arrays$ArrayList" => match graph.field(id, "a")? {
            Value::Ref(arr) => match graph.node(arr)? {
                Node::Array(a) => match &a.data {
                    crate::ArrayData::Object(values) => values.clone(),
                    _ => return Err(Error::UnsupportedCollection(class.to_owned())),
                },
                _ => return Err(Error::WrongNodeKind(arr, "array")),
            },
            _ => Vec::new(),
        },
        "java.util.Collections$EmptyList" => Vec::new(),
        "java.util.Collections$SingletonList" => vec![graph.field(id, "element")?],
        "java.util.Collections$UnmodifiableList"
        | "java.util.Collections$UnmodifiableRandomAccessList"
        | "java.util.Collections$SynchronizedList"
        | "java.util.Collections$SynchronizedRandomAccessList" => {
            return list(graph, graph.field(id, "list")?);
        }
        other => return Err(Error::UnsupportedCollection(other.to_owned())),
    };
    Ok(Some(elements))
}

/// Entries of a `java.util.Map`, in stream order, or `None` for null.
pub fn map(graph: &Graph, value: Value) -> Result<Option<Vec<(Value, Value)>>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    let class = graph.class_name_of(id)?;
    let (mut r, size) = match class {
        HASH_MAP | LINKED_HASH_MAP => {
            let mut r = annotations_of(graph, id, HASH_MAP)?;
            let _buckets = r.i32()?;
            let size = r.i32()?;
            (r, size)
        }
        TREE_MAP => {
            let mut r = annotations_of(graph, id, TREE_MAP)?;
            let size = r.i32()?;
            (r, size)
        }
        HASHTABLE | PROPERTIES => {
            let mut r = annotations_of(graph, id, HASHTABLE)?;
            let _length = r.i32()?;
            let count = r.i32()?;
            (r, count)
        }
        "java.util.Collections$EmptyMap" => return Ok(Some(Vec::new())),
        "java.util.Collections$UnmodifiableMap" | "java.util.Collections$SynchronizedMap" => {
            return map(graph, graph.field(id, "m")?);
        }
        other => return Err(Error::UnsupportedCollection(other.to_owned())),
    };
    let size = checked_len(size, class)?;
    let mut entries = Vec::with_capacity(size.min(1 << 16));
    for _ in 0..size {
        let k = r.object()?;
        let v = r.object()?;
        entries.push((k, v));
    }
    Ok(Some(entries))
}

/// Elements of a `java.util.Set`, in stream order, or `None` for null.
pub fn set(graph: &Graph, value: Value) -> Result<Option<Vec<Value>>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    let class = graph.class_name_of(id)?;
    let (mut r, size) = match class {
        HASH_SET | LINKED_HASH_SET => {
            let mut r = annotations_of(graph, id, HASH_SET)?;
            let _capacity = r.i32()?;
            let _load_factor = r.f32()?;
            let size = r.i32()?;
            (r, size)
        }
        TREE_SET => {
            let mut r = annotations_of(graph, id, TREE_SET)?;
            let _comparator = r.object()?;
            let size = r.i32()?;
            (r, size)
        }
        "java.util.Collections$EmptySet" => return Ok(Some(Vec::new())),
        "java.util.Collections$UnmodifiableSet" | "java.util.Collections$SynchronizedSet" => {
            return set(graph, graph.field(id, "c")?);
        }
        other => return Err(Error::UnsupportedCollection(other.to_owned())),
    };
    let size = checked_len(size, class)?;
    (0..size)
        .map(|_| r.object())
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// Milliseconds since the Unix epoch of a `java.util.Date`, or `None` for null.
pub fn date(graph: &Graph, value: Value) -> Result<Option<i64>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    expect_class(graph, id, DATE)?;
    annotations_of(graph, id, DATE)?.i64().map(Some)
}

/// Value of a `java.lang.Boolean`, or `None` for null.
pub fn boolean(graph: &Graph, value: Value) -> Result<Option<bool>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    expect_class(graph, id, BOOLEAN)?;
    graph.field_bool(id, "value").map(Some)
}

/// Value of a `java.lang.Integer`, or `None` for null.
pub fn integer(graph: &Graph, value: Value) -> Result<Option<i32>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    expect_class(graph, id, INTEGER)?;
    graph.field_i32(id, "value").map(Some)
}

/// Checks that `id` is an instance of exactly `class`.
pub fn expect_class(graph: &Graph, id: NodeId, class: &str) -> Result<()> {
    let found = graph.class_name_of(id)?;
    if found == class {
        Ok(())
    } else {
        Err(Error::WrongClass {
            expected: class.to_owned(),
            found: found.to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Java hashing, to reproduce `HashMap` iteration order.

/// `java.lang.String.hashCode()`.
pub fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, unit| {
        h.wrapping_mul(31).wrapping_add(i32::from(unit))
    })
}

/// `HashMap.hash()`: spreads higher bits downwards.
fn spread(hash: i32) -> i32 {
    hash ^ ((hash as u32) >> 16) as i32
}

/// Table capacity of a `new HashMap<>()` after `len` sequential `put`s.
pub fn hash_map_capacity(len: usize) -> i32 {
    let mut cap = DEFAULT_CAPACITY as usize;
    while len > cap * 3 / 4 {
        cap *= 2;
    }
    i32::try_from(cap).unwrap_or(1 << 30)
}

/// Orders `hashes` (given in insertion order) the way a `HashMap` of capacity `cap` iterates.
///
/// Returns indices into `hashes`.
pub fn hash_map_order(hashes: &[i32], cap: i32) -> Vec<usize> {
    let mask = cap - 1;
    let mut order: Vec<usize> = (0..hashes.len()).collect();
    // Stable sort keeps insertion order within a bucket.
    order.sort_by_key(|&i| spread(hashes[i]) & mask);
    order
}

// ---------------------------------------------------------------------------------------------
// Building

/// Splits `bytes` into block-data chunks the way `ObjectOutputStream` does.
pub fn block_data(bytes: &[u8]) -> Vec<Content> {
    bytes
        .chunks(MAX_BLOCK_SIZE)
        .map(|chunk| Content::BlockData(chunk.to_vec()))
        .collect()
}

/// Specification of one serializable field for [`Builder::class_desc`].
#[derive(Clone, Copy, Debug)]
pub struct FieldSpec<'a> {
    pub field_type: FieldType,
    pub name: &'a str,

    /// JVM type descriptor for object/array fields, e.g. `Ljava/lang/String;`.
    pub signature: Option<&'a str>,
}

impl<'a> FieldSpec<'a> {
    pub const fn prim(field_type: FieldType, name: &'a str) -> Self {
        Self {
            field_type,
            name,
            signature: None,
        }
    }

    pub const fn object(name: &'a str, signature: &'a str) -> Self {
        Self {
            field_type: FieldType::Object,
            name,
            signature: Some(signature),
        }
    }
}

/// Builds a [`Graph`] the way a Java program would produce it.
///
/// Class descriptors and field type strings are shared (Java interns them), enum constants and
/// `Boolean`s are shared (they are singletons in Java), plain strings are not unless requested
/// with [`Builder::interned`].
#[derive(Default)]
pub struct Builder {
    graph: Graph,
    classes: HashMap<String, NodeId>,
    interned: HashMap<String, NodeId>,
    enums: HashMap<(String, String), NodeId>,
    booleans: [Option<NodeId>; 2],
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Finishes the graph with `root` as the single top-level object.
    pub fn finish(mut self, root: Value) -> Graph {
        self.graph.roots = vec![Content::Value(root)];
        self.graph
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Returns the class descriptor named `name`, creating it on first use.
    ///
    /// `fields` must be in Java wire order: primitives first, then objects, each sorted by name.
    pub fn class_desc(
        &mut self,
        name: &str,
        serial_version_uid: i64,
        desc_flags: u8,
        fields: &[FieldSpec<'_>],
        super_class: Option<NodeId>,
    ) -> NodeId {
        if let Some(&id) = self.classes.get(name) {
            return id;
        }
        let fields = fields
            .iter()
            .map(|f| FieldDesc {
                field_type: f.field_type,
                name: f.name.to_owned(),
                class_name: f.signature.map(|s| self.interned_id(s)),
            })
            .collect();
        let id = self.graph.push(Node::ClassDesc(ClassDesc {
            name: name.to_owned(),
            serial_version_uid,
            flags: desc_flags,
            fields,
            annotations: Vec::new(),
            super_class,
        }));
        self.classes.insert(name.to_owned(), id);
        id
    }

    /// A new, unshared `String` instance.
    pub fn string(&mut self, s: &str) -> Value {
        Value::Ref(self.graph.push(Node::String(s.to_owned())))
    }

    /// A nullable string; `None` becomes `null`.
    pub fn opt_string(&mut self, s: Option<&str>) -> Value {
        s.map_or(Value::Null, |s| self.string(s))
    }

    fn interned_id(&mut self, s: &str) -> NodeId {
        if let Some(&id) = self.interned.get(s) {
            return id;
        }
        let id = self.graph.push(Node::String(s.to_owned()));
        self.interned.insert(s.to_owned(), id);
        id
    }

    /// An interned string (like a Java literal): equal strings share one instance.
    pub fn interned(&mut self, s: &str) -> Value {
        Value::Ref(self.interned_id(s))
    }

    /// An enum constant. Repeated constants share one instance.
    pub fn enum_value(&mut self, class: &str, constant: &str) -> Value {
        let key = (class.to_owned(), constant.to_owned());
        if let Some(&id) = self.enums.get(&key) {
            return Value::Ref(id);
        }
        let enum_flags = flags::SERIALIZABLE | flags::ENUM;
        let base = self.class_desc(ENUM, 0, enum_flags, &[], None);
        let class = self.class_desc(class, 0, enum_flags, &[], Some(base));
        let constant_id = self.interned_id(constant);
        let id = self.graph.push(Node::Enum(EnumConst {
            class,
            constant: constant_id,
        }));
        self.enums.insert(key, id);
        Value::Ref(id)
    }

    /// Creates an object with no data yet; fill it with [`Self::set_object_data`].
    ///
    /// Use this for objects that are referenced (e.g. as `parent`) by their own children.
    pub fn reserve_object(&mut self, class: NodeId) -> NodeId {
        self.graph.push(Node::Object(Object {
            class,
            class_data: Vec::new(),
        }))
    }

    pub fn set_object_data(&mut self, id: NodeId, class_data: Vec<ClassData>) {
        if let Ok(obj) = self.graph.object_mut(id) {
            obj.class_data = class_data;
        }
    }

    /// An object of a class with a single serializable class in its hierarchy.
    pub fn simple_object(&mut self, class: NodeId, values: Vec<Value>) -> Value {
        let id = self.reserve_object(class);
        self.set_object_data(
            id,
            vec![ClassData {
                values,
                annotations: Vec::new(),
            }],
        );
        Value::Ref(id)
    }

    fn collection_desc(&mut self, name: &str, uid: i64, fields: &[FieldSpec<'_>]) -> NodeId {
        self.class_desc(
            name,
            uid,
            flags::SERIALIZABLE | flags::WRITE_METHOD,
            fields,
            None,
        )
    }

    /// `java.util.ArrayList` with the given elements.
    pub fn array_list(&mut self, elements: Vec<Value>) -> Value {
        let class = self.collection_desc(
            ARRAY_LIST,
            8_683_452_581_122_892_189,
            &[FieldSpec::prim(FieldType::Int, "size")],
        );
        let size = i32::try_from(elements.len()).expect("list too large");
        let mut annotations = block_data(&size.to_be_bytes());
        annotations.extend(elements.into_iter().map(Content::Value));
        let id = self.reserve_object(class);
        self.set_object_data(
            id,
            vec![ClassData {
                values: vec![Value::Int(size)],
                annotations,
            }],
        );
        Value::Ref(id)
    }

    /// `java.util.HashMap` built with `new HashMap<>()` and `put`s of string keys in the
    /// given order. Entries are written in Java's iteration order.
    pub fn hash_map_str(&mut self, entries: &[(String, Value)]) -> Value {
        let class = self.collection_desc(
            HASH_MAP,
            362_498_820_763_181_265,
            &[
                FieldSpec::prim(FieldType::Float, "loadFactor"),
                FieldSpec::prim(FieldType::Int, "threshold"),
            ],
        );
        let len = entries.len();
        let (buckets, threshold) = if len == 0 {
            (DEFAULT_CAPACITY, 0)
        } else {
            let cap = hash_map_capacity(len);
            (cap, (cap as f32 * LOAD_FACTOR) as i32)
        };
        let hashes: Vec<i32> = entries.iter().map(|(k, _)| java_string_hash(k)).collect();
        let order = hash_map_order(&hashes, buckets);

        let mut header = Vec::with_capacity(8);
        header.extend_from_slice(&buckets.to_be_bytes());
        header.extend_from_slice(&i32::try_from(len).expect("map too large").to_be_bytes());
        let mut annotations = block_data(&header);
        for i in order {
            let (key, value) = &entries[i];
            let key = self.string(key);
            annotations.push(Content::Value(key));
            annotations.push(Content::Value(*value));
        }
        let id = self.reserve_object(class);
        self.set_object_data(
            id,
            vec![ClassData {
                values: vec![Value::Float(LOAD_FACTOR), Value::Int(threshold)],
                annotations,
            }],
        );
        Value::Ref(id)
    }

    /// `java.util.HashSet` built with `new HashSet<>()` and `add`s of strings in the given
    /// order. Elements are written in Java's iteration order.
    pub fn hash_set_str(&mut self, elements: &[String]) -> Value {
        let class = self.collection_desc(HASH_SET, -5_024_744_406_713_321_676, &[]);
        let len = elements.len();
        let capacity = if len == 0 {
            DEFAULT_CAPACITY
        } else {
            hash_map_capacity(len)
        };
        let hashes: Vec<i32> = elements.iter().map(|s| java_string_hash(s)).collect();
        let order = hash_map_order(&hashes, capacity);

        let mut header = Vec::with_capacity(12);
        header.extend_from_slice(&capacity.to_be_bytes());
        header.extend_from_slice(&LOAD_FACTOR.to_bits().to_be_bytes());
        header.extend_from_slice(&i32::try_from(len).expect("set too large").to_be_bytes());
        let mut annotations = block_data(&header);
        for i in order {
            let element = self.string(&elements[i]);
            annotations.push(Content::Value(element));
        }
        let id = self.reserve_object(class);
        self.set_object_data(
            id,
            vec![ClassData {
                values: Vec::new(),
                annotations,
            }],
        );
        Value::Ref(id)
    }

    /// `java.util.Date` at `millis` since the Unix epoch.
    pub fn date(&mut self, millis: i64) -> Value {
        let class = self.collection_desc(DATE, 7_523_967_970_034_938_905, &[]);
        let id = self.reserve_object(class);
        self.set_object_data(
            id,
            vec![ClassData {
                values: Vec::new(),
                annotations: block_data(&millis.to_be_bytes()),
            }],
        );
        Value::Ref(id)
    }

    /// `Boolean.valueOf(b)`; both constants are shared singletons.
    pub fn boolean(&mut self, b: bool) -> Value {
        let slot = usize::from(b);
        if let Some(id) = self.booleans[slot] {
            return Value::Ref(id);
        }
        let class = self.class_desc(
            BOOLEAN,
            -3_665_804_199_014_368_530,
            flags::SERIALIZABLE,
            &[FieldSpec::prim(FieldType::Boolean, "value")],
            None,
        );
        let Value::Ref(id) = self.simple_object(class, vec![Value::Boolean(b)]) else {
            unreachable!()
        };
        self.booleans[slot] = Some(id);
        Value::Ref(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_map_capacity, java_string_hash};

    #[test]
    fn string_hash_matches_java() {
        // Values computed with `"…".hashCode()` in Java.
        assert_eq!(java_string_hash(""), 0);
        assert_eq!(java_string_hash("a"), 97);
        assert_eq!(java_string_hash("hello"), 99_162_322);
        assert_eq!(java_string_hash("racine"), -938_608_820);
    }

    #[test]
    fn capacity_growth() {
        assert_eq!(hash_map_capacity(0), 16);
        assert_eq!(hash_map_capacity(12), 16);
        assert_eq!(hash_map_capacity(13), 32);
        assert_eq!(hash_map_capacity(24), 32);
        assert_eq!(hash_map_capacity(25), 64);
    }
}
