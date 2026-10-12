//! AES cipher and `SealedObject` envelope of `*.a3s.repository`.

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{
    Array, ArrayData, ClassData, Content, FieldType, Graph, Node, Object, Value, flags,
};
use a3s_repo_files::legacy::crypto::{CryptoError, JAVA_AES_KEY, decrypt, encrypt};
use a3s_repo_files::legacy::sealed::{self, SEALED_OBJECT, SEALED_OBJECT_UID, SealedError};
use a3s_repo_files::legacy::{Error, read_object_file};

fn hex(bytes: &[u8]) -> String {
    a3s_repo_files::hash::to_hex(bytes)
}

// ---------------------------------------------------------------------------------------------

#[test]
fn key_and_vectors() {
    assert_eq!(
        JAVA_AES_KEY,
        [
            0x01, 0x72, 0x43, 0x3E, 0x1C, 0x7A, 0x55, 0, 0x01, 0x72, 0x43, 0x3E, 0x1C, 0x7A, 0x55,
            0x4F
        ]
    );
    // Computed with `openssl enc -aes-128-ecb -K 0172433E1C7A55000172433E1C7A554F`.
    assert_eq!(hex(&encrypt(b"hello")), "722efc9890ba35cbe048931ba220cca1");
    let full = encrypt(b"0123456789abcdef");
    assert_eq!(full.len(), 32);
    assert_eq!(
        hex(&full),
        "9133552568478c0cf574c481b6b2fd28b2d90a3d56164689a84a57cd6b6bfe25"
    );
}

#[test]
fn round_trips() {
    for len in [0, 1, 15, 16, 17, 1000] {
        let plain: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
        let cipher = encrypt(&plain);
        assert_eq!(cipher.len(), (len / 16 + 1) * 16, "{len}");
        assert_eq!(decrypt(&cipher).unwrap(), plain, "{len}");
    }
}

#[test]
fn decrypt_errors() {
    assert_eq!(decrypt(&[0; 15]), Err(CryptoError::BadLength(15)));
    assert_eq!(decrypt(&[]), Err(CryptoError::BadLength(0)));
    let mut cipher = encrypt(b"some repository bytes");
    let last = cipher.len() - 1;
    cipher[last] ^= 0x55;
    assert_eq!(decrypt(&cipher), Err(CryptoError::BadPadding));
}

