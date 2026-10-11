//! Verifier tests for WP04 `legacy`: allowlist edge cases (A3SFilesAccessor.java:27-53, 116-142),
//! malformed, truncated and wrong-key encrypted repository files (clean errors, never panics),
//! and Java readability of `sealed::seal` output (ignored test, run with the JDK harness).

use std::io::{Read as _, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{
    Array, ArrayData, ClassData, Content, FieldType, Graph, Node, Object, Value, flags,
};
use a3s_repo_files::legacy::crypto::{self, CryptoError};
use a3s_repo_files::legacy::{
    DeserializeError, Error, FILE_CORRUPTED, check_java_allowlist, read_object_file,
    read_repository_file, sealed,
};

const S: u8 = flags::SERIALIZABLE;
const HTTP: &str = "peruser/data/Arma3Sync/repositories/Sample_HTTP__5ab39964a7ab.a3s.repository";

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_local")
        .join(rel)
}

fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(raw).unwrap();
    encoder.finish().unwrap()
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .unwrap();
    raw
}

fn graph_with_root(mut graph: Graph, id: a3s_jserial::NodeId) -> Graph {
    graph.roots = vec![Content::Value(Value::Ref(id))];
    graph
}

fn array_graph(class: &str, data: ArrayData) -> Graph {
    let mut b = Builder::new();
    let desc = b.class_desc(class, 1, S, &[], None);
    let mut graph = b.finish(Value::Null);
    let id = graph.push(Node::Array(Array { class: desc, data }));
    graph_with_root(graph, id)
}

/// Through the wire, so the graph is what a reader sees.
fn reparse(graph: &Graph) -> Graph {
    a3s_jserial::read(&a3s_jserial::write(graph).unwrap()).unwrap()
}

fn rejected(graph: &Graph) -> Option<String> {
    check_java_allowlist(&reparse(graph)).err().map(|r| r.class)
}

// ---------------------------------------------------------------------------------------------
// Allowlist

#[test]
fn verify_allowlist_array_components() {
    let empty = || ArrayData::Object(Vec::new());
    // Accepted: primitive arrays, Object[], arrays of allowed classes.
    for (class, data) in [
        ("[Z", ArrayData::Boolean(vec![true])),
        ("[B", ArrayData::Byte(vec![1])),
        ("[C", ArrayData::Char(vec![65])),
        ("[S", ArrayData::Short(vec![1])),
        ("[I", ArrayData::Int(vec![1])),
        ("[J", ArrayData::Long(vec![1])),
        ("[F", ArrayData::Float(vec![1.0])),
        ("[D", ArrayData::Double(vec![1.0])),
        ("[Ljava.lang.Object;", empty()),
        ("[Ljava.lang.String;", empty()),
        ("[Ljava.lang.Integer;", empty()),
        ("[Ljava.util.Map$Entry;", empty()),
        ("[Lfr.soe.a3s.domain.Anything;", empty()),
    ] {
        assert_eq!(rejected(&array_graph(class, data)), None, "{class}");
    }
    // Rejected: nested arrays (component is an array class), foreign components.
    for class in [
        "[[B",
        "[[I",
        "[[Ljava.lang.Object;",
        "[[Ljava.lang.String;",
        "[Ljava.lang.Number;",
        "[Ljava.io.File;",
        "[Ljava.util.HashMap$Node;",
        "[Lfr.soe.a3sevil.X;",
    ] {
        assert_eq!(
            rejected(&array_graph(class, empty())).as_deref(),
            Some(class),
            "{class}"
        );
    }
}

