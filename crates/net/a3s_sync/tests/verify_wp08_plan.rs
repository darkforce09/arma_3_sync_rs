//! WP08 verification: adversarial planning tests (WOG fixture, partial local trees, hostile
//! sync trees, extra-file deletion, hidden paths, SHA-1 cache, cancellation, huge sizes).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_repo_files::read_a3s;
use a3s_sync::plan::{
    NoAddons, add_extra_local_files, assign_destinations, compute_local_sha1, is_hidden_path,
    mark_hidden, mark_updated, plan_repository, select_all_except_root,
};
use a3s_sync::{
    EventSender, Phase, PlanError, PlanNode, PlanSummary, PlanTree, RemoteMetadata, ReportTotals,
    SyncEvent, SyncOptions,
};
use a3s_transport::CancellationToken;
use a3s_types::{Event, Events, ServerInfo, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

// ---------------------------------------------------------------------------------------------
// Helpers

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/java_repos/wog_ftp")
}

fn wog_metadata() -> RemoteMetadata {
    let sync: SyncTreeDirectory = read_a3s(&fixture_dir().join("sync")).expect("sync");
    let server_info: ServerInfo = read_a3s(&fixture_dir().join("serverinfo")).expect("info");
    RemoteMetadata {
        sync: Some(Arc::new(sync)),
        server_info: Some(Arc::new(server_info)),
        ..Default::default()
    }
}

fn metadata_of(root: SyncTreeDirectory) -> RemoteMetadata {
    RemoteMetadata {
        sync: Some(Arc::new(root)),
        ..Default::default()
    }
}

fn options(destination: &Path, exact_match: bool) -> SyncOptions {
    SyncOptions {
        repository_name: "R".to_owned(),
        event_name: None,
        destination: destination.to_path_buf(),
        auto_discover: false,
        exact_match,
        hidden_folder_paths: BTreeSet::new(),
        client_connections: 1,
        max_download_speed: 0,
        max_errors: a3s_sync::MAX_DOWNLOAD_ERRORS,
    }
}

fn plan_with(
    metadata: &RemoteMetadata,
    options: &SyncOptions,
    cache: &mut a3s_hash::Sha1Cache,
) -> Result<PlanTree, PlanError> {
    plan_repository(
        metadata,
        options,
        &NoAddons,
        cache,
        &CancellationToken::new(),
        &EventSender::noop(),
    )
}

fn plan(metadata: &RemoteMetadata, options: &SyncOptions) -> Result<PlanTree, PlanError> {
    plan_with(metadata, options, &mut a3s_hash::Sha1Cache::default())
}

fn leaf(name: &str, sha1: Option<&str>, size: i64) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: sha1.map(str::to_owned),
        size,
        ..Default::default()
    })
}

fn dir(name: &str, mark_as_addon: bool, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    SyncTreeNode::Directory(SyncTreeDirectory {
        name: name.to_owned(),
        mark_as_addon,
        children,
        ..Default::default()
    })
}

fn root(children: Vec<SyncTreeNode>) -> SyncTreeDirectory {
    let mut r = SyncTreeDirectory::new_root();
    r.children = children;
    r.sort_children();
    r
}

fn touch(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, content).expect("write");
}

/// Lexical normalization (no symlink resolution): folds `.` and `..`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Every node's local path (and every task path) must be strictly below `base`.
fn escapes(tree: &PlanTree, base: &Path) -> Vec<(String, PathBuf)> {
    let base = normalize(&std::path::absolute(base).expect("absolute"));
    let mut out = Vec::new();
    tree.walk(&mut |path, node| {
        if let Some(local) = node.local_path() {
            let abs = normalize(&std::path::absolute(&local).expect("absolute"));
            if !abs.starts_with(&base) || abs == base {
                out.push((path.to_owned(), local));
            }
        }
    });
    out
}

