//! Adversarial verification of WP11 (repository build): limits at Java's reader boundaries,
//! cancellation followed by a rebuild, file permissions of generated files, symbolic links,
//! Unicode / case-colliding names and rebuild idempotence. The `#[ignore]`d tests need JDK 25
//! (`source scripts/java_env.sh`) and check that Java reads what Rust wrote.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_admin::build::{
    BuildError, BuildEvent, BuildOptions, BuildOutcome, STAGE_SHA1, STAGE_ZIP, STAGE_ZSYNC,
    build_repository,
};
use a3s_hash::Sha1Cache;
use a3s_repo_files::A3sFile;
use a3s_types::local::Repository;
use a3s_types::{Protocol, ProtocolType, ServerInfo, SyncTreeDirectory, SyncTreeNode};

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
        "80",
        "anonymous",
        "",
        false,
    );
    let mut repo = Repository::new("Verify repo", protocol);
    repo.path = Some(root.to_string_lossy().into_owned());
    repo
}

fn run(
    repo: &Repository,
    cache: &mut Sha1Cache,
    cancel: &AtomicBool,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Result<BuildOutcome, BuildError> {
    build_repository(
        repo,
        cache,
        &BuildOptions {
            build_date: Some(DATE),
        },
        cancel,
        on_event,
    )
}

fn build(repo: &Repository, cache: &mut Sha1Cache) -> BuildOutcome {
    run(repo, cache, &AtomicBool::new(false), &|_| {}).unwrap()
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(fs::File::open(path).unwrap())
        .read_to_end(&mut out)
        .unwrap();
    out
}

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

/// Every file below `dir` (relative, `/`-joined).
fn all_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(
                    path.strip_prefix(dir)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    out.sort();
    out
}

fn unzip_single(zip_path: &Path) -> (String, Vec<u8>) {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path).unwrap()).unwrap();
    assert_eq!(archive.len(), 1);
    let mut entry = archive.by_index(0).unwrap();
    let name = entry.name().unwrap().into_owned();
    let mut content = Vec::new();
    entry.read_to_end(&mut content).unwrap();
    (name, content)
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

fn leaf_sha1(sync: &SyncTreeDirectory, path: &str) -> Option<String> {
    let mut found = None;
    sync.walk(&mut |p, node| {
        if p == path
            && let SyncTreeNode::Leaf(l) = node
        {
            found = Some(l.sha1.clone());
        }
    });
    found.expect(path)
}

fn nested(depth: usize) -> String {
    let mut rel = String::new();
    for i in 0..depth {
        rel.push_str(&format!("d{i}/"));
    }
    rel.push_str("f.txt");
    rel
}

/// Java's reader filter (`A3SFilesAccessor.LEGACY_OBJECT_FILTER`) rejects a stream once
/// `ObjectInputStream.totalObjectRefs` passes 250 000. That counter is incremented by **every**
/// `readObject0` call (new objects, strings, back-references and nulls alike), so a leaf costs 4
/// (object, name, parent back-reference, sha1) where the Rust check counts 3 handles. A flat
/// folder of 70 000 files is ~280 000 Java references: Java cannot read the sync file, so the
/// build must fail with `LimitExceeded` and write nothing (Q6, A16).
#[test]
fn verify_70k_files_exceed_java_reference_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    for d in 0..70 {
        for f in 0..1000 {
            write(&root, &format!("@m{d}/f{f}.txt"), b"x");
        }
    }
    let outcome = run(
        &repository(&root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
        &AtomicBool::new(false),
        &|_| {},
    );
    assert!(
        matches!(
            outcome,
            Err(BuildError::LimitExceeded {
                file: A3sFile::Sync,
                ..
            })
        ),
        "Java rejects > 250 000 references (~4 per leaf); got {:?}",
        outcome.as_ref().map(|o| o.server_info.number_of_files)
    );
    assert!(!root.join(".a3s").exists());
}

