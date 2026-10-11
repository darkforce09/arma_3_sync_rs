//! Golden tests: replays the Java-built scenarios of `tests/fixtures/java_built/` with the Rust
//! builder and compares the results with Java's output.
//!
//! Each variant (`http_plain`, `http_zip`, `ftp_plain`, `ftp_zip`) is rebuilt step by step from the
//! same input tree (source files, mtimes, empty directories, the step 3 `events` input) with the
//! same build dates. The decompressed `.a3s` streams and the `.zsync` files must equal Java's
//! byte for byte, with these documented adjustments of the expected Java bytes:
//!
//! * `compressedSize` and `totalFilesSize` depend on the size of the zips each implementation
//!   wrote (semantic compatibility only): the Java values are replaced with the sizes of the
//!   Rust zips before comparing;
//! * `.zsync` `MTime:` lines are rendered in the local time zone (the harness pins UTC);
//! * Java turns its own `mod_b.ebo.zip` into a sync leaf on the next build (steps 2 and 3 of
//!   the zip variants: an extra leaf, one revision more, an extra changelog, an extra
//!   `.ebo.zip.zsync`). Rust excludes it, so for those steps the expected sync is Java's without
//!   that leaf, the revision and file count are one lower, and the changelogs are those of the
//!   corresponding plain variant (which has no such quirk).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use a3s_admin::build::{BuildEvent, BuildOptions, build_repository};
use a3s_jserial::{Content, Graph, NodeId, Value};
use a3s_repo_files::hash::Sha1Cache;
use a3s_types::local::Repository;
use a3s_types::{FavoriteServer, Protocol, ProtocolType};
use parking_lot::Mutex;
use serde_json::Value as Json;

const A3S_FILES: [&str; 5] = ["sync", "serverinfo", "changelogs", "autoconfig", "events"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_built")
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn read_json(path: &Path) -> Json {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
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

/// Relative paths (with `/`) of every file below `root`, except the top-level `.a3s`.
fn files(root: &Path) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == ".a3s" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.insert(rel, fs::metadata(&path).unwrap().len());
            }
        }
    }
    out
}

fn is_generated(rel: &str) -> bool {
    rel.ends_with(".zsync") || rel.ends_with(".zip")
}