#[test]
fn verify_allowlist_rejects_nested_and_superclass_descriptors() {
    // An allowed holder whose field holds a java.io.File.
    let mut b = Builder::new();
    let file = b.class_desc(
        "java.io.File",
        301_077_366_599_181_567,
        S,
        &[FieldSpec::object("path", "Ljava/lang/String;")],
        None,
    );
    let path = b.string("/etc/passwd");
    let file_obj = b.simple_object(file, vec![path]);
    let holder = b.class_desc(
        "fr.soe.a3s.domain.Holder",
        1,
        S,
        &[FieldSpec::object("x", "Ljava/lang/Object;")],
        None,
    );
    let root = b.simple_object(holder, vec![file_obj]);
    let graph = b.finish(root);
    // Built without write-method annotations; check the arena directly.
    assert_eq!(
        check_java_allowlist(&graph).unwrap_err().class,
        "java.io.File"
    );

    // An allowed class with a forbidden serializable superclass.
    let mut b = Builder::new();
    let base = b.class_desc("java.util.AbstractMapLike", 2, S, &[], None);
    let sub = b.class_desc("fr.soe.a3s.domain.Sub", 3, S, &[], Some(base));
    let id = b.reserve_object(sub);
    b.set_object_data(id, vec![ClassData::default(), ClassData::default()]);
    let graph = b.finish(Value::Ref(id));
    assert_eq!(
        rejected(&graph).as_deref(),
        Some("java.util.AbstractMapLike")
    );

    // Only the prefix `fr.soe.a3s.` (with the dot) is trusted.
    let mut b = Builder::new();
    let evil = b.class_desc("fr.soe.a3sevil.Payload", 4, S, &[], None);
    let root = b.simple_object(evil, Vec::new());
    assert_eq!(
        rejected(&b.finish(root)).as_deref(),
        Some("fr.soe.a3sevil.Payload")
    );

    // A `Class` object of a forbidden class: Java's filter sees its descriptor too.
    let mut b = Builder::new();
    let desc = b.class_desc("java.lang.Runtime", 5, S, &[], None);
    let mut graph = b.finish(Value::Null);
    let id = graph.push(Node::Class(desc));
    let graph = graph_with_root(graph, id);
    assert_eq!(rejected(&graph).as_deref(), Some("java.lang.Runtime"));
}

#[test]
fn verify_allowlist_rejection_through_read_object_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested.a3s.profile");
    let graph = array_graph("[[B", ArrayData::Object(Vec::new()));
    std::fs::write(&path, gzip(&a3s_jserial::write(&graph).unwrap())).unwrap();
    let err = read_object_file(&path).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::Deserialize {
                source: DeserializeError::Rejected(_),
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "Failed to deserialize file: nested.a3s.profile"
    );
}

// ---------------------------------------------------------------------------------------------
// Malformed repository files

/// Reads `bytes` as a repository file, converting a panic into a test failure message.
fn read_bytes_as_repository(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    catch_unwind(AssertUnwindSafe(|| read_repository_file(&path)))
        .unwrap_or_else(|_payload| panic!("read_repository_file panicked on {name}"))
        .map(|_file| ())
        .map_err(|err| err.to_string())
}

#[test]
fn verify_empty_and_truncated_repository_files() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_bytes_as_repository(dir.path(), "empty.a3s.repository", b"").unwrap_err();
    assert!(
        err.starts_with("Failed to read file: empty.a3s.repository"),
        "{err}"
    );
    assert!(err.ends_with(FILE_CORRUPTED), "{err}");

    let bytes = std::fs::read(fixture(HTTP)).unwrap();
    for len in 0..bytes.len() {
        let err = read_bytes_as_repository(dir.path(), "cut.a3s.repository", &bytes[..len])
            .expect_err("a truncated file must not decode");
        assert!(err.starts_with("Failed to "), "{len}: {err}");
    }
    // Truncating the *uncompressed* stream gives a clean "corrupted" error too.
    let raw = gunzip(&bytes);
    for len in [0, 1, 4, raw.len() / 2, raw.len() - 1] {
        let err = read_bytes_as_repository(dir.path(), "rawcut.a3s.repository", &gzip(&raw[..len]))
            .unwrap_err();
        assert!(err.starts_with("Failed to "), "{len}: {err}");
    }
}

#[test]
fn verify_bit_flips_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let raw = gunzip(&std::fs::read(fixture(HTTP)).unwrap());
    // Outer stream (SealedObject envelope): flip every byte.
    for i in 0..raw.len() {
        let mut bad = raw.clone();
        bad[i] ^= 0xFF;
        let _result: Result<(), String> =
            read_bytes_as_repository(dir.path(), "flip.a3s.repository", &gzip(&bad));
    }
    // Inner stream (Repository): flip every byte, then seal again.
    let inner = gunzip(&std::fs::read(fixture("plain/Sample_HTTP.repository.gz")).unwrap());
    for i in 0..inner.len() {
        let mut bad = inner.clone();
        bad[i] ^= 0xFF;
        let _result: Result<(), String> = read_bytes_as_repository(
            dir.path(),
            "inner.a3s.repository",
            &sealed::seal(&bad).unwrap(),
        );
    }
}

