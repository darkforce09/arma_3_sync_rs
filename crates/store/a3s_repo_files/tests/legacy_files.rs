//! Reading Java's gzip + serialization files with the class allowlist (A9), and the port of
//! `src/test/java/fr/soe/a3s/dao/A3SFilesAccessorTest.java` (A15).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use a3s_jserial::jdk::{Builder, FieldSpec};
use a3s_jserial::{Array, ArrayData, FieldType, Graph, Node, Value, flags};
use a3s_repo_files::legacy::{
    DeserializeError, Error, FILE_CORRUPTED, check_java_allowlist, read_object_file,
    read_preferences_file,
};
use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_types::constants::{
    CheckRepositoriesFrequency, IconResize, LookAndFeel, MinimizationType, StartWithOs,
};

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

/// Writes `graph` as a Java file (gzip + stream) into `dir`.
fn write_graph(dir: &Path, name: &str, graph: &Graph) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, gzip(&a3s_jserial::write(graph).unwrap())).unwrap();
    path
}

const S: u8 = flags::SERIALIZABLE;

// ---------------------------------------------------------------------------------------------
// A9

#[test]
fn a9_truncated_and_non_gzip_files_are_corrupted() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = std::fs::read(fixture("peruser/config/Arma3Sync/configuration/a3s.prefs")).unwrap();

    let truncated = dir.path().join("a3s.prefs");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let err = read_object_file(&truncated).unwrap_err();
    assert!(matches!(err, Error::Corrupted { .. }), "{err:?}");
    let text = err.to_string();
    assert!(
        text.starts_with("Failed to read file: a3s.prefs\n"),
        "{text}"
    );
    assert!(
        text.ends_with("\nThe file seems to be corrupted."),
        "{text}"
    );
    assert!(text.ends_with(FILE_CORRUPTED));

    let garbage = dir.path().join("garbage.a3s.profile");
    std::fs::write(&garbage, b"definitely not gzip").unwrap();
    let err = read_object_file(&garbage).unwrap_err();
    assert!(matches!(err, Error::Corrupted { .. }), "{err:?}");
    assert!(err.to_string().ends_with(FILE_CORRUPTED));

    // A complete gzip of a truncated stream is corrupted too (Java `EOFException`).
    let raw = {
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut out)
            .unwrap();
        out
    };
    let cut = dir.path().join("cut.a3s.prefs");
    std::fs::write(&cut, gzip(&raw[..raw.len() - 10])).unwrap();
    assert!(
        read_object_file(&cut)
            .unwrap_err()
            .to_string()
            .ends_with(FILE_CORRUPTED)
    );

    let missing = dir.path().join("missing.a3s.cfg");
    let err = read_object_file(&missing).unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err:?}");
    assert!(
        err.to_string()
            .starts_with("Failed to read file: missing.a3s.cfg\n")
    );
}

/// A `java.lang.Integer` (super `java.lang.Number`).
fn boxed_integer() -> Graph {
    let mut b = Builder::new();
    let number = b.class_desc("java.lang.Number", -8_742_448_824_652_078_965, S, &[], None);
    let integer = b.class_desc(
        "java.lang.Integer",
        1_360_826_667_806_852_920,
        S,
        &[FieldSpec::prim(FieldType::Int, "value")],
        Some(number),
    );
    let id = b.reserve_object(integer);
    b.set_object_data(
        id,
        vec![
            a3s_jserial::ClassData::default(),
            a3s_jserial::ClassData {
                values: vec![Value::Int(7)],
                annotations: Vec::new(),
            },
        ],
    );
    b.finish(Value::Ref(id))
}

/// An array of class `class` (elements given as raw data).
fn array(class: &str, data: ArrayData) -> Graph {
    let mut b = Builder::new();
    let desc = b.class_desc(class, 1, S, &[], None);
    let mut graph = b.finish(Value::Null);
    let id = graph.push(Node::Array(Array { class: desc, data }));
    graph.roots = vec![a3s_jserial::Content::Value(Value::Ref(id))];
    graph
}

