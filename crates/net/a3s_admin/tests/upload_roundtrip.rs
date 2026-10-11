//! The uploaded `.a3s` files equal Java's deserialize-then-serialize round trip of the local
//! files (golden fixtures written by the Java harness).

use std::io::Read as _;
use std::path::{Path, PathBuf};

use a3s_admin::build::testing::{JavaHashMap, RawGraph};
use a3s_admin::upload::{
    RoundTripOverrides, hash_set_roundtrip_capacity, java_roundtrip, java_roundtrip_bytes,
};
use a3s_jserial::jdk::Builder;
use a3s_jserial::{Limits, Node};
use a3s_repo_files::A3sFile;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures")
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .expect("gunzip");
    raw
}

fn read_raw(path: &Path) -> RawGraph {
    a3s_repo_files::read_a3s(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The decompressed stream Rust uploads for `path`.
fn uploaded(file: A3sFile, path: &Path, overrides: &RoundTripOverrides) -> Vec<u8> {
    let graph = read_raw(path).graph;
    let bytes = java_roundtrip_bytes(file, &graph, overrides).expect("round trip");
    let raw = gunzip(&bytes);
    a3s_jserial::read_with_limits(&raw, Limits::JAVA_A3S).expect("readable by Java's limits");
    raw
}

fn file_named(name: &str) -> A3sFile {
    A3sFile::ALL
        .into_iter()
        .find(|f| f.file_name() == name)
        .expect("a3s file")
}

/// Every built sample repository: the five local files round-trip to the bytes Java uploaded,
/// `serverinfo` with `compressedPboFilesOnly` from the upload setting.
#[test]
fn built_repositories_upload_java_round_trip() {
    let root = fixtures().join("java_built");
    let mut compared = 0;
    for variant in ["ftp_plain", "ftp_zip", "http_plain", "http_zip"] {
        let scenario: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(variant).join("scenario.json")).expect("scenario"),
        )
        .expect("json");
        let only_zip = scenario["upload_compressed_pbo_files_only"]
            .as_bool()
            .expect("flag");
        for step in ["1_initial", "2_rebuild_nochange", "3_modified"] {
            let dir = root.join(variant).join(step);
            for file in A3sFile::ALL {
                let overrides = RoundTripOverrides {
                    compressed_pbo_files_only: (file == A3sFile::ServerInfo).then_some(only_zip),
                };
                let local = dir.join("repo/.a3s").join(file.file_name());
                let expected = std::fs::read(dir.join("uploaded").join(file.file_name()))
                    .expect("uploaded fixture");
                assert_eq!(
                    uploaded(file, &local, &overrides),
                    gunzip(&expected),
                    "{variant}/{step}/{}",
                    file.file_name()
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 60);
}

/// Java round trips of server infos with 0..25 hidden folders and of event maps built with
/// `put` and `putAll` (JDK 25).
#[test]
fn collection_capacities_match_jdk_round_trips() {
    let root = fixtures().join("java_roundtrip");
    let none = RoundTripOverrides::default();
    for n in [0, 1, 2, 4, 7, 12, 13, 24, 25] {
        let dir = root.join(format!("serverinfo_hidden_{n}"));
        let rt1 = gunzip(&std::fs::read(dir.join("rt1")).expect("rt1"));
        let rt2 = gunzip(&std::fs::read(dir.join("rt2")).expect("rt2"));
        assert_eq!(
            uploaded(A3sFile::ServerInfo, &dir.join("fresh"), &none),
            rt1,
            "{n}"
        );
        assert_eq!(
            uploaded(A3sFile::ServerInfo, &dir.join("rt1"), &none),
            rt2,
            "{n}"
        );
    }
    for n in [1, 6, 12, 13, 24] {
        let dir = root.join(format!("events_map_{n}"));
        let rt1 = gunzip(&std::fs::read(dir.join("rt1")).expect("rt1"));
        assert_eq!(
            uploaded(A3sFile::Events, &dir.join("fresh_put"), &none),
            rt1,
            "{n}"
        );
    }
}

#[test]
fn capacity_models() {
    let sets: Vec<i32> = [0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 24, 25]
        .into_iter()
        .map(hash_set_roundtrip_capacity)
        .collect();
    assert_eq!(sets, [1, 2, 4, 4, 8, 8, 8, 16, 16, 32, 32, 64]);

    for (n, buckets, threshold) in [
        (0, 16, 0),
        (1, 16, 12),
        (11, 16, 12),
        (12, 16, 12),
        (13, 32, 24),
        (24, 32, 24),
        (25, 64, 48),
    ] {
        let map: JavaHashMap<(), ()> =
            JavaHashMap::deserialized((0..n).map(|i| (format!("k{i}"), (), ())));
        assert_eq!(map.write_params(), (buckets, threshold), "{n}");
    }

    // "Aa" and "BB" share a hash: iteration keeps insertion order within the bucket.
    let map: JavaHashMap<(), ()> = JavaHashMap::deserialized(
        ["BB", "x", "Aa"]
            .into_iter()
            .map(|k| (k.to_owned(), (), ())),
    );
    let order: Vec<&str> = map.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(order, ["BB", "Aa", "x"]);
}

/// The public WOG repository's files: everything but `serverinfo` round-trips unchanged; the
/// empty hidden-folder set is written with capacity 1 instead of the older JDK's 16.
#[test]
fn wog_files_round_trip() {
    let dir = fixtures().join("java_repos/wog_ftp");
    let none = RoundTripOverrides::default();
    for name in ["sync", "changelogs", "autoconfig", "events"] {
        let path = dir.join(name);
        let original = gunzip(&std::fs::read(&path).expect("fixture"));
        assert_eq!(uploaded(file_named(name), &path, &none), original, "{name}");
    }

    let path = dir.join("serverinfo");
    let original = gunzip(&std::fs::read(&path).expect("fixture"));
    let graph = read_raw(&path).graph;
    let root = graph.root_value().unwrap().as_ref().unwrap();
    let hidden = graph.field(root, "hiddenFolderPaths").unwrap();
    assert_eq!(
        a3s_jserial::jdk::set(&graph, hidden).unwrap(),
        Some(Vec::new())
    );
    let mut expected = original.clone();
    // The block data `[capacity, loadFactor, size]` of the set: 16 → 1.
    let header = [0, 0, 0, 16, 0x3f, 0x40, 0, 0, 0, 0, 0, 0];
    let at = expected
        .windows(header.len())
        .position(|w| w == header)
        .expect("HashSet block data");
    expected[at + 3] = 1;
    assert_eq!(uploaded(A3sFile::ServerInfo, &path, &none), expected);

    let flag = RoundTripOverrides {
        compressed_pbo_files_only: Some(true),
    };
    let flagged: a3s_types::ServerInfo =
        a3s_types::decode_stream(&uploaded(A3sFile::ServerInfo, &path, &flag)).unwrap();
    assert!(flagged.compressed_pbo_files_only);
}

/// Strings shared in the input stay shared; classes Java never writes are rejected.
#[test]
fn sharing_kept_and_unsupported_classes_rejected() {
    let path = fixtures().join("java_repos/wog_ftp/sync");
    let graph = read_raw(&path).graph;
    let strings = |g: &a3s_jserial::Graph| {
        g.nodes
            .iter()
            .filter(|n| matches!(n, Node::String(_)))
            .count()
    };
    let copy = java_roundtrip(&graph, &RoundTripOverrides::default()).expect("round trip");
    let reread = a3s_jserial::read(&a3s_jserial::write(&copy).unwrap()).unwrap();
    assert_eq!(strings(&reread), strings(&graph));

    let mut b = Builder::new();
    let class = b.class_desc(
        "java.util.TreeMap",
        919_286_545_866_124_006,
        a3s_jserial::flags::SERIALIZABLE | a3s_jserial::flags::WRITE_METHOD,
        &[],
        None,
    );
    let root = b.simple_object(class, Vec::new());
    let tree_map = b.finish(root);
    let err = java_roundtrip_bytes(A3sFile::Events, &tree_map, &RoundTripOverrides::default())
        .expect_err("TreeMap is not uploaded");
    assert!(matches!(
        err,
        a3s_admin::UploadError::RoundTrip {
            file: A3sFile::Events,
            ..
        }
    ));
}