/// The local repository settings of a scenario (the Java harness's `setUpRepository`).
fn repository(scenario: &Json, repo_dir: &Path) -> Repository {
    let p = &scenario["protocol"];
    let protocol_type = ProtocolType::from_java_name(p["type"].as_str().unwrap()).unwrap();
    let protocol = Protocol::new(
        protocol_type,
        p["url"].as_str().unwrap(),
        p["port"].as_str().unwrap(),
        p["login"].as_str().unwrap(),
        p["password"].as_str().unwrap(),
        false,
    );
    let name = scenario["repository_name"].as_str().unwrap();
    let mut repo = Repository::new(name, protocol);
    repo.path = Some(repo_dir.to_string_lossy().into_owned());
    repo.compressed = scenario["compressed"].as_bool().unwrap();
    repo.set_use_partial_file_transfer(scenario["use_partial_file_transfer"].as_bool().unwrap());
    repo.number_of_connections =
        i32::try_from(scenario["number_of_connections"].as_i64().unwrap()).unwrap();
    let abs = |rel: &str| {
        let mut path = repo_dir.to_path_buf();
        path.extend(rel.split('/'));
        path.to_string_lossy().into_owned()
    };
    repo.excluded_files_from_build = scenario["excluded_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| abs(f.as_str().unwrap()))
        .collect();
    repo.excluded_folders_from_sync = scenario["hidden_folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| abs(f.as_str().unwrap()))
        .collect();
    let s = |v: &Json| v.as_str().map(str::to_owned);
    repo.favorite_servers_set_to_autoconfig = scenario["favorite_servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| FavoriteServer {
            name: s(&f["name"]),
            ip_address: s(&f["ip_address"]),
            port: i32::try_from(f["port"].as_i64().unwrap()).unwrap(),
            password: s(&f["password"]),
            selected: f["selected"].as_bool().unwrap(),
            modset_name: s(&f["modset_name"]),
            repository_name: s(&f["repository_name"]),
        })
        .collect();
    repo
}

/// Brings `repo_dir` to the input state of `step`: source files with their mtimes, removed
/// sources (with their generated siblings) deleted, empty directories as listed.
fn prepare_step(variant_dir: &Path, step: &Json, repo_dir: &Path) {
    let step_repo = variant_dir
        .join(step["name"].as_str().unwrap())
        .join("repo");
    let wanted: BTreeMap<String, &Json> = step["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap().to_owned(), f))
        .collect();
    let path_of = |rel: &str| {
        let mut path = repo_dir.to_path_buf();
        path.extend(rel.split('/'));
        path
    };
    // Deleted sources.
    if repo_dir.exists() {
        for rel in files(repo_dir).keys() {
            if !is_generated(rel) && !wanted.contains_key(rel) {
                let path = path_of(rel);
                fs::remove_file(&path).unwrap();
                for ext in [".zip", ".zsync"] {
                    let mut sibling = path.as_os_str().to_owned();
                    sibling.push(ext);
                    let _ignored = fs::remove_file(PathBuf::from(sibling));
                }
            }
        }
    }
    // New or modified sources.
    for (rel, f) in &wanted {
        let path = path_of(rel);
        let mtime = f["mtime_ms"].as_i64().unwrap();
        let size = f["size"].as_u64().unwrap();
        let current = fs::metadata(&path).ok();
        let unchanged = current.is_some_and(|m| {
            m.len() == size && a3s_repo_files::hash::modified_millis(&path).unwrap() == mtime
        });
        if !unchanged {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::copy(path_of_in(&step_repo, rel), &path).unwrap();
            set_mtime(&path, mtime);
        }
    }
    // Empty directories: remove the ones not listed (deepest first), create the listed ones.
    let dirs: BTreeSet<String> = step["dirs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap().to_owned())
        .collect();
    remove_empty_dirs(repo_dir, repo_dir, &dirs);
    for d in &dirs {
        fs::create_dir_all(path_of(d)).unwrap();
    }
    if let Some(events) = step["input_events"].as_str() {
        let a3s = repo_dir.join(".a3s");
        fs::create_dir_all(&a3s).unwrap();
        fs::copy(path_of_in(variant_dir, events), a3s.join("events")).unwrap();
    }
}

fn path_of_in(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(rel.split('/'));
    path
}

fn remove_empty_dirs(root: &Path, dir: &Path, keep: &BTreeSet<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() && path.file_name().unwrap() != ".a3s" {
            remove_empty_dirs(root, &path, keep);
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if fs::read_dir(&path).unwrap().next().is_none() && !keep.contains(&rel) {
                fs::remove_dir(&path).unwrap();
            }
        }
    }
}

// ------------------------------------------------------------------------------- graph surgery

/// Calls `f(relpath, leaf, parent_list)` for every leaf of a Java sync graph.
fn sync_leaves(graph: &Graph, dir: NodeId, prefix: &str, f: &mut impl FnMut(&str, NodeId, NodeId)) {
    let list_value = graph.field(dir, "list").unwrap();
    let list = list_value.as_ref().unwrap();
    for element in a3s_jserial::jdk::list(graph, list_value).unwrap().unwrap() {
        let id = element.as_ref().unwrap();
        let name = graph.field_string(id, "name").unwrap().unwrap();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if graph.class_name_of(id).unwrap() == a3s_types::wire::SYNC_TREE_LEAF {
            f(&rel, id, list);
        } else {
            sync_leaves(graph, id, &rel, f);
        }
    }
}

fn set_field(graph: &mut Graph, object: NodeId, name: &str, value: Value) {
    let class = graph.object(object).unwrap().class;
    let index = graph
        .class_desc(class)
        .unwrap()
        .fields
        .iter()
        .position(|f| f.name == name)
        .unwrap();
    graph.object_mut(object).unwrap().class_data[0].values[index] = value;
}

fn remove_list_element(graph: &mut Graph, list: NodeId, element: NodeId) {
    let elements: Vec<Value> = a3s_jserial::jdk::list(graph, Value::Ref(list))
        .unwrap()
        .unwrap()
        .into_iter()
        .filter(|v| *v != Value::Ref(element))
        .collect();
    let size = i32::try_from(elements.len()).unwrap();
    let data = &mut graph.object_mut(list).unwrap().class_data[0];
    data.values[0] = Value::Int(size);
    data.annotations = a3s_jserial::jdk::block_data(&size.to_be_bytes());
    data.annotations
        .extend(elements.into_iter().map(Content::Value));
}

// ------------------------------------------------------------------------------- replay

struct Variant {
    name: &'static str,
    dir: PathBuf,
    scenario: Json,
    generated: Json,
}

impl Variant {
    fn load(name: &'static str) -> Self {
        let dir = fixtures().join(name);
        Self {
            scenario: read_json(&dir.join("scenario.json")),
            generated: read_json(&dir.join("generated_files.json")),
            name,
            dir,
        }
    }

    fn zip(&self) -> bool {
        self.scenario["compressed"].as_bool().unwrap()
    }

    fn http(&self) -> bool {
        self.name.starts_with("http")
    }

    fn java_raw(&self, step: &str, file: &str) -> Vec<u8> {
        gunzip(&fs::read(self.dir.join(step).join("repo/.a3s").join(file)).unwrap())
    }
}

/// Mtimes of every source file of a step, by relative path.
fn source_mtimes(step: &Json) -> BTreeMap<String, i64> {
    step["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_owned(),
                f["mtime_ms"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// Java's `.zsync` with its `MTime:` line rendered in the local time zone.
fn localize_zsync(java: &[u8], mtime: i64) -> Vec<u8> {
    let split = java.windows(2).position(|w| w == b"\n\n").unwrap();
    let header = std::str::from_utf8(&java[..split]).unwrap();
    let header: Vec<String> = header
        .lines()
        .map(|l| {
            if l.starts_with("MTime: ") {
                format!("MTime: {}", a3s_zsync::format_mtime(mtime))
            } else {
                l.to_owned()
            }
        })
        .collect();
    let mut out = header.join("\n").into_bytes();
    out.extend_from_slice(&java[split..]);
    out
}

/// Replays every step of `variant` and checks the outputs. Returns the Rust repository folder of
/// each step's result (kept alive by the returned tempdir).
fn replay(variant: &Variant, mut check: impl FnMut(usize, &str, &Path)) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp
        .path()
        .join(variant.scenario["repository_folder"].as_str().unwrap());
    let repository = repository(&variant.scenario, &repo_dir);
    let mut cache = Sha1Cache::default();
    let steps = variant.scenario["steps"].as_array().unwrap();
    for (index, step) in steps.iter().enumerate() {
        let name = step["name"].as_str().unwrap();
        prepare_step(&variant.dir, step, &repo_dir);
        let before = files(&repo_dir);
        let events = Mutex::new(Vec::new());
        let options = BuildOptions {
            build_date: step["build_date_ms"].as_i64(),
        };
        let outcome = build_repository(
            &repository,
            &mut cache,
            &options,
            &AtomicBool::new(false),
            &|e| events.lock().push(e),
        )
        .unwrap_or_else(|e| panic!("{}/{name}: {e}", variant.name));
        assert!(
            events.lock().contains(&BuildEvent::Warning(
                "Warning: Removing empty directory from metadata: emptydir".to_owned()
            )),
            "{}/{name}",
            variant.name
        );
        assert_eq!(
            outcome.server_info.revision,
            gunzip_revision(variant, name, index)
        );
        check_step(variant, index, step, &repo_dir, &before);
        check(index, name, &repo_dir);
    }
    tmp
}

/// The revision Rust must reach: Java's, minus the bump its `.ebo.zip` leaf causes in the zip variants.
fn gunzip_revision(variant: &Variant, step: &str, index: usize) -> i32 {
    let java: a3s_types::ServerInfo =
        a3s_types::decode_stream(&variant.java_raw(step, "serverinfo")).unwrap();
    java.revision - i32::from(variant.zip() && index > 0)
}

fn check_step(
    variant: &Variant,
    index: usize,
    step: &Json,
    repo_dir: &Path,
    before: &BTreeMap<String, u64>,
) {
    let step_name = step["name"].as_str().unwrap();
    let ctx = format!("{}/{step_name}", variant.name);
    let d14 = variant.zip() && index > 0;
    let after = files(repo_dir);
    let a3s = repo_dir.join(".a3s");

    // Sizes of the Rust zips, by relative source path.
    let rust_zip_sizes: BTreeMap<String, i64> = after
        .iter()
        .filter_map(|(rel, size)| {
            rel.strip_suffix(".zip")
                .map(|src| (src.to_owned(), i64::try_from(*size).unwrap()))
        })
        .collect();

    // totalFilesSize: generated files present before the build, Java's vs Rust's.
    let rust_generated: i64 = before
        .iter()
        .filter(|(rel, _)| is_generated(rel))
        .map(|(_, s)| i64::try_from(*s).unwrap())
        .sum();
    let java_generated: i64 = if index == 0 {
        0
    } else {
        variant.generated["steps"][index - 1]["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| {
                let rel = f["path"].as_str().unwrap();
                let parent = Path::new(rel).parent().unwrap();
                repo_dir.join(parent).is_dir()
            })
            .map(|f| f["size"].as_i64().unwrap())
            .sum()
    };
    if !variant.zip() {
        assert_eq!(rust_generated, java_generated, "{ctx}: .zsync sizes");
    }

    for file in A3S_FILES {
        let rust_gz = fs::read(a3s.join(file)).unwrap();
        // Java's GZIPOutputStream header: mtime 0, XFL 0, OS 255.
        assert_eq!(
            &rust_gz[..10],
            &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff],
            "{ctx}/{file}: gzip header"
        );
        let rust = gunzip(&rust_gz);
        let expected = match file {
            "sync" => {
                let mut graph = a3s_jserial::read(&variant.java_raw(step_name, file)).unwrap();
                let root = graph.root_value().unwrap().as_ref().unwrap();
                let mut leaves = Vec::new();
                sync_leaves(&graph, root, "", &mut |rel, leaf, list| {
                    leaves.push((rel.to_owned(), leaf, list));
                });
                for (rel, leaf, list) in leaves {
                    if d14 && rel.ends_with(".ebo.zip") {
                        remove_list_element(&mut graph, list, leaf);
                    } else if let Some(size) = rust_zip_sizes.get(&rel)
                        && graph.field_bool(leaf, "compressed").unwrap()
                    {
                        set_field(&mut graph, leaf, "compressedSize", Value::Long(*size));
                    }
                }
                a3s_jserial::write(&graph).unwrap()
            }
            "serverinfo" => {
                let mut graph = a3s_jserial::read(&variant.java_raw(step_name, file)).unwrap();
                let root = graph.root_value().unwrap().as_ref().unwrap();
                let total = graph.field_i64(root, "totalFilesSize").unwrap();
                set_field(
                    &mut graph,
                    root,
                    "totalFilesSize",
                    Value::Long(total - java_generated + rust_generated),
                );
                if d14 {
                    let revision = graph.field_i32(root, "revision").unwrap();
                    let files = graph.field_i64(root, "numberOfFiles").unwrap();
                    set_field(&mut graph, root, "revision", Value::Int(revision - 1));
                    set_field(&mut graph, root, "numberOfFiles", Value::Long(files - 1));
                    if index == 1 {
                        set_field(
                            &mut graph,
                            root,
                            "repositoryContentUpdated",
                            Value::Boolean(false),
                        );
                    }
                }
                a3s_jserial::write(&graph).unwrap()
            }
            "changelogs" if d14 && index == 1 => variant.java_raw("1_initial", file),
            "changelogs" if d14 => {
                let plain = Variant::load(if variant.http() {
                    "http_plain"
                } else {
                    "ftp_plain"
                });
                plain.java_raw(step_name, file)
            }
            _ => variant.java_raw(step_name, file),
        };
        assert!(rust == expected, "{ctx}/{file}: stream differs from Java");
    }

    // .zsync files: the same set as Java (minus Java's .ebo.zip.zsync), byte-identical.
    let java_repo = variant.dir.join(step_name).join("repo");
    let java_files = files(&java_repo);
    let java_zsync: BTreeSet<&String> = java_files
        .keys()
        .filter(|r| r.ends_with(".zsync") && !(d14 && r.ends_with(".ebo.zip.zsync")))
        .collect();
    let rust_zsync: BTreeSet<&String> = after.keys().filter(|r| r.ends_with(".zsync")).collect();
    assert_eq!(rust_zsync, java_zsync, "{ctx}: .zsync files");
    let mtimes = source_mtimes(step);
    for rel in rust_zsync {
        let source = rel.strip_suffix(".zsync").unwrap();
        let java = fs::read(path_of_in(&java_repo, rel)).unwrap();
        let rust = fs::read(path_of_in(repo_dir, rel)).unwrap();
        assert!(
            rust == localize_zsync(&java, mtimes[source]),
            "{ctx}: {rel} differs"
        );
    }

    // .zip files: the same set as Java, each holding one DEFLATE entry equal to its source.
    let java_zips: BTreeSet<&String> = java_files.keys().filter(|r| r.ends_with(".zip")).collect();
    let rust_zips: BTreeSet<&String> = after.keys().filter(|r| r.ends_with(".zip")).collect();
    assert_eq!(rust_zips, java_zips, "{ctx}: .zip files");
    for rel in rust_zips {
        let source = fs::read(path_of_in(repo_dir, rel.strip_suffix(".zip").unwrap())).unwrap();
        let mut archive =
            zip::ZipArchive::new(fs::File::open(path_of_in(repo_dir, rel)).unwrap()).unwrap();
        assert_eq!(archive.len(), 1, "{ctx}: {rel}");
        let mut entry = archive.by_index(0).unwrap();
        let name = Path::new(rel)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            entry.name().unwrap(),
            name.strip_suffix(".zip").unwrap(),
            "{ctx}: {rel}"
        );
        assert_eq!(
            entry.compression(),
            zip::CompressionMethod::Deflated,
            "{ctx}: {rel}"
        );
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert!(content == source, "{ctx}: {rel} content");
    }
}

#[test]
fn golden_http_plain() {
    drop(replay(&Variant::load("http_plain"), |_, _, _| {}));
}

#[test]
fn golden_ftp_plain() {
    drop(replay(&Variant::load("ftp_plain"), |_, _, _| {}));
}

#[test]
fn golden_http_zip_with_d14() {
    drop(replay(&Variant::load("http_zip"), |index, _, repo| {
        // The generated `.ebo.zip` never becomes a leaf, so it never gets a `.zsync`.
        assert!(repo.join("@mod_b/Addons/mod_b.ebo.zip").is_file());
        assert!(
            !repo.join("@mod_b/Addons/mod_b.ebo.zip.zsync").exists(),
            "step {index}"
        );
    }));
}

#[test]
fn golden_ftp_zip_with_d14() {
    drop(replay(&Variant::load("ftp_zip"), |_, _, repo| {
        let sync: a3s_types::SyncTreeDirectory =
            a3s_repo_files::read_a3s(&repo.join(".a3s/sync")).unwrap();
        let mut ebo_zip = false;
        sync.walk(&mut |path, _| ebo_zip |= path.ends_with(".ebo.zip"));
        assert!(!ebo_zip);
    }));
}

/// Java's own readers accept every Rust-built `.a3s` file and zip (and `.zsync` when the
/// local time zone is UTC, as the harness pins it). Needs `source scripts/java_env.sh`.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn java_reads_rust_builds() {
    let harness = a3s_dev_tools::Harness::locate().unwrap();
    let utc = a3s_zsync::format_mtime(0).ends_with("+0000");
    for name in ["http_plain", "http_zip", "ftp_plain", "ftp_zip"] {
        let variant = Variant::load(name);
        let out = tempfile::tempdir().unwrap();
        drop(replay(&variant, |_, step, repo| {
            // Snapshot the step: the next step changes the tree.
            let snapshot = out.path().join(step);
            for (rel, _) in files(repo) {
                if !utc && rel.ends_with(".zsync") {
                    continue;
                }
                let target = path_of_in(&snapshot, &rel);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::copy(path_of_in(repo, &rel), &target).unwrap();
                let mtime = a3s_repo_files::hash::modified_millis(&path_of_in(repo, &rel)).unwrap();
                set_mtime(&target, mtime);
            }
            for file in A3S_FILES {
                let target = snapshot.join(".a3s").join(file);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::copy(repo.join(".a3s").join(file), target).unwrap();
            }
        }));
        let report = harness
            .verify(
                &[out.path().to_path_buf()],
                None,
                a3s_dev_tools::DumpMode::Identity,
            )
            .unwrap();
        assert!(report.is_success(), "{name}: {report:?}");
        assert!(report.ok.len() >= 15, "{name}: {report:?}");
    }
}

