use crate::constants::{BASE_WIRE_HANDLE, STREAM_MAGIC, STREAM_VERSION};
use crate::model::{ArrayData, ClassDesc, Content, Graph, Node, NodeId, Value};
use crate::{Error, Result, mutf8, tc};

/// Serializes `graph` (header plus [`Graph::roots`]).
///
/// Nodes are emitted the first time they are referenced and become back-references afterwards,
/// so sharing in the graph is preserved. Handles are assigned in emission order, exactly as
/// `ObjectOutputStream` does.
pub fn write(graph: &Graph) -> Result<Vec<u8>> {
    let mut writer = Writer {
        graph,
        out: Vec::with_capacity(4096),
        handles: vec![None; graph.nodes.len()],
        next_handle: 0,
    };
    writer.out.extend_from_slice(&STREAM_MAGIC.to_be_bytes());
    writer.out.extend_from_slice(&STREAM_VERSION.to_be_bytes());
    for content in &graph.roots {
        writer.content(content)?;
    }
    Ok(writer.out)
}

struct Writer<'a> {
    graph: &'a Graph,
    out: Vec<u8>,
    handles: Vec<Option<u32>>,
    next_handle: u32,
}

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }

    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }

    fn i64(&mut self, v: i64) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }

    fn utf(&mut self, s: &str) -> Result<()> {
        let bytes = mutf8::encode(s);
        let len = u16::try_from(bytes.len()).map_err(|_err| Error::StringTooLong(bytes.len()))?;
        self.u16(len);
        self.out.extend_from_slice(&bytes);
        Ok(())
    }

    fn assign_handle(&mut self, id: NodeId) {
        self.handles[id.index()] = Some(self.next_handle);
        self.next_handle += 1;
    }

    fn content(&mut self, content: &Content) -> Result<()> {
        match content {
            Content::BlockData(bytes) => {
                if let Ok(len) = u8::try_from(bytes.len()) {
                    self.u8(tc::BLOCKDATA);
                    self.u8(len);
                } else {
                    let len = i32::try_from(bytes.len())
                        .map_err(|_err| Error::LimitExceeded("block data length"))?;
                    self.u8(tc::BLOCKDATALONG);
                    self.i32(len);
                }
                self.out.extend_from_slice(bytes);
                Ok(())
            }
            Content::Value(value) => self.object(*value),
        }
    }

    fn annotations(&mut self, contents: &[Content]) -> Result<()> {
        for content in contents {
            self.content(content)?;
        }
        self.u8(tc::ENDBLOCKDATA);
        Ok(())
    }

    fn object(&mut self, value: Value) -> Result<()> {
        match value {
            Value::Null => {
                self.u8(tc::NULL);
                Ok(())
            }
            Value::Ref(id) => self.node(id),
            _ => Err(Error::Unsupported("primitive value in object position")),
        }
    }

    fn node(&mut self, id: NodeId) -> Result<()> {
        let slot = self
            .handles
            .get(id.index())
            .ok_or(Error::InvalidHandle(id.0))?;
        if let Some(handle) = *slot {
            self.u8(tc::REFERENCE);
            self.i32((BASE_WIRE_HANDLE + handle) as i32);
            return Ok(());
        }

        let graph = self.graph;
        match graph.node(id)? {
            Node::ClassDesc(desc) => self.class_desc(id, desc),
            Node::String(s) => {
                let bytes = mutf8::encode(s);
                if let Ok(len) = u16::try_from(bytes.len()) {
                    self.u8(tc::STRING);
                    self.assign_handle(id);
                    self.u16(len);
                } else {
                    self.u8(tc::LONGSTRING);
                    self.assign_handle(id);
                    self.i64(bytes.len() as i64);
                }
                self.out.extend_from_slice(&bytes);
                Ok(())
            }
            Node::Enum(e) => {
                self.u8(tc::ENUM);
                self.class_desc_ref(Some(e.class))?;
                self.assign_handle(id);
                self.node(e.constant)
            }
            Node::Class(desc) => {
                self.u8(tc::CLASS);
                self.class_desc_ref(Some(*desc))?;
                self.assign_handle(id);
                Ok(())
            }
            Node::Array(array) => {
                self.u8(tc::ARRAY);
                self.class_desc_ref(Some(array.class))?;
                self.assign_handle(id);
                self.array_data(&array.data)
            }
            Node::Object(obj) => {
                self.u8(tc::OBJECT);
                self.class_desc_ref(Some(obj.class))?;
                self.assign_handle(id);
                let hierarchy = graph.hierarchy(obj.class)?;
                if hierarchy.len() != obj.class_data.len() {
                    return Err(Error::MalformedData {
                        class: graph.class_desc(obj.class)?.name.clone(),
                        reason: "class data does not match the class hierarchy",
                    });
                }
                for (desc_id, data) in hierarchy.iter().zip(&obj.class_data) {
                    let desc = graph.class_desc(*desc_id)?;
                    if desc.is_externalizable() {
                        self.annotations(&data.annotations)?;
                    } else if desc.is_serializable() {
                        if data.values.len() != desc.fields.len() {
                            return Err(Error::MalformedData {
                                class: desc.name.clone(),
                                reason: "field count mismatch",
                            });
                        }
                        for value in &data.values {
                            self.field_value(*value)?;
                        }
                        if desc.has_write_method() {
                            self.annotations(&data.annotations)?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn class_desc_ref(&mut self, id: Option<NodeId>) -> Result<()> {
        match id {
            None => {
                self.u8(tc::NULL);
                Ok(())
            }
            Some(id) => {
                self.graph.class_desc(id)?;
                self.node(id)
            }
        }
    }

    fn class_desc(&mut self, id: NodeId, desc: &ClassDesc) -> Result<()> {
        self.u8(tc::CLASSDESC);
        self.utf(&desc.name)?;
        self.i64(desc.serial_version_uid);
        self.assign_handle(id);
        self.u8(desc.flags);
        let count = u16::try_from(desc.fields.len())
            .map_err(|_err| Error::LimitExceeded("number of fields"))?;
        self.u16(count);
        for field in &desc.fields {
            self.u8(field.field_type.code());
            self.utf(&field.name)?;
            if !field.field_type.is_primitive() {
                let class_name = field.class_name.ok_or(Error::MalformedData {
                    class: desc.name.clone(),
                    reason: "object field without type name",
                })?;
                self.graph.string(class_name)?;
                self.node(class_name)?;
            }
        }
        self.annotations(&desc.annotations)?;
        self.class_desc_ref(desc.super_class)
    }

    fn field_value(&mut self, value: Value) -> Result<()> {
        match value {
            Value::Null | Value::Ref(_) => self.object(value)?,
            Value::Byte(v) => self.u8(v as u8),
            Value::Char(v) => self.u16(v),
            Value::Double(v) => self.i64(v.to_bits() as i64),
            Value::Float(v) => self.i32(v.to_bits() as i32),
            Value::Int(v) => self.i32(v),
            Value::Long(v) => self.i64(v),
            Value::Short(v) => self.u16(v as u16),
            Value::Boolean(v) => self.u8(u8::from(v)),
        }
        Ok(())
    }

    fn array_data(&mut self, data: &ArrayData) -> Result<()> {
        let len = |n: usize| i32::try_from(n).map_err(|_err| Error::LimitExceeded("array length"));
        match data {
            ArrayData::Byte(v) => {
                self.i32(len(v.len())?);
                self.out.extend_from_slice(v);
            }
            ArrayData::Char(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.u16(x);
                }
            }
            ArrayData::Double(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.i64(x.to_bits() as i64);
                }
            }
            ArrayData::Float(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.i32(x.to_bits() as i32);
                }
            }
            ArrayData::Int(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.i32(x);
                }
            }
            ArrayData::Long(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.i64(x);
                }
            }
            ArrayData::Short(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.u16(x as u16);
                }
            }
            ArrayData::Boolean(v) => {
                self.i32(len(v.len())?);
                for &x in v {
                    self.u8(u8::from(x));
                }
            }
            ArrayData::Object(v) => {
                self.i32(len(v.len())?);
                for value in v {
                    self.object(*value)?;
                }
            }
        }
        Ok(())
    }
}