fn deleted_paths(tree: &PlanTree) -> Vec<String> {
    let mut out = Vec::new();
    tree.walk(&mut |path, node| {
        if node.deleted() {
            out.push(path.to_owned());
        }
    });
    out
}

fn non_root_count(tree: &PlanTree) -> usize {
    let mut n = 0;
    tree.walk(&mut |_, _| n += 1);
    n
}

// ---------------------------------------------------------------------------------------------
// WOG fixture

#[test]
fn verify_wog_counts_match_serverinfo() {
    let metadata = wog_metadata();
    let info = metadata.server_info.as_deref().expect("info");
    let base = tempfile::tempdir().expect("tempdir");
    let tree = plan(&metadata, &options(base.path(), false)).expect("plan");
    assert_eq!(
        tree.leaf_count() as i64,
        info.number_of_files,
        "leaf count vs serverinfo.number_of_files"
    );
    // total_files_size also counts `.zsync`/`.zip` files: the uncompressed sum is a lower bound.
    let mut size_sum: u64 = 0;
    tree.walk(&mut |_, node| {
        if let PlanNode::Leaf(l) = node {
            size_sum += l.size;
        }
    });
    assert!(
        size_sum <= info.total_files_size as u64,
        "{size_sum} > {}",
        info.total_files_size
    );
    assert!(escapes(&tree, base.path()).is_empty());
}

/// A partially present local copy of WOG: one file present with a cached matching SHA-1,
/// extra files around it, a `.part` and a `.pbo.zip`.
#[test]
fn verify_wog_partial_local_tree() {
    let metadata = wog_metadata();
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    let empty = plan(&metadata, &options(b, false)).expect("plan");
    let full = {
        let mut t = empty.clone();
        select_all_except_root(&mut t);
        PlanSummary::compute(&t)
    };

    // Pick the first leaf at depth >= 2 with a SHA-1.
    let mut pick = None;
    empty.walk(&mut |path, node| {
        if pick.is_none()
            && path.matches('/').count() >= 1
            && let PlanNode::Leaf(l) = node
            && l.sha1.is_some()
        {
            pick = Some((path.to_owned(), l.clone()));
        }
    });
    let (path, remote_leaf) = pick.expect("a nested leaf");
    let local = b.join(&path);
    touch(&local, b"stand-in content");
    let parent = local.parent().expect("parent").to_path_buf();
    touch(&parent.join("zz_extra_junk.bin"), b"x");
    touch(&parent.join("zz_partial.pbo.part"), b"x");
    touch(&parent.join("zz_other.pbo.zip"), b"x");

    // Cache says the stand-in has the remote SHA-1 (same mtime): the file is up to date.
    let abs = std::path::absolute(&local).expect("absolute");
    let mtime = a3s_hash::modified_millis(&abs).expect("mtime");
    let mut cache = a3s_hash::Sha1Cache::default();
    cache.entries.insert(
        abs.clone(),
        a3s_hash::FileAttributes {
            sha1: remote_leaf.sha1.clone().expect("sha1"),
            last_modified: mtime,
        },
    );

    let mut tree = plan_with(&metadata, &options(b, false), &mut cache).expect("plan");
    assert!(
        !tree.get(&path).expect("leaf").updated(),
        "cached matching file must not be updated"
    );
    // Every ancestor exists locally now.
    let segments: Vec<&str> = path.split('/').collect();
    let mut ancestors = Vec::new();
    for i in 1..segments.len() {
        let p = segments[..i].join("/");
        assert!(!tree.get(&p).expect("dir").updated(), "{p}");
        ancestors.push(p);
    }
    let parent_rel = segments[..segments.len() - 1].join("/");
    assert_eq!(
        deleted_paths(&tree),
        [format!("{parent_rel}/zz_extra_junk.bin")],
        "only the junk file is extra (no .part/.pbo.zip)"
    );
    assert!(escapes(&tree, b).is_empty());

    select_all_except_root(&mut tree);
    let s = PlanSummary::compute(&tree);
    let nodes = non_root_count(&tree);
    // nodes includes the added deleted leaf.
    assert_eq!(s.files_updated, nodes - 1 - 1 - ancestors.len());
    assert_eq!(s.files_deleted, 1);
    assert_eq!(s.delete_tasks.len(), 1);
    assert_eq!(
        s.delete_tasks[0].local_path,
        parent.join("zz_extra_junk.bin")
    );
    let removed = if remote_leaf.compressed {
        remote_leaf.compressed_size
    } else {
        remote_leaf.size
    };
    assert_eq!(s.total_download_size, full.total_download_size - removed);
    assert_eq!(
        s.download_tasks.len(),
        full.download_tasks.len() - 1 - ancestors.len()
    );
}

