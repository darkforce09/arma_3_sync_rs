//! `export.a3s.autoconfig` file I/O, and autoconfig URLs of a Java-built repository.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use a3s_jserial::{JavaFilterLimits, Limits};
use a3s_repo_files::autoconfig_export::{
    AUTOCONFIG_EXPORT_FILE_NAME, decode_auto_config_export, encode_auto_config_export,
    export_auto_config, import_auto_config,
};
use a3s_repo_files::{Error, read_a3s};
use a3s_types::autoconfig_url::{autoconfig_display_url, determine_autoconfig_url, parse};
use a3s_types::local::{
    AutoConfigExport, LauncherOptions, Profile, Repository, TreeDirectory, TreeLeaf, TreeNode,
};
use a3s_types::{AutoConfig, FavoriteServer, Protocol, ProtocolType};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures")
        .join(relative)
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut raw)
        .unwrap();
    raw
}

fn gzip(raw: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(raw).unwrap();
    encoder.finish().unwrap()
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn server(name: &str, port: i32) -> FavoriteServer {
    FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some("192.0.2.1".to_owned()),
        port,
        password: Some("server-pass".to_owned()),
        selected: port == 2302,
        modset_name: Some("Event".to_owned()),
        repository_name: Some("WOG".to_owned()),
    }
}

fn sample() -> AutoConfigExport {
    let mut group = TreeDirectory::new("Event");
    group.add_child(TreeNode::Leaf(TreeLeaf::new("@cba_a3")));
    let mut main = Profile::new("Main");
    main.tree.add_child(TreeNode::Directory(group));
    main.additional_parameters = Some("-noSplash".to_owned());
    main.launcher_options = LauncherOptions {
        cpu_count_selection: 8,
        ..Default::default()
    };
    let mut repository = Repository::new(
        "WOG",
        Protocol::new(
            ProtocolType::Ftp,
            "ftp.example/Repo",
            "21",
            "user",
            "secret",
            true,
        ),
    );
    repository.revision = 12;
    AutoConfigExport {
        profiles: vec![main, Profile::new("Default")],
        favorite_servers: vec![server("Alpha", 2302), server("Bravo", 2402)],
        repositories: vec![repository],
    }
}

/// A profile whose tree nests `groups` addon groups, with an addon in the deepest one.
fn deep_profile(groups: usize) -> Profile {
    let mut node = TreeDirectory::new("group 0");
    node.add_child(TreeNode::Leaf(TreeLeaf::new("@bottom")));
    for level in 1..groups {
        let mut parent = TreeDirectory::new(format!("group {level}"));
        parent.add_child(TreeNode::Directory(node));
        node = parent;
    }
    let mut profile = Profile::new("Deep");
    profile.tree.add_child(TreeNode::Directory(node));
    profile
}

fn deep_export(groups: usize) -> AutoConfigExport {
    AutoConfigExport {
        profiles: vec![deep_profile(groups)],
        ..Default::default()
    }
}

/// The deepest profile tree Java's reader accepts, found with the exact filter emulation.
fn deepest_exportable_tree() -> usize {
    (1..200)
        .find(|&groups| encode_auto_config_export(&deep_export(groups + 1)).is_err())
        .unwrap()
}

#[test]
fn wog_autoconfig_urls_round_trip() {
    let auto_config: AutoConfig = read_a3s(&fixture("java_repos/wog_ftp/autoconfig")).unwrap();
    let protocol = auto_config.protocol.unwrap();
    let stored = determine_autoconfig_url(&protocol);
    assert_eq!(stored, "88.99.69.243/Repo/WOG/.a3s/autoconfig");
    let display = autoconfig_display_url(protocol.protocol_type, Some(&stored)).unwrap();
    assert_eq!(display, "ftp://88.99.69.243/Repo/WOG/.a3s/autoconfig");
    let parsed = parse(&display).unwrap();
    assert_eq!(parsed.protocol_type, ProtocolType::Ftp);
    assert_eq!(parsed.url, "88.99.69.243/Repo/WOG/.a3s");
    assert_eq!(parsed.port, "21");
}

#[test]
fn imports_the_java_export() {
    let path = fixture("java_local/export/export.a3s.autoconfig");
    let imported = import_auto_config(&path).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    let export = imported.value;
    let names: Vec<&str> = export.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Event Night", "Default"]);
    for profile in &export.profiles {
        assert_eq!(profile.launcher_options, LauncherOptions::default());
    }
    assert_eq!(export.favorite_servers.len(), 1);
    assert_eq!(
        export.favorite_servers[0].password.as_deref(),
        Some("alpha-pass")
    );
    assert_eq!(export.repositories.len(), 1);
    assert_eq!(export.repositories[0].name, "Sample HTTP");
    assert_eq!(
        export.repositories[0].protocol,
        Protocol::new(
            ProtocolType::Http,
            "repo.example.org/arma3",
            "80",
            "anonymous",
            "",
            true
        )
    );

    // Re-exporting what Java exported gives Java's stream back.
    let java = gunzip(&std::fs::read(&path).unwrap());
    let rust = gunzip(&encode_auto_config_export(&export).unwrap());
    assert!(rust == java, "re-export differs from the Java export");
}