/// A `SealedObject` with `encryptedContent` of the given array class and data.
fn sealed_with(content_class: &str, data: Option<ArrayData>, seal_alg: Option<&str>) -> Vec<u8> {
    let mut b = Builder::new();
    let desc = b.class_desc(
        sealed::SEALED_OBJECT,
        sealed::SEALED_OBJECT_UID,
        S,
        &[
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
        ],
        None,
    );
    let array_desc = b.class_desc(content_class, 1, S, &[], None);
    let alg = b.opt_string(seal_alg);
    let mut graph = b.finish(Value::Null);
    let content = data.map_or(Value::Null, |data| {
        Value::Ref(graph.push(Node::Array(Array {
            class: array_desc,
            data,
        })))
    });
    let root = graph.push(Node::Object(Object {
        class: desc,
        class_data: vec![ClassData {
            values: vec![Value::Null, content, Value::Null, alg],
            annotations: Vec::new(),
        }],
    }));
    gzip(&a3s_jserial::write(&graph_with_root(graph, root)).unwrap())
}

#[test]
fn verify_malformed_sealed_objects_give_clean_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.a3s.repository");

    // Empty ciphertext: BadLength, reported as a decryption failure.
    std::fs::write(
        &path,
        sealed_with("[B", Some(ArrayData::Byte(Vec::new())), Some("AES")),
    )
    .unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(
        matches!(
            err,
            Error::Decrypt {
                source: CryptoError::BadLength(0),
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("Failed to decrypt serialized file: x.a3s.repository"),
        "{err}"
    );

    // Ciphertext of 17 bytes.
    std::fs::write(
        &path,
        sealed_with("[B", Some(ArrayData::Byte(vec![0; 17])), None),
    )
    .unwrap();
    assert!(matches!(
        read_repository_file(&path).unwrap_err(),
        Error::Decrypt {
            source: CryptoError::BadLength(17),
            ..
        }
    ));

    // `encryptedContent` of the wrong array type, or null.
    std::fs::write(
        &path,
        sealed_with("[I", Some(ArrayData::Int(vec![1, 2, 3, 4])), Some("AES")),
    )
    .unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(err.to_string().starts_with("Failed to "), "{err}");
    std::fs::write(&path, sealed_with("[B", None, Some("AES"))).unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(err.to_string().starts_with("Failed to "), "{err}");

    // A root of null.
    let mut graph = Builder::new().finish(Value::Null);
    graph.roots = vec![Content::Value(Value::Null)];
    std::fs::write(&path, gzip(&a3s_jserial::write(&graph).unwrap())).unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Failed to decrypt serialized file: x.a3s.repository"),
        "{err}"
    );
}

#[test]
fn verify_unencrypted_repository_is_unexpected_class() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.a3s.repository");
    std::fs::copy(fixture("plain/Sample_HTTP.repository.gz"), &path).unwrap();
    let err = read_repository_file(&path).unwrap_err();
    match &err {
        Error::UnexpectedClass { file, found } => {
            assert_eq!(file, "plain.a3s.repository");
            assert_eq!(found, "fr.soe.a3s.domain.repository.Repository");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        err.to_string()
            .starts_with("Failed to decrypt serialized file: plain.a3s.repository"),
        "{err}"
    );
}