/// Planning twice reuses the cache (no Progress the 2nd time); touching a file re-hashes it.
#[test]
fn verify_sha1_cache_reuse_by_mtime_through_plan() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    touch(&b.join("@a/a.pbo"), b"abc");
    touch(&b.join("@a/b.pbo"), b"");
    let metadata = metadata_of(root(vec![dir(
        "@a",
        true,
        vec![
            leaf("a.pbo", Some("a9993e364706816aba3e25717850c26c9cd0d89d"), 3),
            leaf("b.pbo", Some("0"), 0),
        ],
    )]));
    let mut cache = a3s_hash::Sha1Cache::default();
    let run = |cache: &mut a3s_hash::Sha1Cache| {
        let (sender, rx) = EventSender::channel();
        let tree = plan_repository(
            &metadata,
            &options(b, false),
            &NoAddons,
            cache,
            &CancellationToken::new(),
            &sender,
        )
        .expect("plan");
        drop(sender);
        (tree, rx.try_iter().collect::<Vec<_>>())
    };
    let (tree, events) = run(&mut cache);
    assert!(!tree.get("@a/a.pbo").expect("a").updated());
    assert!(
        !tree.get("@a/b.pbo").expect("b").updated(),
        "empty file vs \"0\""
    );
    assert_eq!(events[0], SyncEvent::Phase(Phase::CheckingFiles));
    assert!(events.len() > 1, "hashing emits progress");
    assert_eq!(cache.entries.len(), 2);

    let (_, events) = run(&mut cache);
    assert_eq!(events, [SyncEvent::Phase(Phase::CheckingFiles)], "all hits");

    // New content and a different mtime: re-hashed, now outdated.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(b.join("@a/a.pbo"), b"xyz").expect("write");
    let f = std::fs::File::options()
        .write(true)
        .open(b.join("@a/a.pbo"))
        .expect("open");
    f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .expect("set mtime");
    drop(f);
    let (tree, events) = run(&mut cache);
    assert!(tree.get("@a/a.pbo").expect("a").updated());
    assert_eq!(events.len(), 2, "{events:?}");
}

/// Cancellation while hashing many files → Cancelled, and not every file was hashed.
#[test]
fn verify_cancel_during_hashing() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    let n = 600;
    let content = vec![7u8; 256 * 1024];
    let mut leaves = Vec::new();
    for i in 0..n {
        let name = format!("f{i:04}.pbo");
        touch(&b.join("@a").join(&name), &content);
        leaves.push(leaf(&name, Some("x"), content.len() as i64));
    }
    let metadata = metadata_of(root(vec![dir("@a", true, leaves)]));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let events = EventSender::from_fn(move |e| {
        if matches!(e, SyncEvent::Progress { .. }) {
            trigger.cancel();
        }
    });
    let mut cache = a3s_hash::Sha1Cache::default();
    let result = plan_repository(
        &metadata,
        &options(b, false),
        &NoAddons,
        &mut cache,
        &cancel,
        &events,
    );
    assert!(matches!(result, Err(PlanError::Cancelled)), "{result:?}");
    assert!(
        cache.entries.len() < n,
        "hashing stopped early ({} of {n})",
        cache.entries.len()
    );
}

