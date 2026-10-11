//! Folder-level tests of the build stages: scan, total size, SHA-1, revision, server info,
//! autoconfig, zsync, zip, writing, cancellation and limits.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_admin::build::{
    BuildError, BuildEvent, BuildOptions, BuildOutcome, STAGE_SHA1, build_repository,
};
use a3s_hash::Sha1Cache;
use a3s_repo_files::{A3sFile, read_a3s};
use a3s_types::local::Repository;
use a3s_types::{
    AutoConfig, Changelogs, Events, FavoriteServer, Protocol, ProtocolType, ServerInfo,
    SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode,
};
use parking_lot::Mutex;

const DATE: i64 = 1_704_067_200_000;

fn write(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    path
}

fn set_mtime(path: &Path, millis: i64) {
    let time =
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(u64::try_from(millis).unwrap());
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

fn repository(root: &Path, protocol_type: ProtocolType) -> Repository {
    let protocol = Protocol::new(
        protocol_type,
        "example.org/repo",
        "8080",
        "anonymous",
        "",
        false,
    );
    let mut repo = Repository::new("Test repo", protocol);
    repo.path = Some(root.to_string_lossy().into_owned());
    repo
}

struct Run {
    outcome: Result<BuildOutcome, BuildError>,
    events: Vec<BuildEvent>,
}

fn run_with(repo: &Repository, cache: &mut Sha1Cache, date: i64, cancel: &AtomicBool) -> Run {
    let events = Mutex::new(Vec::new());
    let outcome = build_repository(
        repo,
        cache,
        &BuildOptions {
            build_date: Some(date),
        },
        cancel,
        &|e| events.lock().push(e),
    );
    Run {
        outcome,
        events: events.into_inner(),
    }
}

fn build(repo: &Repository, cache: &mut Sha1Cache) -> BuildOutcome {
    run_with(repo, cache, DATE, &AtomicBool::new(false))
        .outcome
        .unwrap()
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut out)
        .unwrap();
    out
}

/// Decompressed streams of the five `.a3s` files.
fn streams(root: &Path) -> BTreeMap<&'static str, Vec<u8>> {
    A3sFile::ALL
        .iter()
        .map(|f| {
            (
                f.file_name(),
                gunzip(&root.join(".a3s").join(f.file_name())),
            )
        })
        .collect()
}

/// Raw bytes of every file in `.a3s/`.
fn a3s_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let dir = root.join(".a3s");
    if !dir.exists() {
        return BTreeMap::new();
    }
    fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

fn names(dir: &SyncTreeDirectory) -> Vec<&str> {
    dir.children.iter().map(SyncTreeNode::name).collect()
}

fn subdir<'a>(dir: &'a SyncTreeDirectory, name: &str) -> &'a SyncTreeDirectory {
    match dir.child(name) {
        Some(SyncTreeNode::Directory(d)) => d,
        other => panic!("{name}: {other:?}"),
    }
}

fn leaf<'a>(dir: &'a SyncTreeDirectory, path: &str) -> &'a SyncTreeLeaf {
    let mut parts: Vec<&str> = path.split('/').collect();
    let last = parts.pop().unwrap();
    let mut d = dir;
    for p in parts {
        d = subdir(d, p);
    }
    match d.child(last) {
        Some(SyncTreeNode::Leaf(l)) => l,
        other => panic!("{path}: {other:?}"),
    }
}

/// Whether the current user can read files whose permissions forbid it (e.g. root).
fn permissions_enforced(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    let probe = write(dir, "probe", b"x");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    let enforced = fs::read(&probe).is_err();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o644)).unwrap();
    fs::remove_file(probe).unwrap();
    enforced
}