#[test]
fn verify_wrong_key_and_wrong_inner_class_are_errors() {
    use aes::cipher::{Array as Block, BlockCipherEncrypt as _, KeyInit as _};

    let dir = tempfile::tempdir().unwrap();
    let inner = gunzip(&std::fs::read(fixture("plain/Sample_HTTP.repository.gz")).unwrap());

    // Same envelope, other key (last byte changed).
    let mut key = crypto::JAVA_AES_KEY;
    key[15] ^= 1;
    let cipher = aes::Aes128::new(&Block::from(key));
    let pad = 16 - inner.len() % 16;
    let mut ct = inner.clone();
    ct.extend(std::iter::repeat_n(u8::try_from(pad).unwrap(), pad));
    for chunk in ct.chunks_exact_mut(16) {
        let mut block = Block::from(<[u8; 16]>::try_from(&*chunk).unwrap());
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    let path = dir.path().join("wrongkey.a3s.repository");
    std::fs::write(
        &path,
        sealed_with("[B", Some(ArrayData::Byte(ct)), Some("AES")),
    )
    .unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(err.to_string().starts_with("Failed to "), "{err}");

    // The right key, but the plaintext is a profile stream, not a repository.
    let profile = gunzip(
        &std::fs::read(fixture(
            "peruser/config/Arma3Sync/profiles/Event Night.a3s.profile",
        ))
        .unwrap(),
    );
    let path = dir.path().join("profile.a3s.repository");
    std::fs::write(&path, sealed::seal(&profile).unwrap()).unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(matches!(err, Error::Deserialize { .. }), "{err:?}");

    // Plaintext that is not a Java stream at all.
    let path = dir.path().join("garbage.a3s.repository");
    std::fs::write(&path, sealed::seal(b"not a java stream").unwrap()).unwrap();
    let err = read_repository_file(&path).unwrap_err();
    assert!(err.to_string().starts_with("Failed to "), "{err}");
}

#[test]
fn verify_padding_edge_lengths_round_trip() {
    for len in [15usize, 16, 31, 32, 33, 4096] {
        let plain: Vec<u8> = (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect();
        let ct = crypto::encrypt(&plain);
        assert_eq!(ct.len(), (len / 16 + 1) * 16, "{len}");
        assert_eq!(crypto::decrypt(&ct).unwrap(), plain);
        // Any change to the final block breaks the padding or the data, never panics.
        let mut bad = ct.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0x80;
        let _result: Result<Vec<u8>, CryptoError> = crypto::decrypt(&bad);
    }
}

// ---------------------------------------------------------------------------------------------
// Java readability of `seal` output (run manually with the JDK harness)

/// Writes sealed repository files into `$A3S_VERIFY_OUT` for `a3s_dev_tools java-verify`.
#[test]
#[ignore = "writes files for the Java harness; set A3S_VERIFY_OUT"]
fn verify_write_sealed_files_for_java() {
    let Some(out) = std::env::var_os("A3S_VERIFY_OUT") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    for (plain, file) in [
        (
            "plain/Sample_HTTP.repository.gz",
            "Sample_HTTP__5ab39964a7ab.a3s.repository",
        ),
        (
            "plain/Sample_FTP.repository.gz",
            "Sample_FTP__21a7c6dd6e0b.a3s.repository",
        ),
        (
            "plain/Sample_SFTP.repository.gz",
            "Sample_SFTP__132c722dd1cb.a3s.repository",
        ),
        (
            "variants/repository_pre_connections.repository.gz",
            "pre_connections.a3s.repository",
        ),
    ] {
        let inner = gunzip(&std::fs::read(fixture(plain)).unwrap());
        std::fs::write(out.join(file), sealed::seal(&inner).unwrap()).unwrap();
        // Read back by Rust too.
        read_repository_file(&out.join(file)).unwrap();
    }
    // A Unicode name, padded so the inner stream is an exact multiple of 16 (a full padding block).
    let inner = gunzip(&std::fs::read(fixture("plain/Sample_HTTP.repository.gz")).unwrap());
    let mut graph = a3s_jserial::read(&inner).unwrap();
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let name_id = graph.field(root, "name").unwrap().as_ref().unwrap();
    for extra in 0..32 {
        let name = format!("Événement 🎮 {}", "x".repeat(extra));
        *graph.node_mut(name_id).unwrap() = Node::String(name.clone());
        let bytes = a3s_jserial::write(&graph).unwrap();
        if bytes.len().is_multiple_of(16) {
            let path = out.join("unicode_full_pad.a3s.repository");
            std::fs::write(&path, sealed::seal(&bytes).unwrap()).unwrap();
            assert_eq!(
                read_repository_file(&path).unwrap().value.repository.name,
                name
            );
            return;
        }
    }
    panic!("no padding-aligned name found");
}