// ---------------------------------------------------------------------------------------------
// Hostile sync trees: no destination outside the download root

#[test]
fn verify_hostile_absolute_names_stay_inside() {
    let base = tempfile::tempdir().expect("tempdir");
    let metadata = metadata_of(root(vec![
        leaf("/tmp/a3s_verify_evil_leaf", Some("x"), 1),
        dir(
            "/tmp/a3s_verify_evil_dir",
            false,
            vec![leaf("f", Some("x"), 1)],
        ),
        dir("ok", false, vec![leaf("/etc/passwd", Some("x"), 1)]),
    ]));
    let tree = plan(&metadata, &options(base.path(), false)).expect("plan");
    let escaped = escapes(&tree, base.path());
    assert!(
        escaped.is_empty(),
        "absolute names escape the root (Java `dest + \"/\" + name` keeps them inside): {escaped:?}"
    );
}

#[test]
fn verify_hostile_dotdot_names_stay_inside() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    std::fs::create_dir(&dest).expect("mkdir");
    let metadata = metadata_of(root(vec![
        dir("..", false, vec![leaf("evil.txt", Some("x"), 1)]),
        dir("ok", false, vec![leaf("..", Some("x"), 1)]),
    ]));
    let tree = plan(&metadata, &options(&dest, false)).expect("plan");
    let escaped = escapes(&tree, &dest);
    assert!(
        escaped.is_empty(),
        "`..` names escape the root: {escaped:?}"
    );
}

/// A remote folder named `..` makes the extra-file scan list the parent of the destination:
/// siblings of the download folder become delete tasks, even without exact match.
#[test]
fn verify_hostile_dotdot_dir_never_deletes_outside() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    std::fs::create_dir(&dest).expect("mkdir");
    touch(&base.path().join("precious/data.bin"), b"keep me");
    let metadata = metadata_of(root(vec![dir("..", false, vec![])]));
    let mut tree = plan(&metadata, &options(&dest, false)).expect("plan");
    select_all_except_root(&mut tree);
    let s = PlanSummary::compute(&tree);
    let dest_norm = normalize(&dest);
    let outside: Vec<_> = s
        .delete_tasks
        .iter()
        .filter(|t| !normalize(&t.local_path).starts_with(&dest_norm))
        .map(|t| t.local_path.clone())
        .collect();
    assert!(
        outside.is_empty(),
        "delete tasks outside the root: {outside:?}"
    );
}

/// A remote folder named "" lists the destination itself: local root content is offered for
/// deletion although exact match is off.
#[test]
fn verify_hostile_empty_dir_name_without_exact_match() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    touch(&b.join("@mine/keep.pbo"), b"x");
    let metadata = metadata_of(root(vec![dir("", false, vec![]), dir("@a", true, vec![])]));
    let tree = plan(&metadata, &options(b, false)).expect("plan");
    let deleted = deleted_paths(&tree);
    assert!(
        !deleted.iter().any(|p| p.contains("@mine")),
        "root content flagged without exact match: {deleted:?}"
    );
}

/// Windows separators in names are plain characters on Unix: no traversal.
#[cfg(unix)]
#[test]
fn verify_backslash_names_are_literal_on_unix() {
    let base = tempfile::tempdir().expect("tempdir");
    let metadata = metadata_of(root(vec![
        leaf("..\\..\\evil.txt", Some("x"), 1),
        dir("a\\b", false, vec![leaf("c", Some("x"), 1)]),
    ]));
    let tree = plan(&metadata, &options(base.path(), false)).expect("plan");
    assert!(escapes(&tree, base.path()).is_empty());
    assert_eq!(
        tree.get("..\\..\\evil.txt")
            .and_then(PlanNode::local_path)
            .expect("local"),
        base.path().join("..\\..\\evil.txt")
    );
}