#[test]
fn scan_order_exclusions_addons_and_pruning() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "b_file.txt", b"1");
    write(&root, "A_file.txt", b"1");
    write(&root, "zeta/x.txt", b"1");
    write(&root, "Alpha/x.txt", b"1");
    write(&root, "@mod/Addons", b"marker file");
    write(&root, "@mod/data/.a3s/hidden.txt", b"1");
    write(&root, "@mod/data/.A3S/hidden.txt", b"1");
    write(&root, "@mod/data/x.zsync", b"1");
    write(&root, "@mod/data/a.pbo.zip", b"1");
    write(&root, "@mod/data/b.a3s.bak", b"1");
    write(&root, "@mod/data/c.ebo.zip", b"1");
    write(&root, "@mod/data/kept.txt", b"1");
    write(&root, "@other/addons/m.pbo", b"1");
    write(&root, "excluded.txt", b"1");
    write(&root, "exdir/inside.txt", b"1");
    write(&root, "only_skipped/y.zsync", b"1");
    fs::create_dir_all(root.join("empty/nested")).unwrap();
    let mut repo = repository(&root, ProtocolType::Ftp);
    repo.excluded_files_from_build = [
        root.join("excluded.txt").to_string_lossy().into_owned(),
        root.join("exdir").to_string_lossy().into_owned(),
    ]
    .into();
    let run = run_with(
        &repo,
        &mut Sha1Cache::default(),
        DATE,
        &AtomicBool::new(false),
    );
    let sync = run.outcome.unwrap().sync;

    // Directories first, then files, each sorted with compareToIgnoreCase.
    assert_eq!(
        names(&sync),
        [
            "@mod",
            "@other",
            "Alpha",
            "exdir",
            "zeta",
            "A_file.txt",
            "b_file.txt"
        ]
    );
    assert!(!sync.mark_as_addon, "the root is never an addon");
    let module = subdir(&sync, "@mod");
    assert!(module.mark_as_addon, "an `Addons` file marks the addon");
    assert!(subdir(&sync, "@other").mark_as_addon);
    assert!(!subdir(&sync, "zeta").mark_as_addon);
    // `.a3s`/`.A3S` skipped at depth 2; `.zsync`, `.pbo.zip`, `.a3s` names skipped;
    // `.ebo.zip` skipped too.
    assert_eq!(names(subdir(module, "data")), ["kept.txt"]);
    // An excluded directory path has no effect.
    assert_eq!(names(subdir(&sync, "exdir")), ["inside.txt"]);
    assert_eq!(sync.leaf_count(), 8);
    for warning in [
        "Warning: Removing empty directory from metadata: empty/nested",
        "Warning: Removing empty directory from metadata: empty",
        "Warning: Removing empty directory from metadata: only_skipped",
    ] {
        assert!(
            run.events
                .contains(&BuildEvent::Warning(warning.to_owned())),
            "{warning}: {:?}",
            run.events
        );
    }
    let info: ServerInfo = read_a3s(&root.join(".a3s/serverinfo")).unwrap();
    assert_eq!(info.number_of_files, 8);
}

#[test]
fn symbolic_links_are_followed_and_loops_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&tmp.path().join("outside"), "x.txt", b"x");
    write(&root, "a/b.txt", b"b");
    std::os::unix::fs::symlink(tmp.path().join("outside"), root.join("linked")).unwrap();
    let repo = repository(&root, ProtocolType::Ftp);
    let sync = build(&repo, &mut Sha1Cache::default()).sync;
    assert_eq!(names(subdir(&sync, "linked")), ["x.txt"]);

    // A link back to an ancestor fails the scan instead of recursing forever.
    std::os::unix::fs::symlink(&root, root.join("a/loop")).unwrap();
    let run = run_with(
        &repo,
        &mut Sha1Cache::default(),
        DATE,
        &AtomicBool::new(false),
    );
    assert!(
        matches!(run.outcome, Err(BuildError::Scan { ref path, .. }) if path.ends_with("a/loop")),
        "{:?}",
        run.outcome
    );
}

#[test]
fn total_files_size_counts_previous_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "@m/addons/a.pbo", &[7u8; 1000]);
    write(&root, "@m/addons/b.txt", &[1u8; 10]);
    write(&root, "excluded.txt", &[1u8; 100]);
    write(&root, "orphan.zsync", &[1u8; 5]);
    write(&root, "old.pbo.zip", &[1u8; 3]);
    write(&root, "sub/.a3s/nested", &[1u8; 7]);
    write(&root, ".a3s/ignored", &vec![1u8; 50_000]);
    let mut repo = repository(&root, ProtocolType::Http);
    repo.compressed = true;
    repo.excluded_files_from_build =
        [root.join("excluded.txt").to_string_lossy().into_owned()].into();
    let mut cache = Sha1Cache::default();
    let first = build(&repo, &mut cache);
    assert_eq!(
        first.server_info.total_files_size,
        1000 + 10 + 100 + 5 + 3 + 7
    );

    // The second build counts the first build's .zsync and .zip files.
    let second = build(&repo, &mut cache);
    let outputs: u64 = [
        "@m/addons/a.pbo.zsync",
        "@m/addons/b.txt.zsync",
        "@m/addons/a.pbo.zip",
        "sub/.a3s/nested.zsync",
    ]
    .iter()
    .filter_map(|rel| fs::metadata(root.join(rel)).ok())
    .map(|m| m.len())
    .sum();
    assert!(outputs > 0);
    assert_eq!(
        second.server_info.total_files_size,
        first.server_info.total_files_size + i64::try_from(outputs).unwrap()
    );
    assert_eq!(second.server_info.revision, first.server_info.revision);
    assert!(!second.content_updated);
}