#[test]
fn seal_then_open() {
    let inner = b"\xac\xed\x00\x05inner stream bytes".to_vec();
    let sealed = sealed::seal(&inner).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.a3s.repository");
    std::fs::write(&path, &sealed).unwrap();

    let graph = read_object_file(&path).unwrap();
    let mut warnings = Vec::new();
    assert_eq!(sealed::open(&graph, &mut warnings).unwrap(), inner);
    assert!(warnings.is_empty(), "{warnings:?}");

    let root = graph.root_value().unwrap().as_ref().unwrap();
    let Node::Object(object) = graph.node(root).unwrap() else {
        panic!("root is not an object");
    };
    let desc = graph.class_desc(object.class).unwrap();
    assert_eq!(desc.name, SEALED_OBJECT);
    assert_eq!(desc.serial_version_uid, SEALED_OBJECT_UID);
    assert_eq!(desc.serial_version_uid, 4_482_838_265_551_344_752);
    assert_eq!(desc.flags, flags::SERIALIZABLE);
    assert_eq!(desc.super_class, None);
    let fields: Vec<(FieldType, &str, &str)> = desc
        .fields
        .iter()
        .map(|f| {
            (
                f.field_type,
                f.name.as_str(),
                graph.string(f.class_name.unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            (FieldType::Array, "encodedParams", "[B"),
            (FieldType::Array, "encryptedContent", "[B"),
            (FieldType::Object, "paramsAlg", "Ljava/lang/String;"),
            (FieldType::Object, "sealAlg", "Ljava/lang/String;"),
        ]
    );
    // The second `[B` is a back-reference to the first.
    assert_eq!(desc.fields[0].class_name, desc.fields[1].class_name);
    assert_eq!(graph.field(root, "encodedParams").unwrap(), Value::Null);
    assert_eq!(graph.field(root, "paramsAlg").unwrap(), Value::Null);
    assert_eq!(
        graph.field_string(root, "sealAlg").unwrap().as_deref(),
        Some("AES")
    );
}

// ---------------------------------------------------------------------------------------------

/// Builds a `SealedObject`-like graph around `content`.
fn sealed_graph(uid: i64, seal_alg: &str, extra_field: bool, content: Option<&[u8]>) -> Graph {
    let mut b = Builder::new();
    let mut fields = vec![
        FieldSpec {
            field_type: FieldType::Array,
            name: "encodedParams",
            signature: Some("[B"),
        },
        FieldSpec {
            field_type: FieldType::Array,
            name: "encryptedContent",
            signature: Some("[B"),
        },
        FieldSpec::object("paramsAlg", "Ljava/lang/String;"),
        FieldSpec::object("sealAlg", "Ljava/lang/String;"),
    ];
    if extra_field {
        fields.push(FieldSpec::object("zFuture", "Ljava/lang/String;"));
    }
    let desc = b.class_desc(SEALED_OBJECT, uid, flags::SERIALIZABLE, &fields, None);
    let byte_array = b.class_desc("[B", sealed::BYTE_ARRAY_UID, flags::SERIALIZABLE, &[], None);
    let alg = b.string(seal_alg);
    let future = b.string("future");
    let mut graph = b.finish(Value::Null);
    let content = content.map_or(Value::Null, |bytes| {
        Value::Ref(graph.push(Node::Array(Array {
            class: byte_array,
            data: ArrayData::Byte(bytes.to_vec()),
        })))
    });
    let mut values = vec![Value::Null, content, Value::Null, alg];
    if extra_field {
        values.push(future);
    }
    let root = graph.push(Node::Object(Object {
        class: desc,
        class_data: vec![ClassData {
            values,
            annotations: Vec::new(),
        }],
    }));
    graph.roots = vec![Content::Value(Value::Ref(root))];
    // Through the wire, so the handles are those a reader sees.
    a3s_jserial::read(&a3s_jserial::write(&graph).unwrap()).unwrap()
}

#[test]
fn lenient_sealed_object() {
    let inner = b"inner".to_vec();
    let cipher = encrypt(&inner);
    let graph = sealed_graph(42, "AES/ECB/PKCS5Padding", true, Some(&cipher));
    let mut warnings = Vec::new();
    assert_eq!(sealed::open(&graph, &mut warnings).unwrap(), inner);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("sealAlg"), "{warnings:?}");

    // "aes" is accepted silently, case-insensitively.
    let graph = sealed_graph(SEALED_OBJECT_UID, "aes", false, Some(&cipher));
    let mut warnings = Vec::new();
    assert_eq!(sealed::open(&graph, &mut warnings).unwrap(), inner);
    assert!(warnings.is_empty());
}

#[test]
fn missing_content_and_wrong_root_are_errors() {
    let graph = sealed_graph(SEALED_OBJECT_UID, "AES", false, None);
    let err = sealed::open(&graph, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, SealedError::Malformed(_)), "{err:?}");

    let mut b = Builder::new();
    let s = b.string("not sealed");
    let graph = b.finish(s);
    let err = sealed::open(&graph, &mut Vec::new()).unwrap_err();
    match err.with_file("r.a3s.repository") {
        Error::UnexpectedClass { found, .. } => assert_eq!(found, "java.lang.String"),
        other => panic!("{other:?}"),
    }

    let graph = sealed_graph(SEALED_OBJECT_UID, "AES", false, Some(&[1, 2, 3]));
    let err = sealed::open(&graph, &mut Vec::new()).unwrap_err();
    assert!(matches!(
        err,
        SealedError::Crypto(CryptoError::BadLength(3))
    ));
}
