//! The bytes Java uploads for a local `.a3s` file.
//!
//! Java does not upload the local files: it deserializes them into objects
//! (`RepositoryService.readLocalyBuildedRepository`) and serializes those objects again
//! (`ObjectOutputStream(GZIPOutputStream)`). That round trip is not the identity:
//!
//! * objects are written with the **current** class descriptors, fields copied by name;
//! * every `String`, `Boolean` and `Date` node of the old stream becomes one new object, so the
//!   old stream's sharing is kept (deserialized `Boolean`s are not the canonical constants);
//! * `HashMap` and `HashSet` are rebuilt by `readObject`, which sizes their tables from the
//!   number of entries, then written in the rebuilt table's iteration order.
//!
//! [`java_roundtrip`] reproduces this on a [`Graph`]; [`java_roundtrip_bytes`] adds the reader
//! limits check and gzip.

#![allow(
    clippy::result_large_err,
    reason = "the errors carry transport errors; they are returned once per operation"
)]

use std::collections::HashMap;

use a3s_jserial::jdk::{
    self, AnnotationReader, BOOLEAN, Builder, DATE, FieldSpec, HASH_MAP, HASH_SET, block_data,
};
use a3s_jserial::{ClassData, Content, FieldType, Graph, Node, NodeId, Value, flags};
use a3s_repo_files::A3sFile;
use a3s_types::wire;

use super::UploadError;
use crate::build::java_hash::JavaHashMap;

/// `HashMap.DEFAULT_LOAD_FACTOR`, the only load factor Arma3Sync streams use.
const LOAD_FACTOR: f32 = 0.75;

/// `HashMap.MAXIMUM_CAPACITY`.
const MAXIMUM_CAPACITY: f32 = (1 << 30) as f32;

/// Field changes applied to the object Java uploads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoundTripOverrides {
    /// `compressedPboFilesOnly` of a `ServerInfo` root, set from the upload settings.
    pub compressed_pbo_files_only: Option<bool>,
}

/// A stream Java would not read back into the objects it uploads.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RoundTripError(pub String);

impl From<a3s_jserial::Error> for RoundTripError {
    fn from(err: a3s_jserial::Error) -> Self {
        Self(err.to_string())
    }
}

type Result<T, E = RoundTripError> = std::result::Result<T, E>;

/// `readObject` then `writeObject` of the single object in `graph`, as Java uploads it.
// Mirrors dao/connection/ftp/FtpDAO.java:575-597 (sftp/SftpDAO.java:316-330,
// webdav/WebdavDAO.java:328-342) applied to service/RepositoryService.java:1394-1419
pub fn java_roundtrip(graph: &Graph, overrides: &RoundTripOverrides) -> Result<Graph> {
    let root = graph.root_value()?;
    let mut copier = Copier {
        old: graph,
        b: Builder::new(),
        values: HashMap::new(),
        overrides: *overrides,
        root: root.as_ref(),
    };
    let new_root = copier.value(root)?;
    Ok(copier.b.finish(new_root))
}

/// The gzip bytes Java uploads for the local `.a3s` file `file` (see [`java_roundtrip`]),
/// checked against Java's and Rust's reader limits.
pub fn java_roundtrip_bytes(
    file: A3sFile,
    graph: &Graph,
    overrides: &RoundTripOverrides,
) -> Result<Vec<u8>, UploadError> {
    let error = |reason: String| UploadError::RoundTrip { file, reason };
    let new = java_roundtrip(graph, overrides).map_err(|e| error(e.0))?;
    let raw = a3s_jserial::write(&new).map_err(|e| error(e.to_string()))?;
    crate::build::write::check_stream(&raw).map_err(|e| error(e.to_string()))?;
    a3s_repo_files::encode_a3s_stream(&raw).map_err(|e| error(e.to_string()))
}

/// The `HashSet` capacity Java writes after deserializing a set of `n` elements.
#[doc(hidden)]
pub fn hash_set_roundtrip_capacity(n: usize) -> i32 {
    let mut set = deserialized_hash_set(n);
    for i in 0..n {
        set.put(i.to_string(), (), ());
    }
    set.write_params().0
}