#[test]
fn sha1_cache_hits_misses_and_stale_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "empty.txt", b"");
    let abc = write(&root, "abc.txt", b"abc");
    let excluded = write(&root, "excluded.txt", b"x");
    let gone = write(&root, "gone.txt", b"gone");
    let mut repo = repository(&root, ProtocolType::Ftp);
    let mut cache = Sha1Cache::default();
    let first = build(&repo, &mut cache);
    assert_eq!(leaf(&first.sync, "empty.txt").sha1.as_deref(), Some("0"));
    assert_eq!(
        leaf(&first.sync, "abc.txt").sha1.as_deref(),
        Some("a9993e364706816aba3e25717850c26c9cd0d89d")
    );
    assert_eq!(first.rehashed.len(), 4);

    // Untouched files: cache hits.
    repo.excluded_files_from_build = [excluded.to_string_lossy().into_owned()].into();
    fs::remove_file(&gone).unwrap();
    let second = build(&repo, &mut cache);
    assert!(second.rehashed.is_empty(), "{:?}", second.rehashed);
    // The deleted file's entry is gone, the excluded (but existing) file's entry stays.
    assert!(!cache.entries.contains_key(&gone));
    assert!(cache.entries.contains_key(&excluded));

    // A new mtime re-hashes (the size is not checked: same content, same size).
    set_mtime(&abc, 1_600_000_000_000);
    let third = build(&repo, &mut cache);
    assert_eq!(third.rehashed, ["abc.txt"]);
}

#[test]
fn unreadable_file_fails_without_touching_a3s() {
    use std::os::unix::fs::PermissionsExt as _;
    let tmp = tempfile::tempdir().unwrap();
    if !permissions_enforced(tmp.path()) {
        return;
    }
    let root = tmp.path().join("repo");
    write(&root, "ok.txt", b"ok");
    let repo = repository(&root, ProtocolType::Ftp);
    build(&repo, &mut Sha1Cache::default());
    let before = a3s_bytes(&root);
    let secret = write(&root, "secret.txt", b"secret");
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    let run = run_with(
        &repo,
        &mut Sha1Cache::default(),
        DATE + 1,
        &AtomicBool::new(false),
    );
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        matches!(run.outcome, Err(BuildError::Hash { ref path, .. }) if *path == secret),
        "{:?}",
        run.outcome
    );
    assert_eq!(a3s_bytes(&root), before);
}

#[test]
fn revision_table() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let file = write(&root, "@m/addons/a.pbo", b"one");
    let mut repo = repository(&root, ProtocolType::Ftp);
    let mut cache = Sha1Cache::default();
    let fresh = build(&repo, &mut cache);
    assert_eq!(fresh.server_info.revision, 1);
    assert!(fresh.server_info.repository_content_updated);

    let unchanged = build(&repo, &mut cache);
    assert_eq!(unchanged.server_info.revision, 1);
    assert!(!unchanged.server_info.repository_content_updated);

    fs::write(&file, b"two").unwrap();
    set_mtime(&file, 1_600_000_000_000);
    let modified = build(&repo, &mut cache);
    assert_eq!(modified.server_info.revision, 2);

    // Neither compression nor hidden folders change the content.
    repo.compressed = true;
    repo.excluded_folders_from_sync = [root.join("@m").to_string_lossy().into_owned()].into();
    let settings = build(&repo, &mut cache);
    assert_eq!(settings.server_info.revision, 2);
    assert!(!settings.content_updated);

    // Without sync (but with serverinfo): content counts as changed.
    fs::remove_file(root.join(".a3s/sync")).unwrap();
    assert_eq!(build(&repo, &mut cache).server_info.revision, 3);

    // Without serverinfo the revision is 1, whatever else exists.
    fs::remove_file(root.join(".a3s/serverinfo")).unwrap();
    let reset = build(&repo, &mut cache);
    assert_eq!(reset.server_info.revision, 1);
}