/// A flat sync tree of `n` empty-file leaves.
fn flat_sync(n: usize) -> Vec<u8> {
    let mut root = a3s_types::SyncTreeDirectory::new_root();
    let mut dir = a3s_types::SyncTreeDirectory::new("@m");
    dir.children = (0..n)
        .map(|i| {
            a3s_types::SyncTreeNode::Leaf(a3s_types::SyncTreeLeaf {
                name: format!("f{i}"),
                sha1: Some("0".to_owned()),
                ..Default::default()
            })
        })
        .collect();
    root.children = vec![a3s_types::SyncTreeNode::Directory(dir)];
    a3s_types::encode_stream(&root).unwrap()
}

fn java_references(n: usize) -> u64 {
    a3s_jserial::java_filter_stats(&flat_sync(n))
        .unwrap()
        .references
}

/// The Java filter emulation agrees with Java at the reference boundary: the largest
/// flat tree within 250 000 references is read, one more leaf is rejected.
#[test]
#[ignore = "needs JDK 25: source scripts/java_env.sh"]
fn java_filter_boundary_matches_java() {
    let limit = a3s_jserial::JavaFilterLimits::JAVA_A3S.max_references;
    // References grow by a constant per leaf.
    let per_leaf = java_references(2000) - java_references(1000);
    let base = java_references(1000) - 1000 * per_leaf;
    let mut n = usize::try_from((limit - base) / per_leaf).unwrap();
    while java_references(n + 1) <= limit {
        n += 1;
    }
    while java_references(n) > limit {
        n -= 1;
    }
    let harness = a3s_dev_tools::Harness::locate().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    for (leaves, accepted) in [(n, true), (n + 1, false)] {
        let raw = flat_sync(leaves);
        let accepted_by_rust = a3s_jserial::java_filter_stats(&raw)
            .unwrap()
            .check(a3s_jserial::JavaFilterLimits::JAVA_A3S)
            .is_ok();
        assert_eq!(accepted_by_rust, accepted, "{leaves} leaves");
        let dir = tmp.path().join(format!("n{leaves}/.a3s"));
        fs::create_dir_all(&dir).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &raw).unwrap();
        fs::write(dir.join("sync"), gz.finish().unwrap()).unwrap();
        let report = harness
            .verify(&[dir.join("sync")], None, a3s_dev_tools::DumpMode::Identity)
            .unwrap();
        assert_eq!(report.is_success(), accepted, "{leaves} leaves: {report:?}");
    }
}