/// The backing map of `new HashSet` as `HashSet.readObject` creates it for `size` elements,
/// before the elements are added.
// Mirrors java.util.HashSet.readObject (OpenJDK 25)
fn deserialized_hash_set<K, V>(size: usize) -> JavaHashMap<K, V> {
    let capacity = (size as f32 * (1.0 / LOAD_FACTOR).min(4.0)).min(MAXIMUM_CAPACITY) as i32;
    JavaHashMap::with_initial_capacity(capacity)
}

/// `java.util.ArrayList` descriptor.
fn array_list_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        jdk::ARRAY_LIST,
        8_683_452_581_122_892_189,
        flags::SERIALIZABLE | flags::WRITE_METHOD,
        &[FieldSpec::prim(FieldType::Int, "size")],
        None,
    )
}

/// `java.util.HashMap` descriptor.
fn hash_map_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        HASH_MAP,
        362_498_820_763_181_265,
        flags::SERIALIZABLE | flags::WRITE_METHOD,
        &[
            FieldSpec::prim(FieldType::Float, "loadFactor"),
            FieldSpec::prim(FieldType::Int, "threshold"),
        ],
        None,
    )
}

/// `java.util.HashSet` descriptor.
fn hash_set_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        HASH_SET,
        -5_024_744_406_713_321_676,
        flags::SERIALIZABLE | flags::WRITE_METHOD,
        &[],
        None,
    )
}

/// `java.util.Date` descriptor.
fn date_desc(b: &mut Builder) -> NodeId {
    b.class_desc(
        DATE,
        7_523_967_970_034_938_905,
        flags::SERIALIZABLE | flags::WRITE_METHOD,
        &[],
        None,
    )
}

/// A new `java.lang.Boolean` object that is not one of the canonical constants (a deserialized
/// `Boolean`).
pub(crate) fn distinct_boolean(b: &mut Builder, value: bool) -> Value {
    let class = b.class_desc(
        BOOLEAN,
        -3_665_804_199_014_368_530,
        flags::SERIALIZABLE,
        &[FieldSpec::prim(FieldType::Boolean, "value")],
        None,
    );
    b.simple_object(class, vec![Value::Boolean(value)])
}

/// Current descriptor of an Arma3Sync class that `.a3s` files hold.
fn domain_desc(b: &mut Builder, name: &str) -> Option<NodeId> {
    Some(match name {
        wire::SYNC_TREE_DIRECTORY => wire::sync_tree_directory_desc(b),
        wire::SYNC_TREE_LEAF => wire::sync_tree_leaf_desc(b),
        wire::SERVER_INFO => wire::server_info_desc(b),
        wire::CHANGELOGS => wire::changelogs_desc(b),
        wire::CHANGELOG => wire::changelog_desc(b),
        wire::EVENTS => wire::events_desc(b),
        wire::EVENT => wire::event_desc(b),
        wire::AUTO_CONFIG => wire::auto_config_desc(b),
        wire::FAVORITE_SERVER => wire::favorite_server_desc(b),
        wire::HTTP | wire::FTP | wire::WEBDAV | wire::SOCKS | wire::SFTP => {
            wire::protocol_class_desc(b, name)
        }
        _ => return None,
    })
}

/// The value a field gets when the stream does not have it.
fn default_value(field_type: FieldType) -> Value {
    match field_type {
        FieldType::Byte => Value::Byte(0),
        FieldType::Char => Value::Char(0),
        FieldType::Double => Value::Double(0.0),
        FieldType::Float => Value::Float(0.0),
        FieldType::Int => Value::Int(0),
        FieldType::Long => Value::Long(0),
        FieldType::Short => Value::Short(0),
        FieldType::Boolean => Value::Boolean(false),
        FieldType::Array | FieldType::Object => Value::Null,
    }
}

/// Re-emits an old graph node by node with a memo: one new object per old node.
struct Copier<'g> {
    old: &'g Graph,
    b: Builder,
    values: HashMap<NodeId, Value>,
    overrides: RoundTripOverrides,
    root: Option<NodeId>,
}