/// A symlink inside a remote folder pointing outside the destination: its content is offered
/// for deletion (Java follows it too), so a delete task resolves outside the root.
#[cfg(unix)]
#[test]
fn verify_symlinked_extra_dir_never_deletes_outside() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
    touch(&base.path().join("outside/data.bin"), b"keep me");
    std::os::unix::fs::symlink(base.path().join("outside"), dest.join("@a/link")).expect("ln");
    let metadata = metadata_of(root(vec![dir("@a", true, vec![])]));
    let mut tree = plan(&metadata, &options(&dest, false)).expect("plan");
    select_all_except_root(&mut tree);
    let s = PlanSummary::compute(&tree);
    let dest_canon = dest.canonicalize().expect("canon");
    let outside: Vec<_> = s
        .delete_tasks
        .iter()
        .filter(|t| {
            t.local_path
                .canonicalize()
                .is_ok_and(|c| !c.starts_with(&dest_canon))
        })
        .map(|t| t.local_path.clone())
        .collect();
    assert!(
        outside.is_empty(),
        "delete tasks resolving outside the root through a symlink: {outside:?}"
    );
}

/// Two symlinks to their own folder: the scan must terminate in reasonable time.
#[cfg(unix)]
#[test]
fn verify_symlink_loop_terminates() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path().to_path_buf();
    std::fs::create_dir_all(b.join("@a")).expect("mkdir");
    std::os::unix::fs::symlink(".", b.join("@a/l1")).expect("ln");
    std::os::unix::fs::symlink(".", b.join("@a/l2")).expect("ln");
    let (tx, rx) = std::sync::mpsc::channel();
    let bb = b.clone();
    std::thread::Builder::new()
        .name("verify-symlink-loop".to_owned())
        .spawn(move || {
            let metadata = metadata_of(root(vec![dir("@a", true, vec![])]));
            let r = plan(&metadata, &options(&bb, false)).map(|t| non_root_count(&t));
            tx.send(r.is_ok()).ok();
        })
        .expect("spawn");
    let finished = rx.recv_timeout(Duration::from_secs(5));
    assert!(
        finished.is_ok(),
        "planning did not finish within 5 s with a symlink loop"
    );
}

// ---------------------------------------------------------------------------------------------
// Extra files, exact match, case

#[test]
fn verify_exact_match_root_scan_and_exclusions() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    touch(&b.join("root.pbo.part"), b"x");
    touch(&b.join("root.pbo.zip"), b"x");
    touch(&b.join("Stale.txt"), b"x");
    touch(&b.join("@A/x.pbo"), b"x"); // case variant of remote `@a`
    let metadata = metadata_of(root(vec![dir("@a", true, vec![leaf("x.pbo", None, 1)])]));
    let tree = plan(&metadata, &options(b, true)).expect("plan");
    // Remote `@a` vs local `@A`: equal ignoring case, so not extra (Java parity), while `@a`
    // does not exist on a case-sensitive file system and is updated.
    assert_eq!(deleted_paths(&tree), ["Stale.txt"]);
    assert!(tree.get("@a").expect("@a").updated());
    let PlanNode::Leaf(l) = tree.get("Stale.txt").expect("stale") else {
        panic!("leaf")
    };
    assert_eq!(l.destination_path.as_deref(), Some(b));
    assert!(l.sha1.is_none() && l.size == 0);
}

#[test]
fn verify_extra_files_in_hidden_subfolder_not_scanned() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    touch(&b.join("@a/keep/extra.txt"), b"x");
    touch(&b.join("@a/extra2.txt"), b"x");
    let metadata = metadata_of(root(vec![dir(
        "@a",
        true,
        vec![dir("keep", false, vec![])],
    )]));
    let mut opts = options(b, false);
    // Windows separator in the hidden path.
    opts.hidden_folder_paths = ["@a\\keep".to_owned()].into();
    let tree = plan(&metadata, &opts).expect("plan");
    assert_eq!(deleted_paths(&tree), ["@a/extra2.txt"]);
}