#[test]
fn a9_allowlist() {
    let dir = tempfile::tempdir().unwrap();

    // Quirk: Integer is allowed, its superclass Number is not.
    let integer = boxed_integer();
    assert_eq!(
        check_java_allowlist(&integer).unwrap_err().class,
        "java.lang.Number"
    );
    let path = write_graph(dir.path(), "int.a3s.profile", &integer);
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
    // Java: `InvalidClassException("filter status: REJECTED")`, an `IOException`.
    assert_eq!(
        err.to_string(),
        "Failed to read file: int.a3s.profile\nfilter status: REJECTED"
    );

    // Nested arrays are rejected.
    let nested = array("[[B", ArrayData::Object(Vec::new()));
    assert_eq!(check_java_allowlist(&nested).unwrap_err().class, "[[B");
    assert!(read_object_file(&write_graph(dir.path(), "nested", &nested)).is_err());

    // Primitive arrays, allowed object arrays and enums are accepted.
    let ints = array("[I", ArrayData::Int(vec![1, 2]));
    assert!(read_object_file(&write_graph(dir.path(), "ints", &ints)).is_ok());
    let strings = array("[Ljava.lang.String;", ArrayData::Object(Vec::new()));
    assert!(read_object_file(&write_graph(dir.path(), "strings", &strings)).is_ok());
    let foreign = array("[Ljava.io.File;", ArrayData::Object(Vec::new()));
    assert!(check_java_allowlist(&foreign).is_err());

    let mut b = Builder::new();
    let constant = b.enum_value("fr.soe.a3s.constant.ModsetType", "EVENT");
    let enum_graph = b.finish(constant);
    assert!(read_object_file(&write_graph(dir.path(), "enum", &enum_graph)).is_ok());

    // Every real Java file passes.
    for rel in [
        "peruser/config/Arma3Sync/configuration/a3s.cfg",
        "peruser/config/Arma3Sync/configuration/a3s.prefs",
        "peruser/config/Arma3Sync/profiles/Event Night.a3s.profile",
        "peruser/data/Arma3Sync/repositories/Sample_HTTP__5ab39964a7ab.a3s.repository",
        "plain/Sample_HTTP.repository.gz",
        "variants/cfg_with_aia.a3s.cfg",
    ] {
        read_object_file(&fixture(rel)).unwrap_or_else(|err| panic!("{rel}: {err}"));
    }
}

// ---------------------------------------------------------------------------------------------
// A15

/// A Java-like `fr.soe.a3s.domain.Preferences` (descriptor from the JDK 25 dump).
fn java_preferences(game_launch: &str, minimized: &str, laf: &str, icon: &str) -> Graph {
    let mut b = Builder::new();
    let desc = b.class_desc(
        "fr.soe.a3s.domain.Preferences",
        7_034_433_806_261_372_240,
        S,
        &[
            FieldSpec::object(
                "checkRepositoriesFrequency",
                "Lfr/soe/a3s/constant/CheckRepositoriesFrequency;",
            ),
            FieldSpec::object("iconResizeSize", "Lfr/soe/a3s/constant/IconResize;"),
            FieldSpec::object(
                "launchPanelGameLaunch",
                "Lfr/soe/a3s/constant/MinimizationType;",
            ),
            FieldSpec::object(
                "launchPanelMinimized",
                "Lfr/soe/a3s/constant/MinimizationType;",
            ),
            FieldSpec::object("lookAndFeel", "Lfr/soe/a3s/constant/LookAndFeel;"),
            FieldSpec::object("preferGitHubUpdates", "Ljava/lang/Boolean;"),
            FieldSpec::object("startWithOS", "Lfr/soe/a3s/constant/StartWithOS;"),
        ],
        None,
    );
    let id = b.reserve_object(desc);
    let values = vec![
        b.enum_value("fr.soe.a3s.constant.CheckRepositoriesFrequency", "FREQ1"),
        b.enum_value("fr.soe.a3s.constant.IconResize", icon),
        b.enum_value("fr.soe.a3s.constant.MinimizationType", game_launch),
        b.enum_value("fr.soe.a3s.constant.MinimizationType", minimized),
        b.enum_value("fr.soe.a3s.constant.LookAndFeel", laf),
        b.boolean(true),
        b.enum_value("fr.soe.a3s.constant.StartWithOS", "ENABLED"),
    ];
    b.set_object_data(
        id,
        vec![a3s_jserial::ClassData {
            values,
            annotations: Vec::new(),
        }],
    );
    b.finish(Value::Ref(id))
}

