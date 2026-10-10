use crate::{Error, Result, flags};

/// Index of a [`Node`] in a [`Graph`].
///
/// For graphs produced by [`crate::read`], this equals the wire handle minus `0x7E0000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

impl NodeId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A field value, or an element of an object array.
///
/// Object-typed values are either [`Value::Null`] or a [`Value::Ref`] to a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Null,
    Ref(NodeId),
    Byte(i8),
    Char(u16),
    Double(f64),
    Float(f32),
    Int(i32),
    Long(i64),
    Short(i16),
    Boolean(bool),
}

impl Value {
    pub fn as_ref(self) -> Option<NodeId> {
        match self {
            Self::Ref(id) => Some(id),
            _ => None,
        }
    }
}

/// Type of a serializable field, as written in a class descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldType {
    Byte,
    Char,
    Double,
    Float,
    Int,
    Long,
    Short,
    Boolean,
    Array,
    Object,
}

impl FieldType {
    pub fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            b'B' => Self::Byte,
            b'C' => Self::Char,
            b'D' => Self::Double,
            b'F' => Self::Float,
            b'I' => Self::Int,
            b'J' => Self::Long,
            b'S' => Self::Short,
            b'Z' => Self::Boolean,
            b'[' => Self::Array,
            b'L' => Self::Object,
            other => return Err(Error::InvalidFieldType(other)),
        })
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Byte => b'B',
            Self::Char => b'C',
            Self::Double => b'D',
            Self::Float => b'F',
            Self::Int => b'I',
            Self::Long => b'J',
            Self::Short => b'S',
            Self::Boolean => b'Z',
            Self::Array => b'[',
            Self::Object => b'L',
        }
    }

    pub fn is_primitive(self) -> bool {
        !matches!(self, Self::Array | Self::Object)
    }
}

/// One field of a class descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDesc {
    pub field_type: FieldType,
    pub name: String,

    /// For object and array fields: a [`Node::String`] holding the JVM type descriptor,
    /// e.g. `Ljava/lang/String;`. Shared descriptors are written as back-references.
    pub class_name: Option<NodeId>,
}

/// An item of a class annotation or of `writeObject` data.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// One block-data chunk, kept as written so streams round-trip exactly.
    BlockData(Vec<u8>),

    /// An object (or null) written with `writeObject`.
    Value(Value),
}

/// A class descriptor (`TC_CLASSDESC`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClassDesc {
    pub name: String,
    pub serial_version_uid: i64,
    pub flags: u8,
    pub fields: Vec<FieldDesc>,

    /// Class annotation (normally empty).
    pub annotations: Vec<Content>,
    pub super_class: Option<NodeId>,
}

impl ClassDesc {
    pub fn has_write_method(&self) -> bool {
        self.flags & flags::WRITE_METHOD != 0
    }

    pub fn is_serializable(&self) -> bool {
        self.flags & flags::SERIALIZABLE != 0
    }

    pub fn is_externalizable(&self) -> bool {
        self.flags & flags::EXTERNALIZABLE != 0
    }

    pub fn is_enum(&self) -> bool {
        self.flags & flags::ENUM != 0
    }
}

/// Serialized data of one class in an object's hierarchy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassData {
    /// Field values, index-aligned with [`ClassDesc::fields`].
    pub values: Vec<Value>,

    /// Data written by the class's `writeObject` method (if it has one), or external data.
    pub annotations: Vec<Content>,
}

/// An object instance (`TC_OBJECT`).
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub class: NodeId,

    /// One entry per class of the hierarchy, top-most superclass first.
    pub class_data: Vec<ClassData>,
}

/// An enum constant (`TC_ENUM`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumConst {
    pub class: NodeId,

    /// [`Node::String`] holding the constant name.
    pub constant: NodeId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArrayData {
    Byte(Vec<u8>),
    Char(Vec<u16>),
    Double(Vec<f64>),
    Float(Vec<f32>),
    Int(Vec<i32>),
    Long(Vec<i64>),
    Short(Vec<i16>),
    Boolean(Vec<bool>),
    Object(Vec<Value>),
}

/// An array (`TC_ARRAY`).
#[derive(Clone, Debug, PartialEq)]
pub struct Array {
    pub class: NodeId,
    pub data: ArrayData,
}

/// A handle-bearing item of a stream.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    ClassDesc(ClassDesc),
    Object(Object),
    String(String),
    Enum(EnumConst),
    Array(Array),

    /// A `java.lang.Class` instance, referring to its class descriptor.
    Class(NodeId),
}