#[test]
fn verify_hidden_path_edge_cases() {
    let set =
        |items: &[&str]| -> BTreeSet<String> { items.iter().map(|s| (*s).to_owned()).collect() };
    // Java builds paths without a leading slash: "/@a" never matches.
    assert!(!is_hidden_path("@a", &set(&["/@a"])));
    // Trailing separator: hides the subfolders only.
    assert!(!is_hidden_path("@a", &set(&["@a\\"])));
    assert!(is_hidden_path("@a/addons", &set(&["@a\\"])));
    // Longer than the path: no match.
    assert!(!is_hidden_path("@a", &set(&["@a/addons"])));
    // Unicode, exact prefix.
    assert!(is_hidden_path("@äöü/x", &set(&["@äö"])));
    assert!(!is_hidden_path("@ÄÖÜ/x", &set(&["@äö"])));

    // mark_hidden on leaves: untouched.
    let mut tree = PlanTree::from_sync_tree(&root(vec![
        leaf("@a", Some("x"), 1),
        dir("@ab", false, vec![]),
    ]));
    assign_destinations(&mut tree, Path::new("/d"), false, &NoAddons);
    mark_hidden(&mut tree, &set(&["@a"]));
    assert!(matches!(tree.get("@ab"), Some(PlanNode::Directory(d)) if d.hidden));
}

