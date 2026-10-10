use crate::NodeId;

/// Errors produced while reading, writing or interpreting a serialization stream.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unexpected end of stream")]
    UnexpectedEof,

    #[error("bad stream header {magic:#06x}/{version}")]
    BadHeader { magic: u16, version: u16 },

    #[error("unexpected type code {code:#04x} at offset {offset} (expected {expected})")]
    UnexpectedTypeCode {
        code: u8,
        offset: usize,
        expected: &'static str,
    },

    #[error("unsupported stream feature: {0}")]
    Unsupported(&'static str),

    #[error("invalid handle {0:#x}")]
    InvalidHandle(u32),

    #[error("invalid field type code {0:#04x}")]
    InvalidFieldType(u8),

    #[error("invalid modified UTF-8 string")]
    InvalidUtf8,

    #[error("string too long for writeUTF ({0} bytes)")]
    StringTooLong(usize),

    #[error("stream exceeds limit: {0}")]
    LimitExceeded(&'static str),

    #[error("node {0:?} has the wrong kind: expected {1}")]
    WrongNodeKind(NodeId, &'static str),

    #[error("expected class {expected}, found {found}")]
    WrongClass { expected: String, found: String },

    #[error("field `{field}` not found in class {class}")]
    MissingField { class: String, field: String },

    #[error("field `{field}` of class {class} has an unexpected type")]
    WrongFieldType { class: String, field: String },

    #[error("unexpected null")]
    UnexpectedNull,

    #[error("malformed {class} data: {reason}")]
    MalformedData { class: String, reason: &'static str },

    #[error("unsupported collection class {0}")]
    UnsupportedCollection(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