#[test]
fn export_round_trips_through_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let export = sample();
    let path = export_auto_config(&export, dir.path()).unwrap();
    assert_eq!(path, dir.path().join(AUTOCONFIG_EXPORT_FILE_NAME));
    assert_eq!(file_names(dir.path()), [AUTOCONFIG_EXPORT_FILE_NAME]);

    let bytes = std::fs::read(&path).unwrap();
    // Gzip magic, deflate, no flags, mtime 0, then XFL and OS 255 like `GZIPOutputStream`.
    assert_eq!(bytes[..8], [0x1f, 0x8b, 8, 0, 0, 0, 0, 0]);
    assert_eq!(bytes[9], 255);
    let raw = gunzip(&bytes);
    let graph = a3s_jserial::read_with_limits(&raw, Limits::JAVA_A3S).unwrap();
    let root = graph.root_value().unwrap();
    let a3s_jserial::Value::Ref(root) = root else {
        panic!("root is not an object: {root:?}")
    };
    assert_eq!(
        graph.class_name_of(root).unwrap(),
        "fr.soe.a3s.domain.AutoConfig"
    );

    let imported = import_auto_config(&path).unwrap();
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    let imported = imported.value;
    assert_eq!(imported.favorite_servers, export.favorite_servers);
    assert_eq!(imported.profiles.len(), 2);
    for (imported, original) in imported.profiles.iter().zip(&export.profiles) {
        assert_eq!(imported.launcher_options, LauncherOptions::default());
        let mut expected = original.clone();
        expected.launcher_options = LauncherOptions::default();
        assert_eq!(imported, &expected);
    }
    // Repositories travel as `new Repository(name, protocol)`, password included.
    let repository = &imported.repositories[0];
    assert_eq!(repository.name, "WOG");
    assert_eq!(repository.protocol, export.repositories[0].protocol);
    assert_eq!(repository.protocol.password.as_deref(), Some("secret"));
    assert_eq!(repository.revision, 0);
}

#[test]
fn export_replaces_an_existing_file_and_needs_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(AUTOCONFIG_EXPORT_FILE_NAME);
    std::fs::write(&path, b"old").unwrap();
    export_auto_config(&sample(), dir.path()).unwrap();
    assert_eq!(import_auto_config(&path).unwrap().value.profiles.len(), 2);
    assert_eq!(file_names(dir.path()), [AUTOCONFIG_EXPORT_FILE_NAME]);

    let missing = dir.path().join("missing");
    match export_auto_config(&sample(), &missing) {
        Err(Error::Io { path, source }) => {
            assert_eq!(path, missing);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("{other:?}"),
    }
    assert!(!missing.exists());

    // A file where the folder should be.
    match export_auto_config(&sample(), &path) {
        Err(Error::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotADirectory);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn import_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        import_auto_config(&dir.path().join("missing")),
        Err(Error::Io { .. })
    ));
    assert!(matches!(
        decode_auto_config_export(b"not gzip"),
        Err(Error::Gzip(_))
    ));
    // A repository's `.a3s/autoconfig` has a different root class.
    let remote = std::fs::read(fixture("java_repos/wog_ftp/autoconfig")).unwrap();
    assert!(matches!(
        decode_auto_config_export(&remote),
        Err(Error::Types(_))
    ));

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let zeros = vec![0u8; 1 << 20];
    for _ in 0..257 {
        encoder.write_all(&zeros).unwrap();
    }
    let bomb = encoder.finish().unwrap();
    assert!(matches!(
        decode_auto_config_export(&bomb),
        Err(Error::TooLarge(_))
    ));
}

#[test]
fn export_refuses_trees_too_deep_for_java() {
    let deepest = deepest_exportable_tree();
    // Each group level costs two levels of Java's depth limit of 100.
    assert!((40..50).contains(&deepest), "{deepest}");

    let dir = tempfile::tempdir().unwrap();
    export_auto_config(&deep_export(deepest), dir.path()).unwrap();
    let path = dir.path().join(AUTOCONFIG_EXPORT_FILE_NAME);
    let accepted = gunzip(&std::fs::read(&path).unwrap());
    a3s_jserial::java_filter_stats(&accepted)
        .unwrap()
        .check(JavaFilterLimits::JAVA_A3S)
        .unwrap();
    std::fs::remove_file(&path).unwrap();

    match export_auto_config(&deep_export(deepest + 1), dir.path()) {
        Err(Error::Serialization(a3s_jserial::Error::LimitExceeded(_))) => {}
        other => panic!("{other:?}"),
    }
    assert!(file_names(dir.path()).is_empty());
}

#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn java_reads_rust_exports_and_agrees_on_the_depth_limit() {
    let harness = a3s_dev_tools::Harness::locate().unwrap();
    let deepest = deepest_exportable_tree();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let folder = dir.path().join(name);
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join(AUTOCONFIG_EXPORT_FILE_NAME);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    let sample = write("sample", &encode_auto_config_export(&sample()).unwrap());
    let deepest_path = write(
        "deepest",
        &encode_auto_config_export(&deep_export(deepest)).unwrap(),
    );
    // Bypasses the refusal to show that Java rejects the next level.
    let too_deep = write(
        "too_deep",
        &gzip(&a3s_types::encode_stream(&deep_export(deepest + 1)).unwrap()),
    );

    for (path, accepted) in [(sample, true), (deepest_path, true), (too_deep, false)] {
        let report = harness
            .verify(
                std::slice::from_ref(&path),
                None,
                a3s_dev_tools::DumpMode::Identity,
            )
            .unwrap();
        assert_eq!(
            report.is_success(),
            accepted,
            "{}: {report:?}",
            path.display()
        );
        assert_eq!(report.ok.len() + report.failed.len(), 1, "{report:?}");
    }
}
