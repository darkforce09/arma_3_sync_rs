//! `javax.crypto.SealedObject`, the envelope of the Java client's `*.a3s.repository` files.

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{
    Array, ArrayData, ClassData, Content, FieldType, Graph, Node, Object, Value, flags,
};

use super::crypto::{self, CryptoError};
use super::error::{DeserializeError, Error};
use super::files::gzip;

/// Class name of the envelope.
pub const SEALED_OBJECT: &str = "javax.crypto.SealedObject";

/// `serialVersionUID` of `SealedObject` (confirmed by the JDK 25 descriptor dump). The reader
/// accepts any UID.
pub const SEALED_OBJECT_UID: i64 = 4_482_838_265_551_344_752;

/// Class name of `byte[]`.
pub const BYTE_ARRAY: &str = "[B";

/// `serialVersionUID` of `byte[]`.
pub const BYTE_ARRAY_UID: i64 = -5_984_413_125_824_719_648;

/// The seal algorithm Java writes (`Cipher.getInstance("AES").getAlgorithm()`).
const SEAL_ALG: &str = "AES";

/// Why [`open`] failed; turned into an [`Error`] with the file name by [`Self::with_file`].
#[derive(Debug, thiserror::Error)]
pub enum SealedError {
    /// The root object is not a `SealedObject`.
    #[error("expected {SEALED_OBJECT}, found {0}")]
    UnexpectedClass(String),

    /// The `SealedObject` is malformed.
    #[error(transparent)]
    Malformed(#[from] DeserializeError),

    /// The content cannot be decrypted.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

impl SealedError {
    /// The corresponding [`Error`] for `file`.
    pub fn with_file(self, file: &str) -> Error {
        let file = file.to_owned();
        match self {
            Self::UnexpectedClass(found) => Error::UnexpectedClass { file, found },
            Self::Malformed(source) => Error::Deserialize { file, source },
            Self::Crypto(source) => Error::Decrypt { file, source },
        }
    }
}

/// Decrypts the `SealedObject` at the root of `graph`, returning the inner (uncompressed)
/// serialization stream.
///
/// Lenient: any UID, fields looked up by name, unknown fields ignored. A non-null
/// `encodedParams` or a `sealAlg` other than `"AES"` only adds a warning, since the cipher is
/// fixed (Java ignores both when `getObject(Cipher)` is used).
// Mirrors dao/A3SFilesAccessor.java:94-95
pub fn open(graph: &Graph, warnings: &mut Vec<String>) -> Result<Vec<u8>, SealedError> {
    let root = graph.root_value().map_err(DeserializeError::Stream)?;
    let id = match root {
        Value::Ref(id) => id,
        Value::Null => return Err(SealedError::UnexpectedClass("null".to_owned())),
        _ => return Err(SealedError::UnexpectedClass("a primitive".to_owned())),
    };
    let is_sealed = match graph.node(id).map_err(DeserializeError::Stream)? {
        Node::Object(object) => graph
            .hierarchy(object.class)
            .map_err(DeserializeError::Stream)?
            .into_iter()
            .any(|desc| {
                graph
                    .class_desc(desc)
                    .is_ok_and(|d| d.name == SEALED_OBJECT)
            }),
        _ => false,
    };
    if !is_sealed {
        let found = match graph.node(id).map_err(DeserializeError::Stream)? {
            Node::String(_) => "java.lang.String".to_owned(),
            Node::ClassDesc(_) | Node::Class(_) => "java.lang.Class".to_owned(),
            _ => graph
                .class_name_of(id)
                .map_err(DeserializeError::Stream)?
                .to_owned(),
        };
        return Err(SealedError::UnexpectedClass(found));
    }

    let field = |name: &str| graph.opt_field(id, name).map_err(DeserializeError::Stream);
    if let Some(Value::Ref(_)) = field("encodedParams")? {
        warnings.push(format!("{SEALED_OBJECT}.encodedParams: not null, ignored"));
    }
    if let Some(value) = field("sealAlg")? {
        match graph.opt_string(value) {
            Ok(None) => {}
            Ok(Some(alg)) if alg.eq_ignore_ascii_case(SEAL_ALG) => {}
            Ok(Some(alg)) => warnings.push(format!(
                "{SEALED_OBJECT}.sealAlg: {alg:?} instead of {SEAL_ALG:?}, decrypting with {SEAL_ALG}"
            )),
            Err(_err) => warnings.push(format!("{SEALED_OBJECT}.sealAlg: not a string, ignored")),
        }
    }
    let content = match field("encryptedContent")? {
        Some(Value::Ref(array)) => match graph.node(array).map_err(DeserializeError::Stream)? {
            Node::Array(Array {
                data: ArrayData::Byte(bytes),
                ..
            }) => bytes,
            _ => return Err(DeserializeError::MissingEncryptedContent.into()),
        },
        _ => return Err(DeserializeError::MissingEncryptedContent.into()),
    };
    Ok(crypto::decrypt(content)?)
}

/// Produces `GZIP(ObjectOutputStream(SealedObject))` around `inner_stream`.
///
/// Like Java's `RepositoryDAO.write`: `encodedParams = null`,
/// `encryptedContent = AES(inner_stream)`, `paramsAlg = null`, `sealAlg = "AES"`.
///
/// For tests and cross-checks only; the Rust client never writes Java files.
// Mirrors dao/A3SFilesAccessor.java:164-186
pub fn seal(inner_stream: &[u8]) -> Result<Vec<u8>, a3s_jserial::Error> {
    let mut builder = Builder::new();
    let sealed_desc = builder.class_desc(
        SEALED_OBJECT,
        SEALED_OBJECT_UID,
        flags::SERIALIZABLE,
        &[
            // Byte arrays: type code `[`, signature `[B` (the second one a back-reference).
            FieldSpec {
                field_type: FieldType::Array,
                name: "encodedParams",
                signature: Some(BYTE_ARRAY),
            },
            FieldSpec {
                field_type: FieldType::Array,
                name: "encryptedContent",
                signature: Some(BYTE_ARRAY),
            },
            FieldSpec::object("paramsAlg", "Ljava/lang/String;"),
            FieldSpec::object("sealAlg", "Ljava/lang/String;"),
        ],
        None,
    );
    let byte_array_desc =
        builder.class_desc(BYTE_ARRAY, BYTE_ARRAY_UID, flags::SERIALIZABLE, &[], None);
    let seal_alg = builder.string(SEAL_ALG);
    let mut graph = builder.finish(Value::Null);
    let content = graph.push(Node::Array(Array {
        class: byte_array_desc,
        data: ArrayData::Byte(crypto::encrypt(inner_stream)),
    }));
    let sealed = graph.push(Node::Object(Object {
        class: sealed_desc,
        class_data: vec![ClassData {
            values: vec![Value::Null, Value::Ref(content), Value::Null, seal_alg],
            annotations: Vec::new(),
        }],
    }));
    graph.roots = vec![Content::Value(Value::Ref(sealed))];
    let raw = a3s_jserial::write(&graph)?;
    gzip(&raw).map_err(a3s_jserial::Error::Io)
}