#[test]
fn server_info_fields() {
    let sep = std::path::MAIN_SEPARATOR;
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().display().to_string();
    // The main folder name is matched case-insensitively.
    let root = tmp.path().join("Repo5X");
    write(&root, "docs/hidden/a.txt", b"1");
    let mut repo = repository(&root, ProtocolType::Ftp);
    repo.number_of_connections = 0;
    repo.set_use_partial_file_transfer(false);
    repo.compressed = true;
    repo.excluded_folders_from_sync = [
        format!("{base}{sep}repo5x{sep}docs{sep}hidden"),
        // Backslashes become slashes.
        format!("{base}{sep}Repo5X{sep}x\\y"),
        // First-occurrence prefix quirk: `/repo5x` matches inside `/Repo5X_other`.
        format!("{base}{sep}Repo5X_other{sep}Repo5X{sep}z"),
    ]
    .into();
    let outcome = build(&repo, &mut Sha1Cache::default());
    let info = outcome.server_info;
    assert_eq!(info.number_of_connections, 0, "raw value");
    assert!(info.no_partial_file_transfer);
    assert!(!info.compressed_pbo_files_only);
    assert_eq!(info.build_date, Some(DATE));
    assert_eq!(
        info.hidden_folder_paths
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["docs/hidden", "other/Repo5X/z", "x/y"]
    );
    assert!(
        !subdir(&outcome.sync, "docs").hidden,
        "hidden paths do not change the tree"
    );

    // An index past the end fails like Java's substring.
    repo.excluded_folders_from_sync = [format!("{sep}x")].into();
    let run = run_with(
        &repo,
        &mut Sha1Cache::default(),
        DATE,
        &AtomicBool::new(false),
    );
    assert!(
        matches!(run.outcome, Err(BuildError::HiddenFolder { ref path }) if *path == format!("{sep}x"))
    );
}

#[test]
fn auto_config() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "a.txt", b"1");
    let mut repo = repository(&root, ProtocolType::Https);
    let server = |name: &str, repository: &str| FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some("192.0.2.1".to_owned()),
        port: 2302,
        password: None,
        selected: false,
        modset_name: None,
        repository_name: Some(repository.to_owned()),
    };
    repo.favorite_servers_set_to_autoconfig = vec![
        server("B", "Test repo"),
        server("A", "Other"),
        server("C", "Test repo"),
    ];
    let outcome = build(&repo, &mut Sha1Cache::default());
    let expected = AutoConfig {
        repository_name: Some("Test repo".to_owned()),
        protocol: Some(repo.protocol.clone()),
        favorite_servers: repo.favorite_servers_set_to_autoconfig.clone(),
    };
    assert_eq!(outcome.auto_config, expected);
    assert_eq!(
        read_a3s::<AutoConfig>(&root.join(".a3s/autoconfig")).unwrap(),
        expected
    );

    // R5: the favorites with this repository's name and the autoconfig share one String.
    let graph = a3s_jserial::read(&gunzip(&root.join(".a3s/autoconfig"))).unwrap();
    let auto = graph.root_value().unwrap().as_ref().unwrap();
    let favorites = a3s_jserial::jdk::list(&graph, graph.field(auto, "favoriteServers").unwrap())
        .unwrap()
        .unwrap();
    let name_of =
        |v: a3s_jserial::Value| graph.field(v.as_ref().unwrap(), "repositoryName").unwrap();
    let own = graph.field(auto, "repositoryName").unwrap();
    assert_eq!(name_of(favorites[0]), own);
    assert_eq!(name_of(favorites[2]), own);
    assert_ne!(name_of(favorites[1]), own);
}

fn expected_zsync(file: &Path, name: &str, url: &str, sha1: &str) -> Vec<u8> {
    let bytes = fs::read(file).unwrap();
    let mtime = a3s_hash::modified_millis(file).unwrap();
    a3s_zsync::ControlFile::generate(
        &bytes[..],
        bytes.len() as u64,
        name,
        &a3s_zsync::format_mtime(mtime),
        url,
        sha1,
    )
    .unwrap()
    .to_bytes()
}