impl Copier<'_> {
    fn value(&mut self, value: Value) -> Result<Value> {
        let Value::Ref(id) = value else {
            return Ok(value);
        };
        if let Some(&copy) = self.values.get(&id) {
            return Ok(copy);
        }
        let old = self.old;
        let copy = match old.node(id)? {
            Node::String(s) => self.b.string(s),
            Node::Enum(e) => {
                let class = &old.class_desc(e.class)?.name;
                if !class.starts_with("fr.soe.a3s.") {
                    return Err(unsupported(class));
                }
                self.b.enum_value(class, old.string(e.constant)?)
            }
            Node::Object(_) => return self.object(id),
            Node::ClassDesc(_) | Node::Array(_) | Node::Class(_) => {
                return Err(RoundTripError(format!(
                    "unsupported stream content at handle {}",
                    id.0
                )));
            }
        };
        self.values.insert(id, copy);
        Ok(copy)
    }

    /// Reserves the new object of `old_id` (so references from inside it become back
    /// references) and returns its id.
    fn reserve(&mut self, old_id: NodeId, class: NodeId) -> NodeId {
        let new_id = self.b.reserve_object(class);
        self.values.insert(old_id, Value::Ref(new_id));
        new_id
    }

    fn object(&mut self, id: NodeId) -> Result<Value> {
        let old = self.old;
        let class_name = old.class_name_of(id)?.to_owned();
        let value = Value::Ref(id);
        match class_name.as_str() {
            jdk::ARRAY_LIST => {
                let elements = jdk::list(old, value)?.unwrap_or_default();
                let class = array_list_desc(&mut self.b);
                let new_id = self.reserve(id, class);
                let size = i32::try_from(elements.len())
                    .map_err(|_err| RoundTripError("list too large".to_owned()))?;
                let mut annotations = block_data(&size.to_be_bytes());
                for element in elements {
                    annotations.push(Content::Value(self.value(element)?));
                }
                self.b.set_object_data(
                    new_id,
                    vec![ClassData {
                        values: vec![Value::Int(size)],
                        annotations,
                    }],
                );
            }
            HASH_MAP => self.hash_map(id)?,
            HASH_SET => self.hash_set(id)?,
            DATE => {
                let millis = jdk::date(old, value)?.unwrap_or_default();
                let class = date_desc(&mut self.b);
                let new_id = self.reserve(id, class);
                self.b.set_object_data(
                    new_id,
                    vec![ClassData {
                        values: Vec::new(),
                        annotations: block_data(&millis.to_be_bytes()),
                    }],
                );
            }
            BOOLEAN => {
                let flag = jdk::boolean(old, value)?.unwrap_or_default();
                let copy = distinct_boolean(&mut self.b, flag);
                self.values.insert(id, copy);
            }
            name if name.starts_with("fr.soe.a3s.") => self.domain_object(id, name)?,
            other => return Err(unsupported(other)),
        }
        Ok(self.values[&id])
    }

    /// An Arma3Sync object: current descriptors, fields copied by name per class level.
    // Mirrors java.io.ObjectInputStream.defaultReadFields (class evolution: missing fields get
    // their default value, unknown stream fields are dropped)
    fn domain_object(&mut self, id: NodeId, class_name: &str) -> Result<()> {
        let old = self.old;
        let class = domain_desc(&mut self.b, class_name).ok_or_else(|| unsupported(class_name))?;
        let new_id = self.reserve(id, class);
        let object = old.object(id)?;
        let old_levels = old.hierarchy(object.class)?;
        let new_levels = self.b.graph().hierarchy(class)?;
        let mut class_data = Vec::with_capacity(new_levels.len());
        for level in new_levels {
            let desc = self.b.graph().class_desc(level)?.clone();
            let mut old_level = None;
            for (old_desc, data) in old_levels.iter().zip(&object.class_data) {
                let old_desc = old.class_desc(*old_desc)?;
                if old_desc.name == desc.name {
                    old_level = Some((old_desc, data));
                }
            }
            let mut values = Vec::with_capacity(desc.fields.len());
            for field in &desc.fields {
                let old_field = old_level.and_then(|(old_desc, data)| {
                    let i = old_desc.fields.iter().position(|f| f.name == field.name)?;
                    Some((old_desc.fields[i].field_type, data.values.get(i).copied()))
                });
                let mut new_value = match old_field {
                    None => default_value(field.field_type),
                    Some((old_type, _)) if old_type != field.field_type => {
                        return Err(RoundTripError(format!(
                            "incompatible types for field {}.{}",
                            desc.name, field.name
                        )));
                    }
                    Some((_, None)) => {
                        return Err(RoundTripError(format!(
                            "missing value of field {}.{}",
                            desc.name, field.name
                        )));
                    }
                    Some((_, Some(v))) => self.value(v)?,
                };
                // Mirrors service/connection/ConnectionUploadProcessor.java:89-90
                if let Some(flag) = self.overrides.compressed_pbo_files_only
                    && self.root == Some(id)
                    && desc.name == wire::SERVER_INFO
                    && field.name == "compressedPboFilesOnly"
                {
                    new_value = Value::Boolean(flag);
                }
                values.push(new_value);
            }
            class_data.push(ClassData {
                values,
                annotations: Vec::new(),
            });
        }
        self.b.set_object_data(new_id, class_data);
        Ok(())
    }

    /// The text of a `String` key or element, which decides its bucket.
    fn key_text(&self, value: Value) -> Result<String> {
        match self.old.opt_string(value) {
            Ok(Some(text)) => Ok(text.to_owned()),
            _ => Err(RoundTripError(
                "hash collections may only hold strings".to_owned(),
            )),
        }
    }

    fn load_factor(found: f32) -> Result<()> {
        if found.to_bits() == LOAD_FACTOR.to_bits() {
            Ok(())
        } else {
            Err(RoundTripError(format!("unsupported load factor {found}")))
        }
    }

    // Mirrors java.util.HashMap.readObject and writeObject (OpenJDK 25)
    fn hash_map(&mut self, id: NodeId) -> Result<()> {
        let old = self.old;
        let Value::Float(load_factor) = old.field(id, "loadFactor")? else {
            return Err(RoundTripError("malformed HashMap".to_owned()));
        };
        Self::load_factor(load_factor)?;
        let entries = jdk::map(old, Value::Ref(id))?.unwrap_or_default();
        let class = hash_map_desc(&mut self.b);
        let new_id = self.reserve(id, class);
        let mut model = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let text = self.key_text(key)?;
            let new_key = self.value(key)?;
            let new_value = self.value(value)?;
            model.push((text, new_key, new_value));
        }
        let map = JavaHashMap::deserialized(model);
        let (buckets, threshold) = map.write_params();
        let size =
            i32::try_from(map.len()).map_err(|_err| RoundTripError("map too large".to_owned()))?;
        let mut header = buckets.to_be_bytes().to_vec();
        header.extend_from_slice(&size.to_be_bytes());
        let mut annotations = block_data(&header);
        for entry in map.iter() {
            annotations.push(Content::Value(entry.key_payload));
            annotations.push(Content::Value(entry.value));
        }
        self.b.set_object_data(
            new_id,
            vec![ClassData {
                values: vec![Value::Float(LOAD_FACTOR), Value::Int(threshold)],
                annotations,
            }],
        );
        Ok(())
    }

    // Mirrors java.util.HashSet.readObject and writeObject (OpenJDK 25)
    fn hash_set(&mut self, id: NodeId) -> Result<()> {
        let old = self.old;
        let data = old.class_data_of(id, HASH_SET)?;
        let mut reader = AnnotationReader::new(&data.annotations, HASH_SET);
        let _capacity = reader.i32()?;
        Self::load_factor(reader.f32()?)?;
        let elements = jdk::set(old, Value::Ref(id))?.unwrap_or_default();
        let class = hash_set_desc(&mut self.b);
        let new_id = self.reserve(id, class);
        let mut set = deserialized_hash_set(elements.len());
        for element in elements {
            let text = self.key_text(element)?;
            let new_element = self.value(element)?;
            set.put(text, new_element, ());
        }
        let (capacity, _threshold) = set.write_params();
        let size =
            i32::try_from(set.len()).map_err(|_err| RoundTripError("set too large".to_owned()))?;
        let mut header = capacity.to_be_bytes().to_vec();
        header.extend_from_slice(&LOAD_FACTOR.to_bits().to_be_bytes());
        header.extend_from_slice(&size.to_be_bytes());
        let mut annotations = block_data(&header);
        annotations.extend(set.iter().map(|e| Content::Value(e.key_payload)));
        self.b.set_object_data(
            new_id,
            vec![ClassData {
                values: Vec::new(),
                annotations,
            }],
        );
        Ok(())
    }
}

fn unsupported(class: &str) -> RoundTripError {
    RoundTripError(format!("unsupported class {class}"))
}