#[test]
fn a15_write_then_read_preferences_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let graph = java_preferences("TRAY", "TASK_BAR", "LAF_GRAPHITE", "AUTO");
    let path = write_graph(dir.path(), "prefs.bin", &graph);
    assert!(path.is_file(), "Serialized preference file should exist");

    let restored = read_preferences_file(&path).unwrap();
    assert!(restored.warnings.is_empty(), "{:?}", restored.warnings);
    let prefs = restored.value;
    assert_eq!(prefs.look_and_feel, LookAndFeel::LafGraphite);
    assert_eq!(prefs.launch_panel_game_launch, MinimizationType::Tray);
    assert_eq!(prefs.launch_panel_minimized, MinimizationType::TaskBar);
    assert_eq!(prefs.icon_resize_size, IconResize::Auto);
    assert_eq!(prefs.start_with_os, StartWithOs::Enabled);
    assert_eq!(
        prefs.check_repositories_frequency,
        CheckRepositoriesFrequency::Freq1
    );

    // The same values through the Rust store.
    let store = LocalStore::with_dirs(LocalDirs {
        config: dir.path().join("config"),
        profiles: dir.path().join("profiles"),
        repositories: dir.path().join("repositories"),
        sha1_cache: dir.path().join("sha1_cache"),
    });
    store.write_preferences(&prefs).unwrap();
    assert_eq!(store.read_preferences().unwrap(), prefs);
}

#[test]
fn a15_rejects_serialized_classes_outside_the_legacy_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Builder::new();
    let number = b.class_desc("java.lang.Number", -8_742_448_824_652_078_965, S, &[], None);
    let big_integer = b.class_desc(
        "java.math.BigInteger",
        -8_287_574_255_936_472_291,
        S | flags::WRITE_METHOD,
        &[
            FieldSpec::prim(FieldType::Int, "signum"),
            FieldSpec {
                field_type: FieldType::Array,
                name: "magnitude",
                signature: Some("[B"),
            },
        ],
        Some(number),
    );
    let byte_array = b.class_desc("[B", -5_984_413_125_824_719_648, S, &[], None);
    let id = b.reserve_object(big_integer);
    let mut graph = b.finish(Value::Ref(id));
    let magnitude = graph.push(Node::Array(Array {
        class: byte_array,
        data: ArrayData::Byte(vec![1]),
    }));
    graph.object_mut(id).unwrap().class_data = vec![
        a3s_jserial::ClassData::default(),
        a3s_jserial::ClassData {
            values: vec![Value::Int(1), Value::Ref(magnitude)],
            annotations: Vec::new(),
        },
    ];
    let path = write_graph(dir.path(), "unexpected.bin", &graph);
    let err = read_object_file(&path).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Failed to read file: unexpected.bin\nfilter status: REJECTED"
    );
}

#[test]
fn a15_accepts_legacy_object_array_containers() {
    let dir = tempfile::tempdir().unwrap();
    let mut b = Builder::new();
    let desc = b.class_desc(
        "[Ljava.lang.Object;",
        -8_012_369_246_846_506_644,
        S,
        &[],
        None,
    );
    let first = b.string("sync-node");
    let second = b.string("1");
    let mut graph = b.finish(Value::Null);
    let id = graph.push(Node::Array(Array {
        class: desc,
        data: ArrayData::Object(vec![first, second]),
    }));
    graph.roots = vec![a3s_jserial::Content::Value(Value::Ref(id))];
    let path = write_graph(dir.path(), "legacy-array.bin", &graph);

    let restored = read_object_file(&path).unwrap();
    let root = restored.root_value().unwrap().as_ref().unwrap();
    let Node::Array(Array {
        data: ArrayData::Object(elements),
        ..
    }) = restored.node(root).unwrap()
    else {
        panic!("not an object array");
    };
    let strings: Vec<_> = elements
        .iter()
        .map(|v| restored.opt_string(*v).unwrap().unwrap())
        .collect();
    assert_eq!(strings, ["sync-node", "1"]);
}