/// Steps in isolation, including the root of a tree with a deleted remote flag.
#[test]
fn verify_steps_never_touch_remote_deleted_flag() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    let mut r = root(vec![dir("@a", true, vec![leaf("x", Some("0"), 0)])]);
    if let SyncTreeNode::Directory(d) = &mut r.children[0] {
        d.deleted = true;
    }
    let mut tree = PlanTree::from_sync_tree(&r);
    assign_destinations(&mut tree, b, false, &NoAddons);
    compute_local_sha1(
        &mut tree,
        &mut a3s_hash::Sha1Cache::default(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .expect("hash");
    mark_updated(&mut tree);
    add_extra_local_files(&mut tree, b, true);
    assert!(tree.get("@a").expect("@a").deleted(), "decoded flag kept");
}

// ---------------------------------------------------------------------------------------------
// Events

#[test]
fn verify_event_with_userconfig_through_plan() {
    let base = tempfile::tempdir().expect("tempdir");
    let b = base.path();
    let mut metadata = metadata_of(root(vec![
        dir(
            "userconfig",
            false,
            vec![dir("ace", false, vec![leaf("ace.hpp", Some("x"), 1)])],
        ),
        dir("@a", true, vec![leaf("a.pbo", Some("x"), 1)]),
        dir("@b", true, vec![]),
    ]));
    metadata.events = Some(Arc::new(Events {
        list: vec![Event {
            name: "op".to_owned(),
            addon_names: [("@a".to_owned(), true)].into(),
            userconfig_folder_names: [("ace".to_owned(), false)].into(),
            ..Default::default()
        }],
    }));
    let mut opts = options(b, true);
    opts.event_name = Some("op".to_owned());
    touch(&b.join("@b/old.pbo"), b"x");
    let tree = plan(&metadata, &opts).expect("plan");
    let names: Vec<_> = tree.root.children.iter().map(PlanNode::name).collect();
    // With exact match the local `@b` (not in the event) is offered for deletion.
    assert_eq!(names, ["@a", "@b", "userconfig"]);
    assert!(tree.get("@b").expect("@b").deleted());
    assert_eq!(
        tree.get("userconfig/ace/ace.hpp")
            .and_then(PlanNode::local_path)
            .expect("local"),
        std::path::absolute(b.join("userconfig/ace/ace.hpp")).expect("abs")
    );
    let mut t = tree.clone();
    select_all_except_root(&mut t);
    assert_eq!(
        PlanSummary::compute(&t).userconfig_dir.as_deref(),
        Some("userconfig")
    );

    // Unknown event (DV-P9) and events missing.
    opts.event_name = Some("OP".to_owned());
    assert!(matches!(
        plan(&metadata, &opts),
        Err(PlanError::EventNotFound { name }) if name == "OP"
    ));
    metadata.events = None;
    assert!(matches!(
        plan(&metadata, &opts),
        Err(PlanError::EventsMissing { .. })
    ));
}

// ---------------------------------------------------------------------------------------------
// Auto-discover

#[test]
fn verify_auto_discover_destinations_and_hashing() {
    let base = tempfile::tempdir().expect("tempdir");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let dest = base.path();
    touch(&elsewhere.path().join("@ACE/addons/ace.pbo"), b"abc");
    let metadata = metadata_of(root(vec![dir(
        "mods",
        false,
        vec![dir(
            "@ACE",
            true,
            vec![dir(
                "addons",
                false,
                vec![leaf(
                    "ace.pbo",
                    Some("a9993e364706816aba3e25717850c26c9cd0d89d"),
                    3,
                )],
            )],
        )],
    )]));
    let mut opts = options(dest, true);
    opts.auto_discover = true;
    let found = elsewhere.path().to_path_buf();
    let locator = move |name: &str| (name.to_lowercase() == "@ace").then(|| found.clone());
    let tree = plan_repository(
        &metadata,
        &opts,
        &locator,
        &mut a3s_hash::Sha1Cache::default(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .expect("plan");
    assert_eq!(
        tree.get("mods/@ACE").and_then(PlanNode::destination_path),
        Some(elsewhere.path())
    );
    assert!(!tree.get("mods/@ACE").expect("ace").updated());
    assert!(!tree.get("mods/@ACE/addons/ace.pbo").expect("pbo").updated());
    // `mods` itself does not exist locally.
    assert!(tree.get("mods").expect("mods").updated());
}

// ---------------------------------------------------------------------------------------------
// Sizes beyond 2 GiB and i64

#[test]
fn verify_huge_sizes_do_not_overflow() {
    let five_gib: i64 = 5 * 1024 * 1024 * 1024;
    let mut tree = PlanTree::from_sync_tree(&root(vec![dir(
        "@a",
        true,
        vec![
            leaf("big1", Some("x"), five_gib),
            leaf("max1", Some("x"), i64::MAX),
            leaf("max2", Some("x"), i64::MAX),
            leaf("max3", Some("x"), i64::MAX),
        ],
    )]));
    assign_destinations(&mut tree, Path::new("/d"), false, &NoAddons);
    mark_updated(&mut tree);
    select_all_except_root(&mut tree);
    let r = std::panic::catch_unwind(|| PlanSummary::compute(&tree));
    assert!(
        r.is_ok(),
        "PlanSummary::compute panics on huge (hostile) sizes"
    );
    let s = r.expect("summary");
    let r2 = std::panic::catch_unwind(|| ReportTotals::compute(&s.download_tasks));
    assert!(r2.is_ok(), "ReportTotals::compute panics on huge sizes");
}

#[test]
fn verify_five_gib_download_size() {
    let five_gib: i64 = 5 * 1024 * 1024 * 1024;
    let mut tree = PlanTree::from_sync_tree(&root(vec![leaf("big", Some("x"), five_gib)]));
    assign_destinations(&mut tree, Path::new("/d"), false, &NoAddons);
    mark_updated(&mut tree);
    select_all_except_root(&mut tree);
    assert_eq!(
        PlanSummary::compute(&tree).total_download_size,
        five_gib as u64
    );
    if let Some(PlanNode::Leaf(l)) = tree.get_mut("big") {
        l.complete = 37.5;
    }
    let s = PlanSummary::compute(&tree);
    assert_eq!(
        s.total_download_size,
        (five_gib as f64 * 62.5 / 100.0) as u64
    );
}