/// An arena of stream items plus the top-level contents of the stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,

    /// Top-level contents, in order. `ObjectOutputStream.writeObject(x)` produces one
    /// [`Content::Value`]; `writeInt` etc. produce [`Content::BlockData`].
    pub roots: Vec<Content>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, node: Node) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("graph too large"));
        self.nodes.push(node);
        id
    }

    pub fn node(&self, id: NodeId) -> Result<&Node> {
        self.nodes.get(id.index()).ok_or(Error::InvalidHandle(id.0))
    }

    pub fn node_mut(&mut self, id: NodeId) -> Result<&mut Node> {
        self.nodes
            .get_mut(id.index())
            .ok_or(Error::InvalidHandle(id.0))
    }

    /// The single object written at the top level, if the stream contains exactly one.
    pub fn root_value(&self) -> Result<Value> {
        match self.roots.as_slice() {
            [Content::Value(v)] => Ok(*v),
            _ => Err(Error::Unsupported(
                "stream does not contain exactly one object",
            )),
        }
    }

    pub fn class_desc(&self, id: NodeId) -> Result<&ClassDesc> {
        match self.node(id)? {
            Node::ClassDesc(desc) => Ok(desc),
            _ => Err(Error::WrongNodeKind(id, "class descriptor")),
        }
    }

    pub fn object(&self, id: NodeId) -> Result<&Object> {
        match self.node(id)? {
            Node::Object(obj) => Ok(obj),
            _ => Err(Error::WrongNodeKind(id, "object")),
        }
    }

    pub fn object_mut(&mut self, id: NodeId) -> Result<&mut Object> {
        match self.node_mut(id)? {
            Node::Object(obj) => Ok(obj),
            _ => Err(Error::WrongNodeKind(id, "object")),
        }
    }

    pub fn string(&self, id: NodeId) -> Result<&str> {
        match self.node(id)? {
            Node::String(s) => Ok(s),
            _ => Err(Error::WrongNodeKind(id, "string")),
        }
    }

    /// Interprets `value` as a nullable string.
    pub fn opt_string(&self, value: Value) -> Result<Option<&str>> {
        match value {
            Value::Null => Ok(None),
            Value::Ref(id) => self.string(id).map(Some),
            _ => Err(Error::Unsupported("primitive where a string was expected")),
        }
    }

    /// Fully-qualified class name of the object or enum constant `id`.
    pub fn class_name_of(&self, id: NodeId) -> Result<&str> {
        let class = match self.node(id)? {
            Node::Object(obj) => obj.class,
            Node::Enum(e) => e.class,
            Node::Array(a) => a.class,
            _ => return Err(Error::WrongNodeKind(id, "object, enum or array")),
        };
        Ok(&self.class_desc(class)?.name)
    }

    /// Class descriptors of the hierarchy of `class`, top-most superclass first.
    pub fn hierarchy(&self, class: NodeId) -> Result<Vec<NodeId>> {
        let mut chain = Vec::new();
        let mut cur = Some(class);
        while let Some(id) = cur {
            if chain.len() > 64 {
                return Err(Error::LimitExceeded("class hierarchy depth"));
            }
            chain.push(id);
            cur = self.class_desc(id)?.super_class;
        }
        chain.reverse();
        Ok(chain)
    }

    /// Looks up a serializable field by name, searching from the most-derived class upwards.
    pub fn field(&self, object: NodeId, name: &str) -> Result<Value> {
        let obj = self.object(object)?;
        let hierarchy = self.hierarchy(obj.class)?;
        for (desc_id, data) in hierarchy.iter().zip(&obj.class_data).rev() {
            let desc = self.class_desc(*desc_id)?;
            if let Some(i) = desc.fields.iter().position(|f| f.name == name) {
                return data
                    .values
                    .get(i)
                    .copied()
                    .ok_or_else(|| Error::MalformedData {
                        class: desc.name.clone(),
                        reason: "missing field value",
                    });
            }
        }
        Err(Error::MissingField {
            class: self.class_desc(obj.class)?.name.clone(),
            field: name.to_owned(),
        })
    }

    /// Like [`Self::field`], but returns `None` if the class has no such field.
    ///
    /// Useful for fields that were added in later versions of a class.
    pub fn opt_field(&self, object: NodeId, name: &str) -> Result<Option<Value>> {
        match self.field(object, name) {
            Ok(v) => Ok(Some(v)),
            Err(Error::MissingField { .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Writer-side helper: data of `class_name` in the hierarchy of `object`.
    pub fn class_data_of(&self, object: NodeId, class_name: &str) -> Result<&ClassData> {
        let obj = self.object(object)?;
        let hierarchy = self.hierarchy(obj.class)?;
        for (desc_id, data) in hierarchy.iter().zip(&obj.class_data) {
            if self.class_desc(*desc_id)?.name == class_name {
                return Ok(data);
            }
        }
        Err(Error::WrongClass {
            expected: class_name.to_owned(),
            found: self.class_desc(obj.class)?.name.clone(),
        })
    }

    fn wrong_field_type(&self, object: NodeId, field: &str) -> Error {
        let class = self
            .object(object)
            .and_then(|o| self.class_desc(o.class))
            .map(|d| d.name.clone())
            .unwrap_or_default();
        Error::WrongFieldType {
            class,
            field: field.to_owned(),
        }
    }

    pub fn field_bool(&self, object: NodeId, name: &str) -> Result<bool> {
        match self.field(object, name)? {
            Value::Boolean(b) => Ok(b),
            _ => Err(self.wrong_field_type(object, name)),
        }
    }

    pub fn field_i32(&self, object: NodeId, name: &str) -> Result<i32> {
        match self.field(object, name)? {
            Value::Int(v) => Ok(v),
            _ => Err(self.wrong_field_type(object, name)),
        }
    }

    pub fn field_i64(&self, object: NodeId, name: &str) -> Result<i64> {
        match self.field(object, name)? {
            Value::Long(v) => Ok(v),
            _ => Err(self.wrong_field_type(object, name)),
        }
    }

    pub fn field_f64(&self, object: NodeId, name: &str) -> Result<f64> {
        match self.field(object, name)? {
            Value::Double(v) => Ok(v),
            _ => Err(self.wrong_field_type(object, name)),
        }
    }

    pub fn field_string(&self, object: NodeId, name: &str) -> Result<Option<String>> {
        let value = self.field(object, name)?;
        self.opt_string(value).map(|s| s.map(str::to_owned))
    }

    /// Name of the enum constant referenced by `value`, if not null.
    pub fn enum_name(&self, value: Value) -> Result<Option<&str>> {
        match value {
            Value::Null => Ok(None),
            Value::Ref(id) => match self.node(id)? {
                Node::Enum(e) => self.string(e.constant).map(Some),
                _ => Err(Error::WrongNodeKind(id, "enum constant")),
            },
            _ => Err(Error::Unsupported("primitive where an enum was expected")),
        }
    }
}