#[test]
fn zsync_generation() {
    for (protocol_type, prompt) in [
        (ProtocolType::Http, "http://"),
        (ProtocolType::Https, "https://"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let pbo = write(&root, "@m/addons/a b.pbo", &vec![3u8; 20_000]);
        let txt = write(&root, "@m/readme.txt", b"hello");
        let mut repo = repository(&root, protocol_type);
        repo.compressed = true;
        let mut cache = Sha1Cache::default();
        let outcome = build(&repo, &mut cache);
        let sha1 = leaf(&outcome.sync, "@m/addons/a b.pbo")
            .sha1
            .clone()
            .unwrap();
        // No port (the protocol has 8080) and no URL encoding.
        let url = format!("{prompt}example.org/repo/@m/addons/a b.pbo");
        let zsync = root.join("@m/addons/a b.pbo.zsync");
        assert_eq!(
            fs::read(&zsync).unwrap(),
            expected_zsync(&pbo, "a b.pbo", &url, &sha1)
        );
        assert_eq!(outcome.zsync_generated, 2);
        // The compression stage's zip is not a leaf, so it has no .zsync.
        assert!(root.join("@m/addons/a b.pbo.zip").exists());
        assert!(!root.join("@m/addons/a b.pbo.zip.zsync").exists());

        // Unchanged files keep their .zsync; an empty .zsync is regenerated.
        set_mtime(&zsync, 1_000_000_000_000);
        let txt_zsync = root.join("@m/readme.txt.zsync");
        fs::write(&txt_zsync, b"").unwrap();
        let second = build(&repo, &mut cache);
        assert_eq!(second.zsync_generated, 1);
        assert_eq!(
            a3s_hash::modified_millis(&zsync).unwrap(),
            1_000_000_000_000
        );
        let txt_sha1 = leaf(&second.sync, "@m/readme.txt").sha1.clone().unwrap();
        assert_eq!(
            fs::read(&txt_zsync).unwrap(),
            expected_zsync(
                &txt,
                "readme.txt",
                &format!("{prompt}example.org/repo/@m/readme.txt"),
                &txt_sha1
            )
        );
    }

    // Non-HTTP repositories leave existing .zsync files alone (and create none).
    for protocol_type in [ProtocolType::Ftp, ProtocolType::HttpWebdav] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        write(&root, "a.txt", b"a");
        write(&root, "a.txt.zsync", b"old");
        write(&root, "orphan.zsync", b"old");
        let run = run_with(
            &repository(&root, protocol_type),
            &mut Sha1Cache::default(),
            DATE,
            &AtomicBool::new(false),
        );
        assert_eq!(run.outcome.unwrap().zsync_generated, 0);
        assert!(
            run.events
                .contains(&BuildEvent::Stage(a3s_admin::build::STAGE_DELETE_ZSYNC))
        );
        assert_eq!(fs::read(root.join("a.txt.zsync")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("orphan.zsync")).unwrap(), b"old");
    }
}

fn unzip_single(zip_path: &Path) -> (String, Vec<u8>) {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path).unwrap()).unwrap();
    assert_eq!(archive.len(), 1);
    let mut entry = archive.by_index(0).unwrap();
    assert_eq!(entry.compression(), zip::CompressionMethod::Deflated);
    let name = entry.name().unwrap().into_owned();
    let mut content = Vec::new();
    entry.read_to_end(&mut content).unwrap();
    (name, content)
}

