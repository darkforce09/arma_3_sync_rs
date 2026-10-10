use crate::constants::{BASE_WIRE_HANDLE, STREAM_MAGIC, STREAM_VERSION};
use crate::model::{
    Array, ArrayData, ClassData, ClassDesc, Content, EnumConst, FieldDesc, FieldType, Graph, Node,
    NodeId, Object, Value,
};
use crate::{Error, Result, mutf8, tc};

/// Resource limits applied while reading untrusted streams.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum nesting depth of objects.
    pub max_depth: usize,

    /// Maximum number of handle-bearing items.
    pub max_nodes: usize,

    /// Maximum number of elements in a single array.
    pub max_array_len: usize,
}

impl Limits {
    /// Limits enforced by Arma3Sync's Java reader (`A3SFilesAccessor.LEGACY_OBJECT_FILTER`).
    ///
    /// Anything written for Java clients must stay within these.
    pub const JAVA_A3S: Self = Self {
        max_depth: 100,
        max_nodes: 250_000,
        max_array_len: 256 * 1024 * 1024,
    };
}

impl Default for Limits {
    /// Generous limits for reading: we accept everything Java accepts, and more.
    fn default() -> Self {
        Self {
            max_depth: 1_000,
            max_nodes: 10_000_000,
            max_array_len: 512 * 1024 * 1024,
        }
    }
}

/// Parses a complete serialization stream with [`Limits::default`].
pub fn read(bytes: &[u8]) -> Result<Graph> {
    read_with_limits(bytes, Limits::default())
}

