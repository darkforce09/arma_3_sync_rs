//! Repositories, stores and event capture for the end-to-end synchronization tests.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_sync::cleanup::RemovedFiles;
use a3s_sync::pipeline::{CheckRequest, ReconnectPolicy, SyncJob, UserconfigPolicy};
use a3s_sync::plan::NoAddons;
use a3s_sync::{MemoryStore, SyncEvent};
use a3s_types::local::Repository;
use a3s_types::{Protocol, ServerInfo, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

pub const REPOSITORY: &str = "test repository";

/// `tests/fixtures/java_built/<variant>/<step>/repo`.
pub fn fixture(variant: &str, step: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_built")
        .join(variant)
        .join(step)
        .join("repo")
}

/// Replaces the content of `root` by a copy of `from`.
pub fn serve(root: &Path, from: &Path) {
    if root.exists() {
        std::fs::remove_dir_all(root).expect("clear served root");
    }
    copy_tree(from, root);
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// The repository's `.a3s/sync`.
pub fn read_sync(repo: &Path) -> SyncTreeDirectory {
    a3s_repo_files::read_a3s(&repo.join(".a3s/sync")).expect("sync")
}

/// The repository's `.a3s/serverinfo`.
pub fn read_server_info(repo: &Path) -> ServerInfo {
    a3s_repo_files::read_a3s(&repo.join(".a3s/serverinfo")).expect("serverinfo")
}

/// Every leaf of a sync tree with its path.
pub fn leaves(sync: &SyncTreeDirectory) -> Vec<(String, SyncTreeLeaf)> {
    fn walk(dir: &SyncTreeDirectory, prefix: &str, out: &mut Vec<(String, SyncTreeLeaf)>) {
        for child in &dir.children {
            let path = if prefix.is_empty() {
                child.name().to_owned()
            } else {
                format!("{prefix}/{}", child.name())
            };
            match child {
                SyncTreeNode::Leaf(leaf) => out.push((path, leaf.clone())),
                SyncTreeNode::Directory(d) => walk(d, &path, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(sync, "", &mut out);
    out
}

/// Every file below `dir`, as `/`-joined relative paths, sorted (links are not followed).
pub fn files_below(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&entry.path(), &path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

/// A stored repository synchronized into `destination`.
pub fn repository(protocol: Protocol, destination: &Path, exact_match: bool) -> Repository {
    let mut repository = Repository::new(REPOSITORY, protocol);
    repository.default_download_location = Some(destination.to_string_lossy().into_owned());
    repository.exact_match = exact_match;
    repository
}

/// A store holding `repository`.
pub fn store(repository: Repository) -> Arc<MemoryStore> {
    Arc::new(MemoryStore::new([repository]))
}

/// The check of the test repository.
pub fn check_request(protocol: Protocol) -> CheckRequest {
    CheckRequest {
        repository_name: REPOSITORY.to_owned(),
        event_name: None,
        protocol,
        addons: Arc::new(NoAddons),
        removed_files: RemovedFiles::Delete,
    }
}

/// A headless job without userconfig copy or reconnection.
pub fn job(protocol: Protocol) -> SyncJob {
    SyncJob {
        check: check_request(protocol),
        arma3_exe_path: None,
        userconfig: UserconfigPolicy::Silent,
        reconnect: ReconnectPolicy::NONE,
    }
}

pub fn finished(events: &[SyncEvent]) -> Vec<&a3s_sync::Outcome> {
    events
        .iter()
        .filter_map(|e| match e {
            SyncEvent::Finished(outcome) => Some(outcome),
            _ => None,
        })
        .collect()
}

pub fn count(events: &[SyncEvent], f: impl Fn(&SyncEvent) -> bool) -> usize {
    events.iter().filter(|e| f(e)).count()
}

/// One file of a hand-built repository.
pub struct RepoFile<'a> {
    pub path: &'a str,
    pub content: &'a [u8],
    pub compressed: bool,
}

/// Writes a repository into `root`: the files (and a single-entry `.zip` next to compressed
/// ones), `.a3s/sync`, `.a3s/serverinfo` with `revision`, and the other `.a3s` files of a
/// Java-built fixture.
pub fn write_repository(root: &Path, files: &[RepoFile<'_>], revision: i32) {
    let mut sync = SyncTreeDirectory::new_root();
    for file in files {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, file.content).expect("write");
        let mut compressed_size = 0;
        if file.compressed {
            let zip = zip_of(file.content);
            compressed_size = zip.len() as i64;
            std::fs::write(format!("{}.zip", path.display()), zip).expect("zip");
        }
        let sha1 = a3s_repo_files::hash::sha1_file(&path).expect("sha1");
        insert_leaf(
            &mut sync,
            file.path,
            SyncTreeLeaf {
                name: file.path.rsplit('/').next().expect("name").to_owned(),
                sha1: Some(sha1),
                size: file.content.len() as i64,
                compressed_size,
                compressed: file.compressed,
                ..Default::default()
            },
        );
    }
    let a3s = root.join(".a3s");
    std::fs::create_dir_all(&a3s).expect("mkdir");
    a3s_repo_files::write_a3s(&a3s.join("sync"), &sync).expect("sync");
    let server_info = ServerInfo {
        revision,
        number_of_files: files.len() as i64,
        number_of_connections: 2,
        compressed_pbo_files_only: files.iter().any(|f| f.compressed),
        ..Default::default()
    };
    a3s_repo_files::write_a3s(&a3s.join("serverinfo"), &server_info).expect("serverinfo");
    let other = fixture("http_plain", "1_initial").join(".a3s");
    for name in ["changelogs", "events", "autoconfig"] {
        std::fs::copy(other.join(name), a3s.join(name)).expect("copy .a3s file");
    }
}

fn insert_leaf(root: &mut SyncTreeDirectory, path: &str, leaf: SyncTreeLeaf) {
    let mut dir = root;
    let segments: Vec<&str> = path.split('/').collect();
    for segment in &segments[..segments.len() - 1] {
        let existing = dir.children.iter().position(|c| c.name() == *segment);
        let index = existing.unwrap_or_else(|| {
            dir.children
                .push(SyncTreeNode::Directory(SyncTreeDirectory {
                    name: (*segment).to_owned(),
                    mark_as_addon: segment.starts_with('@'),
                    ..Default::default()
                }));
            dir.children.len() - 1
        });
        let SyncTreeNode::Directory(next) = &mut dir.children[index] else {
            panic!("{segment} is a file");
        };
        dir = next;
    }
    dir.children.push(SyncTreeNode::Leaf(leaf));
}

/// A single-entry DEFLATE archive of `content`.
pub fn zip_of(content: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("entry", zip::write::SimpleFileOptions::default())
        .expect("start");
    writer.write_all(content).expect("write");
    writer.finish().expect("finish").into_inner()
}