/// Depth boundary: Java's depth reaches 2N+4 for a leaf below N nested folders, so 48 folders
/// are readable and 49 are not.
#[test]
fn verify_depth_boundary_48_ok_49_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let ok_root = tmp.path().join("ok");
    write(&ok_root, &nested(48), b"x");
    build(
        &repository(&ok_root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
    );
    for (name, raw) in streams(&ok_root) {
        a3s_jserial::read_with_limits(&raw, a3s_jserial::Limits::JAVA_A3S)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    let bad_root = tmp.path().join("bad");
    write(&bad_root, &nested(49), b"x");
    let outcome = run(
        &repository(&bad_root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
        &AtomicBool::new(false),
        &|_| {},
    );
    assert!(
        matches!(
            outcome,
            Err(BuildError::LimitExceeded {
                file: A3sFile::Sync,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert!(!bad_root.join(".a3s").exists());
}

/// Cancelling after the SHA-1 stage has hashed a modified file, then rebuilding with the same
/// (session) cache, must not leave a stale `.zip`/`.zsync` behind a new SHA-1. In Java a cancel
/// during the SHA-1 stage only stops that stage: the zsync and zip stages still run for the
/// re-hashed files in the same build (`RepositoryBuildProcessor.java:283-317`, `cancel()` at
/// :619-636 only reaches processors that already exist).
#[test]
fn verify_cancel_after_hash_then_rebuild_has_fresh_zip_and_zsync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    let pbo = write(&root, "@m/addons/a.pbo", &vec![1u8; 50_000]);
    let mut repo = repository(&root, ProtocolType::Http);
    repo.compressed = true;
    let mut cache = Sha1Cache::default();
    build(&repo, &mut cache);

    fs::write(&pbo, vec![2u8; 60_000]).unwrap();
    set_mtime(&pbo, 1_600_000_000_000);
    let cancel = AtomicBool::new(false);
    let in_sha1 = AtomicBool::new(false);
    let outcome = run(&repo, &mut cache, &cancel, &|e| match e {
        BuildEvent::Stage(s) => in_sha1.store(s == STAGE_SHA1, Ordering::SeqCst),
        BuildEvent::Progress(100) if in_sha1.load(Ordering::SeqCst) => {
            cancel.store(true, Ordering::SeqCst);
        }
        _ => {}
    });
    assert!(matches!(outcome, Err(BuildError::Cancelled)), "{outcome:?}");

    let rebuilt = build(&repo, &mut cache);
    let sha1 = leaf_sha1(&rebuilt.sync, "@m/addons/a.pbo").unwrap();
    let (_, content) = unzip_single(&root.join("@m/addons/a.pbo.zip"));
    assert!(
        content == fs::read(&pbo).unwrap(),
        "stale a.pbo.zip reused after a cancelled build (sync says sha1 {sha1})"
    );
    assert_eq!(
        fs::read(root.join("@m/addons/a.pbo.zsync")).unwrap(),
        expected_zsync(
            &pbo,
            "a.pbo",
            "http://example.org/repo/@m/addons/a.pbo",
            &sha1
        ),
        "stale a.pbo.zsync after a cancelled build"
    );
}

/// Cancelling in the middle of the zip stage leaves the previous `.a3s` files byte-identical, no
/// temporary files, and only complete zips.
#[test]
fn verify_cancel_mid_zip_and_mid_zsync_leaves_no_partial_files() {
    for stage in [STAGE_ZSYNC, STAGE_ZIP] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        write(&root, "keep.txt", b"keep");
        let mut repo = repository(&root, ProtocolType::Http);
        repo.compressed = true;
        build(&repo, &mut Sha1Cache::default());
        let before = a3s_bytes(&root);
        for i in 0..6 {
            let bytes: Vec<u8> = (0..400_000u32)
                .map(|x| (x.wrapping_mul(2_654_435_761).wrapping_add(i) >> 13) as u8)
                .collect();
            write(&root, &format!("@m/addons/p{i}.pbo"), &bytes);
        }
        let cancel = AtomicBool::new(false);
        let in_stage = AtomicBool::new(false);
        let outcome = run(&repo, &mut Sha1Cache::default(), &cancel, &|e| match e {
            BuildEvent::Stage(s) => in_stage.store(s == stage, Ordering::SeqCst),
            BuildEvent::Progress(_) if in_stage.load(Ordering::SeqCst) => {
                cancel.store(true, Ordering::SeqCst);
            }
            _ => {}
        });
        assert!(
            matches!(outcome, Err(BuildError::Cancelled)),
            "{stage}: {outcome:?}"
        );
        assert_eq!(a3s_bytes(&root), before, "{stage}");
        for rel in all_files(&root.join("@m/addons")) {
            assert!(
                rel.ends_with(".pbo") || rel.ends_with(".zip") || rel.ends_with(".zsync"),
                "{stage}: stray file {rel}"
            );
            if let Some(source) = rel.strip_suffix(".zip") {
                let (_, content) = unzip_single(&root.join("@m/addons").join(&rel));
                assert!(content == fs::read(root.join("@m/addons").join(source)).unwrap());
            }
        }
    }
}

/// `.zsync` (`MetaFileMaker`: `FileWriter`/`FileOutputStream`) and `.zip` (`ZipDAO`:
/// `FileOutputStream`) are created by Java with the process umask (typically 0644), so a web
/// server running as another user can serve them. Temporary files from `tempfile` are 0600.
#[test]
fn verify_generated_files_have_umask_permissions() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, "@m/addons/a.pbo", &vec![1u8; 10_000]);
    let mut repo = repository(&root, ProtocolType::Http);
    repo.compressed = true;
    build(&repo, &mut Sha1Cache::default());
    let probe = tmp.path().join("probe");
    fs::File::create(&probe).unwrap();
    let expected = fs::metadata(&probe).unwrap().permissions().mode() & 0o777;
    for rel in ["@m/addons/a.pbo.zip", "@m/addons/a.pbo.zsync"] {
        let mode = fs::metadata(root.join(rel)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, expected, "{rel}: {mode:o} vs Java {expected:o}");
    }
}

/// `Files.walk` does not follow a symbolic link given as its start (`NOFOLLOW_LINKS` attributes),
/// so Java reports `totalFilesSize = 0` for a repository whose path is a link to a folder, while
/// `File.listFiles` still scans it (checked with JDK 25).
#[test]
fn verify_symlinked_root_total_files_size_like_java() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    write(&real, "sub/a.txt", b"hello");
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let outcome = build(
        &repository(&link, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
    );
    assert_eq!(outcome.server_info.number_of_files, 1);
    assert_eq!(
        outcome.server_info.total_files_size, 0,
        "Java: Files.walk(link) yields only the link itself"
    );
}

/// Inside the repository, linked folders are scanned but not counted by `totalFilesSize`; linked
/// files are counted with their target's size. A cycle through two sibling links fails cleanly.
#[test]
fn verify_inner_symlinks_and_sibling_cycles() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    write(&outside, "big.bin", &[0u8; 100]);
    write(&outside, "f.bin", &[0u8; 50]);
    let root = tmp.path().join("repo");
    write(&root, "real.txt", &[0u8; 7]);
    std::os::unix::fs::symlink(&outside, root.join("linkdir")).unwrap();
    std::os::unix::fs::symlink(outside.join("f.bin"), root.join("lf.bin")).unwrap();
    let outcome = build(
        &repository(&root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
    );
    assert_eq!(outcome.server_info.number_of_files, 4);
    assert_eq!(outcome.server_info.total_files_size, 7 + 50);

    let cyc = tmp.path().join("cyc");
    write(&cyc, "a/x.txt", b"x");
    write(&cyc, "b/y.txt", b"y");
    std::os::unix::fs::symlink(cyc.join("b"), cyc.join("a/l")).unwrap();
    std::os::unix::fs::symlink(cyc.join("a"), cyc.join("b/l")).unwrap();
    let outcome = run(
        &repository(&cyc, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
        &AtomicBool::new(false),
        &|_| {},
    );
    assert!(
        matches!(outcome, Err(BuildError::Scan { .. })),
        "{outcome:?}"
    );
    assert!(!cyc.join(".a3s").exists());
}

/// Unicode (incl. supplementary characters), spaces, names that differ only in case, several
/// empty files (some cached, some new), a hidden folder and an excluded file. Rebuilds without
/// a change keep the revision and the streams.
fn unicode_repo(root: &Path) -> Repository {
    write(root, "@Mod/addons/Ünïcødé ✓.pbo", &vec![3u8; 3000]);
    write(root, "@Mod/addons/🎮 game.pbo", &vec![4u8; 2000]);
    write(root, "@mod/Addons/x.EBO", &vec![5u8; 1000]);
    write(root, "with space/Case.txt", b"upper");
    write(root, "with space/case.txt", b"lower");
    write(root, "with space/empty1", b"");
    write(root, "with space/empty2", b"");
    write(root, "deep/e/empty3", b"");
    write(root, "deep/e/excluded.txt", b"excluded");
    write(root, "hidden/h.txt", b"hidden");
    let mut repo = repository(root, ProtocolType::Http);
    repo.compressed = true;
    repo.excluded_files_from_build = [root
        .join("deep/e/excluded.txt")
        .to_string_lossy()
        .into_owned()]
    .into();
    repo.excluded_folders_from_sync = [root.join("hidden").to_string_lossy().into_owned()].into();
    repo
}

#[test]
fn verify_unicode_case_and_empty_files_rebuild_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("répo ✓");
    let repo = unicode_repo(&root);
    let mut cache = Sha1Cache::default();
    let first = build(&repo, &mut cache);
    assert_eq!(first.server_info.revision, 1);
    assert_eq!(first.server_info.number_of_files, 9);
    assert_eq!(
        first
            .server_info
            .hidden_folder_paths
            .iter()
            .collect::<Vec<_>>(),
        ["hidden"]
    );
    let names: Vec<&str> = first.sync.children.iter().map(SyncTreeNode::name).collect();
    assert_eq!(names, ["@Mod", "@mod", "deep", "hidden", "with space"]);
    for path in ["with space/empty1", "with space/empty2", "deep/e/empty3"] {
        assert_eq!(leaf_sha1(&first.sync, path).as_deref(), Some("0"));
    }
    assert_eq!(first.changelogs.list.len(), 1);
    assert_eq!(first.changelogs.list[0].addons, ["@Mod", "@mod"]);
    let s1 = streams(&root);
    for (name, raw) in &s1 {
        a3s_jserial::read_with_limits(raw, a3s_jserial::Limits::JAVA_A3S)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    let second = build(&repo, &mut cache);
    let s2 = streams(&root);
    let third = build(&repo, &mut cache);
    let s3 = streams(&root);
    assert!(second.rehashed.is_empty() && third.rehashed.is_empty());
    assert_eq!(second.server_info.revision, 1);
    assert_eq!(third.server_info.revision, 1);
    assert!(!second.content_updated && !third.content_updated);
    assert_eq!(s2, s3);
    for file in ["sync", "changelogs", "autoconfig", "events"] {
        assert!(s1[file] == s2[file], "{file} changed on a no-op rebuild");
    }

    // Add two more empty files next to the cached ones: one content change, one revision.
    write(&root, "with space/empty4", b"");
    write(&root, "deep/empty5", b"");
    let fourth = build(&repo, &mut cache);
    assert_eq!(fourth.server_info.revision, 2);
    assert_eq!(fourth.server_info.number_of_files, 11);
    let raw = gunzip(&root.join(".a3s/sync"));
    let decoded: SyncTreeDirectory = a3s_types::decode_stream(&raw).unwrap();
    assert_eq!(decoded, fourth.sync);
    let info: ServerInfo = a3s_repo_files::read_a3s(&root.join(".a3s/serverinfo")).unwrap();
    assert_eq!(info, fourth.server_info);
}

/// Copies `repo` (without `.zsync` files unless the local time zone is UTC, since the harness
/// regenerates them in UTC) into `out/<name>` for `Harness::verify`.
fn snapshot(repo: &Path, out: &Path) {
    let utc = a3s_zsync::format_mtime(0).ends_with("+0000");
    for rel in all_files(repo) {
        if !utc && rel.ends_with(".zsync") {
            continue;
        }
        let target = out.join(&rel);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(repo.join(&rel), &target).unwrap();
        let mtime = a3s_hash::modified_millis(&repo.join(&rel)).unwrap();
        set_mtime(&target, mtime);
    }
}

fn java_verify(dir: &Path) {
    let harness = a3s_dev_tools::Harness::locate().unwrap();
    let report = harness
        .verify(
            &[dir.to_path_buf()],
            None,
            a3s_dev_tools::DumpMode::Identity,
        )
        .unwrap();
    assert!(report.is_success(), "{report:?}");
    assert!(report.ok.len() >= 5, "{report:?}");
}

/// Java reads the Unicode/case/empty-file repository and unzips its zips (`ZipDAO.unZip`).
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_java_reads_unicode_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("répo ✓");
    let repo = unicode_repo(&root);
    let mut cache = Sha1Cache::default();
    build(&repo, &mut cache);
    write(&root, "with space/empty4", b"");
    build(&repo, &mut cache);
    let out = tmp.path().join("snapshot");
    snapshot(&root, &out);
    java_verify(&out);
}

/// Java reads the deepest tree Rust accepts (48 nested folders).
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_java_reads_deepest_accepted_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    write(&root, &nested(48), b"x");
    build(
        &repository(&root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
    );
    let out = tmp.path().join("snapshot");
    snapshot(&root, &out);
    java_verify(&out);
}

/// Whatever number of files Rust accepts, Java must be able to read the sync file.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn verify_java_reads_largest_accepted_flat_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    for d in 0..70 {
        for f in 0..1000 {
            write(&root, &format!("@m{d}/f{f}.txt"), b"x");
        }
    }
    let outcome = run(
        &repository(&root, ProtocolType::Ftp),
        &mut Sha1Cache::default(),
        &AtomicBool::new(false),
        &|_| {},
    );
    if outcome.is_err() {
        return; // Rejected before writing: nothing Java could fail to read.
    }
    let out = tmp.path().join("snapshot/.a3s");
    fs::create_dir_all(&out).unwrap();
    for file in A3sFile::ALL {
        fs::copy(
            root.join(".a3s").join(file.file_name()),
            out.join(file.file_name()),
        )
        .unwrap();
    }
    java_verify(&tmp.path().join("snapshot"));
}