/// Parses a complete serialization stream.
pub fn read_with_limits(bytes: &[u8], limits: Limits) -> Result<Graph> {
    let mut reader = Reader {
        bytes,
        pos: 0,
        graph: Graph::new(),
        limits,
        depth: 0,
    };
    let magic = reader.u16()?;
    let version = reader.u16()?;
    if magic != STREAM_MAGIC || version != STREAM_VERSION {
        return Err(Error::BadHeader { magic, version });
    }
    while reader.pos < bytes.len() {
        let content = reader.content()?;
        reader.graph.roots.push(content);
    }
    Ok(reader.graph)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    graph: Graph,
    limits: Limits,
    depth: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::UnexpectedEof)?;
        let slice = self.bytes.get(self.pos..end).ok_or(Error::UnexpectedEof)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("length checked"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn peek(&self) -> Result<u8> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or(Error::UnexpectedEof)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }

    fn utf(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        mutf8::decode(self.take(len)?)
    }

    fn long_utf(&mut self) -> Result<String> {
        let len = usize::try_from(self.i64()?).map_err(|_err| Error::UnexpectedEof)?;
        mutf8::decode(self.take(len)?)
    }

    fn unexpected(&self, code: u8, expected: &'static str) -> Error {
        Error::UnexpectedTypeCode {
            code,
            offset: self.pos.saturating_sub(1),
            expected,
        }
    }

    /// Allocates the next handle with a placeholder node, to be filled in later.
    fn new_handle(&mut self) -> Result<NodeId> {
        if self.graph.nodes.len() >= self.limits.max_nodes {
            return Err(Error::LimitExceeded("number of handles"));
        }
        Ok(self.graph.push(Node::String(String::new())))
    }

    fn set(&mut self, id: NodeId, node: Node) {
        self.graph.nodes[id.index()] = node;
    }

    fn reference(&mut self) -> Result<NodeId> {
        let handle = self.i32()? as u32;
        let index = handle
            .checked_sub(BASE_WIRE_HANDLE)
            .ok_or(Error::InvalidHandle(handle))?;
        if index as usize >= self.graph.nodes.len() {
            return Err(Error::InvalidHandle(handle));
        }
        Ok(NodeId(index))
    }

    /// Reads one `content` item: block data or an object.
    fn content(&mut self) -> Result<Content> {
        match self.peek()? {
            tc::BLOCKDATA => {
                self.pos += 1;
                let len = self.u8()? as usize;
                Ok(Content::BlockData(self.take(len)?.to_vec()))
            }
            tc::BLOCKDATALONG => {
                self.pos += 1;
                let len = usize::try_from(self.i32()?).map_err(|_err| Error::UnexpectedEof)?;
                Ok(Content::BlockData(self.take(len)?.to_vec()))
            }
            _ => Ok(Content::Value(self.object()?)),
        }
    }

    /// Reads an `object` production: any object-ish item or null.
    fn object(&mut self) -> Result<Value> {
        self.depth += 1;
        if self.depth > self.limits.max_depth {
            return Err(Error::LimitExceeded("nesting depth"));
        }
        let result = self.object_inner();
        self.depth -= 1;
        result
    }

    fn object_inner(&mut self) -> Result<Value> {
        let code = self.u8()?;
        let id = match code {
            tc::NULL => return Ok(Value::Null),
            tc::REFERENCE => self.reference()?,
            tc::OBJECT => self.new_object()?,
            tc::STRING => {
                let id = self.new_handle()?;
                let s = self.utf()?;
                self.set(id, Node::String(s));
                id
            }
            tc::LONGSTRING => {
                let id = self.new_handle()?;
                let s = self.long_utf()?;
                self.set(id, Node::String(s));
                id
            }
            tc::ENUM => {
                let class = self.class_desc()?.ok_or(Error::UnexpectedNull)?;
                let id = self.new_handle()?;
                let constant = match self.object()? {
                    Value::Ref(c) if matches!(self.graph.node(c)?, Node::String(_)) => c,
                    _ => return Err(Error::Unsupported("enum constant name is not a string")),
                };
                self.set(id, Node::Enum(EnumConst { class, constant }));
                id
            }
            tc::ARRAY => self.new_array()?,
            tc::CLASS => {
                let desc = self.class_desc()?.ok_or(Error::UnexpectedNull)?;
                let id = self.new_handle()?;
                self.set(id, Node::Class(desc));
                id
            }
            tc::CLASSDESC => self.new_class_desc()?,
            tc::PROXYCLASSDESC => return Err(Error::Unsupported("proxy class descriptors")),
            tc::RESET => return Err(Error::Unsupported("TC_RESET")),
            tc::EXCEPTION => return Err(Error::Unsupported("serialized exceptions")),
            other => return Err(self.unexpected(other, "object")),
        };
        Ok(Value::Ref(id))
    }

    /// Reads a `classDesc` production: a new descriptor, a reference to one, or null.
    fn class_desc(&mut self) -> Result<Option<NodeId>> {
        let code = self.u8()?;
        match code {
            tc::NULL => Ok(None),
            tc::CLASSDESC => self.new_class_desc().map(Some),
            tc::REFERENCE => {
                let id = self.reference()?;
                self.graph.class_desc(id)?;
                Ok(Some(id))
            }
            tc::PROXYCLASSDESC => Err(Error::Unsupported("proxy class descriptors")),
            other => Err(self.unexpected(other, "class descriptor")),
        }
    }

    fn new_class_desc(&mut self) -> Result<NodeId> {
        let name = self.utf()?;
        let serial_version_uid = self.i64()?;
        let id = self.new_handle()?;
        let flags = self.u8()?;
        let field_count = self.u16()?;
        let mut fields = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            let field_type = FieldType::from_code(self.u8()?)?;
            let field_name = self.utf()?;
            let class_name = if field_type.is_primitive() {
                None
            } else {
                match self.object()? {
                    Value::Ref(s) if matches!(self.graph.node(s)?, Node::String(_)) => Some(s),
                    _ => return Err(Error::Unsupported("field type name is not a string")),
                }
            };
            fields.push(FieldDesc {
                field_type,
                name: field_name,
                class_name,
            });
        }
        let annotations = self.annotations()?;
        let super_class = self.class_desc()?;
        self.set(
            id,
            Node::ClassDesc(ClassDesc {
                name,
                serial_version_uid,
                flags,
                fields,
                annotations,
                super_class,
            }),
        );
        Ok(id)
    }

    /// Reads contents up to and including `TC_ENDBLOCKDATA`.
    fn annotations(&mut self) -> Result<Vec<Content>> {
        let mut out = Vec::new();
        loop {
            if self.peek()? == tc::ENDBLOCKDATA {
                self.pos += 1;
                return Ok(out);
            }
            out.push(self.content()?);
        }
    }

    fn new_object(&mut self) -> Result<NodeId> {
        let class = self.class_desc()?.ok_or(Error::UnexpectedNull)?;
        let id = self.new_handle()?;
        // Store a partial object first so that self-references resolve to an object.
        self.set(
            id,
            Node::Object(Object {
                class,
                class_data: Vec::new(),
            }),
        );
        let hierarchy = self.graph.hierarchy(class)?;
        let mut class_data = Vec::with_capacity(hierarchy.len());
        for desc_id in hierarchy {
            let desc = self.graph.class_desc(desc_id)?.clone();
            let mut data = ClassData::default();
            if desc.is_externalizable() {
                if desc.flags & crate::flags::BLOCK_DATA == 0 {
                    return Err(Error::Unsupported(
                        "externalizable data in protocol version 1",
                    ));
                }
                data.annotations = self.annotations()?;
            } else if desc.is_serializable() {
                data.values.reserve(desc.fields.len());
                for field in &desc.fields {
                    let value = self.field_value(field.field_type)?;
                    data.values.push(value);
                }
                if desc.has_write_method() {
                    data.annotations = self.annotations()?;
                }
            }
            class_data.push(data);
        }
        if let Node::Object(obj) = &mut self.graph.nodes[id.index()] {
            obj.class_data = class_data;
        }
        Ok(id)
    }

    fn field_value(&mut self, field_type: FieldType) -> Result<Value> {
        Ok(match field_type {
            FieldType::Byte => Value::Byte(self.u8()? as i8),
            FieldType::Char => Value::Char(self.u16()?),
            FieldType::Double => Value::Double(f64::from_bits(self.i64()? as u64)),
            FieldType::Float => Value::Float(f32::from_bits(self.i32()? as u32)),
            FieldType::Int => Value::Int(self.i32()?),
            FieldType::Long => Value::Long(self.i64()?),
            FieldType::Short => Value::Short(self.u16()? as i16),
            FieldType::Boolean => Value::Boolean(self.u8()? != 0),
            FieldType::Array | FieldType::Object => self.object()?,
        })
    }

    fn new_array(&mut self) -> Result<NodeId> {
        let class = self.class_desc()?.ok_or(Error::UnexpectedNull)?;
        let id = self.new_handle()?;
        let len = usize::try_from(self.i32()?).map_err(|_err| Error::UnexpectedEof)?;
        if len > self.limits.max_array_len {
            return Err(Error::LimitExceeded("array length"));
        }
        let element = self
            .graph
            .class_desc(class)?
            .name
            .as_bytes()
            .get(1)
            .copied()
            .ok_or(Error::Unsupported("array class name"))?;
        // Don't trust `len` for pre-allocation beyond what the input can hold.
        let cap = len.min(self.bytes.len() - self.pos);
        let data = match FieldType::from_code(element)? {
            FieldType::Byte => ArrayData::Byte(self.take(len)?.to_vec()),
            FieldType::Char => ArrayData::Char(self.collect(len, cap, |r| r.u16())?),
            FieldType::Double => {
                ArrayData::Double(self.collect(len, cap, |r| Ok(f64::from_bits(r.i64()? as u64)))?)
            }
            FieldType::Float => {
                ArrayData::Float(self.collect(len, cap, |r| Ok(f32::from_bits(r.i32()? as u32)))?)
            }
            FieldType::Int => ArrayData::Int(self.collect(len, cap, |r| r.i32())?),
            FieldType::Long => ArrayData::Long(self.collect(len, cap, |r| r.i64())?),
            FieldType::Short => {
                ArrayData::Short(self.collect(len, cap, |r| Ok(r.u16()? as i16))?)
            }
            FieldType::Boolean => ArrayData::Boolean(self.collect(len, cap, |r| Ok(r.u8()? != 0))?),
            FieldType::Array | FieldType::Object => {
                ArrayData::Object(self.collect(len, cap, |r| r.object())?)
            }
        };
        self.set(id, Node::Array(Array { class, data }));
        Ok(id)
    }

    fn collect<T>(
        &mut self,
        len: usize,
        cap: usize,
        mut f: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut out = Vec::with_capacity(cap);
        for _ in 0..len {
            out.push(f(self)?);
        }
        Ok(out)
    }
}