#[test]
fn zip_compression_reuse_and_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let pbo = write(&root, "@m/addons/a.pbo", &vec![5u8; 50_000]);
    let ebo = write(&root, "@m/addons/b.EBO", &vec![6u8; 30_000]);
    write(&root, "@m/addons/c.txt", b"not compressible");
    write(&root, "@m/addons/orphan.pbo.zip", b"orphan");
    let mut repo = repository(&root, ProtocolType::Ftp);
    repo.compressed = true;
    let mut cache = Sha1Cache::default();
    let outcome = build(&repo, &mut cache);
    assert_eq!(outcome.zipped, 2);
    for (source, rel) in [(&pbo, "@m/addons/a.pbo"), (&ebo, "@m/addons/b.EBO")] {
        let zip_path = root.join(format!("{rel}.zip"));
        let (name, content) = unzip_single(&zip_path);
        assert_eq!(name, source.file_name().unwrap().to_string_lossy());
        assert!(content == fs::read(source).unwrap());
        let l = leaf(&outcome.sync, rel);
        assert!(l.compressed);
        assert_eq!(
            l.compressed_size,
            i64::try_from(fs::metadata(&zip_path).unwrap().len()).unwrap()
        );
    }
    let written: SyncTreeDirectory = read_a3s(&root.join(".a3s/sync")).unwrap();
    assert_eq!(written, outcome.sync);
    let txt = leaf(&outcome.sync, "@m/addons/c.txt");
    assert!(!txt.compressed);
    assert_eq!(txt.compressed_size, 0);

    // Reused without rewriting unless the source was re-hashed.
    let zip_a = root.join("@m/addons/a.pbo.zip");
    let zip_b = root.join("@m/addons/b.EBO.zip");
    set_mtime(&zip_a, 1_000_000_000_000);
    set_mtime(&zip_b, 1_000_000_000_000);
    fs::write(&ebo, vec![7u8; 30_000]).unwrap();
    set_mtime(&ebo, 1_600_000_000_000);
    let second = build(&repo, &mut cache);
    assert_eq!(second.zipped, 1);
    assert_eq!(
        a3s_hash::modified_millis(&zip_a).unwrap(),
        1_000_000_000_000
    );
    assert_ne!(
        a3s_hash::modified_millis(&zip_b).unwrap(),
        1_000_000_000_000
    );
    assert!(unzip_single(&zip_b).1 == vec![7u8; 30_000]);

    // Uncompressed: zips of leaves deleted, flags cleared, orphan zips kept.
    repo.compressed = false;
    let third = build(&repo, &mut cache);
    assert_eq!(third.zips_deleted, 2);
    assert!(!zip_a.exists() && !zip_b.exists());
    assert!(root.join("@m/addons/orphan.pbo.zip").exists());
    assert!(!leaf(&third.sync, "@m/addons/a.pbo").compressed);
    assert!(!leaf(&third.sync, "@m/addons/b.EBO").compressed);
}

#[test]
fn outcome_equals_written_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "@m/addons/a.pbo", b"a");
    write(&root, "@m/addons/empty", b"");
    let repo = repository(&root, ProtocolType::Http);
    let run = run_with(
        &repo,
        &mut Sha1Cache::default(),
        DATE,
        &AtomicBool::new(false),
    );
    let outcome = run.outcome.unwrap();
    let dir = root.join(".a3s");
    assert_eq!(
        read_a3s::<SyncTreeDirectory>(&dir.join("sync")).unwrap(),
        outcome.sync
    );
    assert_eq!(
        read_a3s::<ServerInfo>(&dir.join("serverinfo")).unwrap(),
        outcome.server_info
    );
    assert_eq!(
        read_a3s::<Changelogs>(&dir.join("changelogs")).unwrap(),
        outcome.changelogs
    );
    assert_eq!(
        read_a3s::<AutoConfig>(&dir.join("autoconfig")).unwrap(),
        outcome.auto_config
    );
    assert_eq!(
        read_a3s::<Events>(&dir.join("events")).unwrap(),
        outcome.events
    );
    assert!(outcome.events.list.is_empty());
    let stages: Vec<&BuildEvent> = run
        .events
        .iter()
        .filter(|e| matches!(e, BuildEvent::Stage(_)))
        .collect();
    assert_eq!(
        stages,
        [
            &BuildEvent::Stage(STAGE_SHA1),
            &BuildEvent::Stage(a3s_admin::build::STAGE_ZSYNC),
            &BuildEvent::Stage(a3s_admin::build::STAGE_DELETE_ZIP),
        ]
    );
    assert!(run.events.contains(&BuildEvent::Progress(100)));
}

#[test]
fn cancel_during_sha1_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "a.txt", b"a");
    let repo = repository(&root, ProtocolType::Ftp);
    build(&repo, &mut Sha1Cache::default());
    let before = a3s_bytes(&root);
    write(&root, "b.txt", b"b");
    let cancel = AtomicBool::new(false);
    let outcome = build_repository(
        &repo,
        &mut Sha1Cache::default(),
        &BuildOptions {
            build_date: Some(DATE + 1),
        },
        &cancel,
        &|e| {
            if e == BuildEvent::Stage(STAGE_SHA1) {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(matches!(outcome, Err(BuildError::Cancelled)), "{outcome:?}");
    assert_eq!(a3s_bytes(&root), before);
}

#[test]
fn zip_failure_writes_nothing() {
    use std::os::unix::fs::PermissionsExt as _;
    let tmp = tempfile::tempdir().unwrap();
    if !permissions_enforced(tmp.path()) {
        return;
    }
    let root = tmp.path().join("repo");
    write(&root, "a.txt", b"a");
    let mut repo = repository(&root, ProtocolType::Ftp);
    build(&repo, &mut Sha1Cache::default());
    let before = a3s_bytes(&root);
    let locked = root.join("@m/addons");
    write(&root, "@m/addons/a.pbo", b"pbo");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    repo.compressed = true;
    let run = run_with(
        &repo,
        &mut Sha1Cache::default(),
        DATE + 1,
        &AtomicBool::new(false),
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(run.outcome, Err(BuildError::Zip { .. })),
        "{:?}",
        run.outcome
    );
    assert_eq!(a3s_bytes(&root), before);
}

#[test]
fn missing_or_unset_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mut repo = repository(&tmp.path().join("nope"), ProtocolType::Ftp);
    assert!(
        run_with(
            &repo,
            &mut Sha1Cache::default(),
            DATE,
            &AtomicBool::new(false),
        )
        .outcome
        .is_err()
    );
    repo.path = Some(String::new());
    assert!(
        run_with(
            &repo,
            &mut Sha1Cache::default(),
            DATE,
            &AtomicBool::new(false),
        )
        .outcome
        .is_err()
    );
}

#[test]
fn rebuilds_are_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "@m/addons/a.pbo", &[1u8; 9000]);
    write(&root, "@m/addons/e.txt", b"");
    write(&root, "@n/addons/b.ebo", &[2u8; 100]);
    let mut repo = repository(&root, ProtocolType::Http);
    repo.compressed = true;
    let mut cache = Sha1Cache::default();
    build(&repo, &mut cache);
    let first = streams(&root);
    build(&repo, &mut cache);
    let second = streams(&root);
    build(&repo, &mut cache);
    let third = streams(&root);
    assert_eq!(second, third);
    for file in ["sync", "changelogs", "autoconfig", "events"] {
        assert_eq!(first[file], second[file], "{file}");
    }
    let info = |raw: &[u8]| a3s_types::decode_stream::<ServerInfo>(raw).unwrap();
    let (a, b) = (info(&first["serverinfo"]), info(&second["serverinfo"]));
    // Besides `totalFilesSize` (the first build's outputs), only the first build reports a
    // content change.
    assert!(b.total_files_size > a.total_files_size);
    assert!(a.repository_content_updated && !b.repository_content_updated);
    let normalized = |info: ServerInfo| ServerInfo {
        total_files_size: 0,
        repository_content_updated: false,
        ..info
    };
    assert_eq!(normalized(a), normalized(b));
}

#[test]
fn streams_pass_java_limits_and_deep_trees_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "@m/addons/a.pbo", b"a");
    let repo = repository(&root, ProtocolType::Http);
    build(&repo, &mut Sha1Cache::default());
    for (name, raw) in streams(&root) {
        a3s_jserial::read_with_limits(&raw, a3s_jserial::Limits::JAVA_A3S)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    let deep_root = tmp.path().join("deep");
    let mut rel = String::new();
    for i in 0..60 {
        rel.push_str(&format!("d{i}/"));
    }
    write(&deep_root, &format!("{rel}f.txt"), b"x");
    let run = run_with(
        &repository(&deep_root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
        DATE,
        &AtomicBool::new(false),
    );
    assert!(
        matches!(
            run.outcome,
            Err(BuildError::LimitExceeded {
                file: A3sFile::Sync,
                ..
            })
        ),
        "{:?}",
        run.outcome
    );
    assert!(!deep_root.join(".a3s").exists());
}

#[test]
fn apply_to_sets_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "a.txt", b"a");
    let mut repo = repository(&root, ProtocolType::Ftp);
    repo.revision = 41;
    let outcome = build(&repo, &mut Sha1Cache::default());
    outcome.apply_to(&mut repo);
    assert_eq!(repo.revision, 1);
}
