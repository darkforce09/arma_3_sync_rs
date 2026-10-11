//! Deleting extra local files never follows links and never leaves the destination; copying
//! folders.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::Arc;

use a3s_sync::cleanup::{DeleteRoots, copy_directory, delete_extra_files, protected_paths};
use a3s_sync::plan::{plan_repository, select_all_except_root};
use a3s_sync::{
    DeleteTask, DownloadStatus, DownloadTask, DownloadTaskKind, EventSender, Phase, PlanSummary,
    RemoteMetadata, SyncEvent, SyncOptions,
};
use a3s_transport::CancellationToken;
use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, b"x").expect("write");
}

fn task(dest: &Path, relative: &str, is_directory: bool) -> DeleteTask {
    DeleteTask {
        relative_path: relative.to_owned(),
        local_path: dest.join(relative),
        is_directory,
    }
}

fn delete(tasks: &[DeleteTask], dest: &Path) -> a3s_sync::cleanup::CleanupReport {
    delete_extra_files(
        tasks,
        &DeleteRoots::from_plan(&a3s_sync::PlanTree::new(), dest),
        &HashSet::new(),
        &BTreeSet::new(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
}

#[test]
fn deletes_files_and_folders() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path();
    touch(&dest.join("extra.txt"));
    touch(&dest.join("@old/addons/a.pbo"));
    touch(&dest.join("@old/addons/sub/b.pbo"));
    let tasks = [
        task(dest, "extra.txt", false),
        // A folder before its content: the content is then already gone.
        task(dest, "@old", true),
        task(dest, "@old/addons", true),
        task(dest, "@old/addons/a.pbo", false),
        task(dest, "missing.txt", false),
    ];
    let (events, rx) = EventSender::channel();
    let report = delete_extra_files(
        &tasks,
        &DeleteRoots::from_plan(&a3s_sync::PlanTree::new(), dest),
        &HashSet::new(),
        &BTreeSet::new(),
        &CancellationToken::new(),
        &events,
    );
    assert_eq!(report.deleted, 2);
    assert_eq!(report.skipped, 3);
    assert!(report.refused.is_empty() && report.failed.is_empty());
    assert!(std::fs::read_dir(dest).expect("dest").next().is_none());
    assert_eq!(
        rx.try_iter().collect::<Vec<_>>(),
        [SyncEvent::Phase(Phase::DeletingExtraFiles)]
    );

    // Nothing to do: no phase.
    let (events, rx) = EventSender::channel();
    delete_extra_files(
        &[],
        &DeleteRoots::default(),
        &HashSet::new(),
        &BTreeSet::new(),
        &CancellationToken::new(),
        &events,
    );
    assert!(rx.try_iter().next().is_none());
}

#[cfg(unix)]
#[test]
fn links_are_removed_never_followed() {
    use std::os::unix::fs::symlink;

    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    let outside = base.path().join("outside");
    touch(&outside.join("keep.txt"));
    std::fs::create_dir_all(&dest).expect("mkdir");

    // A link to an outside folder, listed as a file and as a folder.
    for is_directory in [false, true] {
        symlink(&outside, dest.join("@link")).expect("symlink");
        let report = delete(&[task(&dest, "@link", is_directory)], &dest);
        assert_eq!(report.deleted, 1);
        assert!(std::fs::symlink_metadata(dest.join("@link")).is_err());
        assert!(outside.join("keep.txt").exists());
    }

    // A link inside a real folder that is deleted.
    touch(&dest.join("@real/f.txt"));
    symlink(&outside, dest.join("@real/inner")).expect("symlink");
    let report = delete(&[task(&dest, "@real", true)], &dest);
    assert_eq!(report.deleted, 1);
    assert!(!dest.join("@real").exists());
    assert!(outside.join("keep.txt").exists());

    // A dangling link.
    symlink(base.path().join("nowhere"), dest.join("dangling")).expect("symlink");
    let report = delete(&[task(&dest, "dangling", false)], &dest);
    assert_eq!(report.deleted, 1);
    assert!(std::fs::symlink_metadata(dest.join("dangling")).is_err());
}

#[cfg(windows)]
#[test]
fn windows_directory_links_are_removed_never_followed() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    let outside = base.path().join("outside");
    touch(&outside.join("keep.txt"));
    std::fs::create_dir_all(&dest).expect("mkdir");
    if std::os::windows::fs::symlink_dir(&outside, dest.join("@link")).is_ok() {
        let report = delete(&[task(&dest, "@link", false)], &dest);
        assert_eq!(report.deleted, 1, "{:?}", report.failed);
        assert!(std::fs::symlink_metadata(dest.join("@link")).is_err());
        assert!(outside.join("keep.txt").exists());
    }
    let junction = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(dest.join("@junction"))
        .arg(&outside)
        .status();
    if junction.is_ok_and(|s| s.success()) {
        let report = delete(&[task(&dest, "@junction", true)], &dest);
        assert_eq!(report.deleted, 1, "{:?}", report.failed);
        assert!(std::fs::symlink_metadata(dest.join("@junction")).is_err());
        assert!(outside.join("keep.txt").exists());
    }
}

#[test]
fn unsafe_tasks_are_refused() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    touch(&base.path().join("precious.txt"));
    touch(&dest.join("docs/hidden/secret.txt"));
    touch(&dest.join("@a/addons/x.ebo.zip"));
    let tasks = [
        DeleteTask {
            relative_path: "precious.txt".to_owned(),
            local_path: base.path().join("precious.txt"),
            is_directory: false,
        },
        DeleteTask {
            relative_path: "x".to_owned(),
            local_path: dest.join("../precious.txt"),
            is_directory: false,
        },
        DeleteTask {
            relative_path: "dest".to_owned(),
            local_path: dest.clone(),
            is_directory: true,
        },
        DeleteTask {
            relative_path: "../precious.txt".to_owned(),
            local_path: dest.join("precious.txt"),
            is_directory: false,
        },
        task(&dest, "docs/hidden/secret.txt", false),
    ];
    let hidden: BTreeSet<String> = ["docs/hidden".to_owned()].into();
    let report = delete_extra_files(
        &tasks,
        &DeleteRoots::from_plan(&a3s_sync::PlanTree::new(), &dest),
        &HashSet::new(),
        &hidden,
        &CancellationToken::new(),
        &EventSender::noop(),
    );
    assert_eq!(report.refused.len(), tasks.len());
    assert_eq!(report.deleted, 0);
    assert!(base.path().join("precious.txt").exists());
    assert!(dest.join("docs/hidden/secret.txt").exists());

    // The archive a paused run left for a file still to download is protected.
    let download = DownloadTask {
        relative_path: "@a/addons/x.ebo".to_owned(),
        destination_dir: dest.join("@a/addons"),
        kind: DownloadTaskKind::File {
            size: 1,
            compressed: true,
            compressed_size: 1,
            sha1: None,
            complete: 0.0,
        },
        status: DownloadStatus::Done,
    };
    let report = delete_extra_files(
        &[task(&dest, "@a/addons/x.ebo.zip", false)],
        &DeleteRoots::from_plan(&a3s_sync::PlanTree::new(), &dest),
        &protected_paths(&[download]),
        &BTreeSet::new(),
        &CancellationToken::new(),
        &EventSender::noop(),
    );
    assert_eq!((report.deleted, report.skipped), (0, 1));
    assert!(dest.join("@a/addons/x.ebo.zip").exists());

    // A cancelled token deletes nothing.
    touch(&dest.join("extra.txt"));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let report = delete_extra_files(
        &[task(&dest, "extra.txt", false)],
        &DeleteRoots::from_plan(&a3s_sync::PlanTree::new(), &dest),
        &HashSet::new(),
        &BTreeSet::new(),
        &cancel,
        &EventSender::noop(),
    );
    assert_eq!(report.deleted, 0);
    assert!(dest.join("extra.txt").exists());
}

fn leaf(name: &str) -> SyncTreeNode {
    SyncTreeNode::Leaf(SyncTreeLeaf {
        name: name.to_owned(),
        sha1: Some("abc".to_owned()),
        size: 1,
        ..Default::default()
    })
}

fn dir(name: &str, children: Vec<SyncTreeNode>) -> SyncTreeNode {
    SyncTreeNode::Directory(SyncTreeDirectory {
        name: name.to_owned(),
        mark_as_addon: name.starts_with('@'),
        children,
        ..Default::default()
    })
}

/// Plans `children` into `dest` with everything selected.
fn plan(
    children: Vec<SyncTreeNode>,
    dest: &Path,
    addons: &(dyn a3s_sync::AddonLocator + Send + Sync),
) -> (a3s_sync::PlanTree, PlanSummary) {
    let mut root = SyncTreeDirectory::new_root();
    root.children = children;
    let metadata = RemoteMetadata {
        sync: Some(Arc::new(root)),
        ..Default::default()
    };
    let options = SyncOptions {
        repository_name: "r".to_owned(),
        event_name: None,
        destination: dest.to_path_buf(),
        auto_discover: true,
        exact_match: true,
        hidden_folder_paths: BTreeSet::new(),
        client_connections: 1,
        max_download_speed: 0,
        max_errors: 10,
    };
    let mut tree = plan_repository(
        &metadata,
        &options,
        addons,
        &mut a3s_repo_files::hash::Sha1Cache::default(),
        &CancellationToken::new(),
        &EventSender::noop(),
    )
    .expect("plan");
    select_all_except_root(&mut tree);
    let summary = PlanSummary::compute(&tree);
    (tree, summary)
}

#[test]
fn auto_discovered_addon_folders_are_roots() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    let elsewhere = base.path().join("elsewhere");
    std::fs::create_dir_all(&dest).expect("mkdir");
    touch(&elsewhere.join("@ace/addons/ace.pbo"));
    touch(&elsewhere.join("@ace/addons/stale.pbo"));
    touch(&elsewhere.join("sibling.txt"));
    let found = elsewhere.clone();
    let addons = move |name: &str| (name == "@ace").then(|| found.clone());
    let (tree, summary) = plan(
        vec![dir("@ace", vec![dir("addons", vec![leaf("ace.pbo")])])],
        &dest,
        &addons,
    );

    let roots = DeleteRoots::from_plan(&tree, &dest);
    assert!(roots.contains(&elsewhere.join("@ace/addons/stale.pbo")));
    assert!(!roots.contains(&elsewhere.join("sibling.txt")));
    assert!(!roots.contains(&elsewhere.join("@ace")));
    let report = delete_extra_files(
        &summary.delete_tasks,
        &roots,
        &HashSet::new(),
        &BTreeSet::new(),
        &CancellationToken::new(),
        &EventSender::noop(),
    );
    assert_eq!(report.deleted, 1, "{:?}", summary.delete_tasks);
    assert!(!elsewhere.join("@ace/addons/stale.pbo").exists());
    assert!(elsewhere.join("@ace/addons/ace.pbo").exists());
    assert!(elsewhere.join("sibling.txt").exists());
}

#[cfg(unix)]
#[test]
fn extras_inside_a_user_linked_addon_folder_are_listed_through_the_link() {
    let base = tempfile::tempdir().expect("tempdir");
    let dest = base.path().join("dest");
    let other_disk = base.path().join("other_disk/@a");
    touch(&other_disk.join("addons/a.pbo"));
    touch(&other_disk.join("addons/stale.pbo"));
    std::fs::create_dir_all(&dest).expect("mkdir");
    std::os::unix::fs::symlink(&other_disk, dest.join("@a")).expect("symlink");
    let (_, summary) = plan(
        vec![dir("@a", vec![dir("addons", vec![leaf("a.pbo")])])],
        &dest,
        &a3s_sync::plan::NoAddons,
    );
    let deleted: Vec<&str> = summary
        .delete_tasks
        .iter()
        .map(|t| t.relative_path.as_str())
        .collect();
    assert_eq!(deleted, ["@a/addons/stale.pbo"]);
    let report = delete_extra_files(
        &summary.delete_tasks,
        &DeleteRoots::from_plan(&a3s_sync::PlanTree::new(), &dest),
        &HashSet::new(),
        &BTreeSet::new(),
        &CancellationToken::new(),
        &EventSender::noop(),
    );
    assert_eq!(report.deleted, 1);
    assert!(!other_disk.join("addons/stale.pbo").exists());
    assert!(other_disk.join("addons/a.pbo").exists());
    assert!(std::fs::symlink_metadata(dest.join("@a")).is_ok_and(|m| m.file_type().is_symlink()));
}

#[cfg(unix)]
fn files_below(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            out.extend(files_below(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn copy_directory_cases() {
    let base = tempfile::tempdir().expect("tempdir");
    let source = base.path().join("source");
    let target = base.path().join("target");
    touch(&source.join("a.txt"));
    touch(&source.join("sub/b.txt"));
    std::fs::write(source.join("sub/b.txt"), b"new").expect("write");
    std::fs::create_dir_all(target.join("sub")).expect("mkdir");
    std::fs::write(target.join("sub/b.txt"), b"old").expect("old");
    std::fs::write(target.join("extra.txt"), b"keep").expect("extra");

    let stats = copy_directory(&source, &target).expect("copy");
    assert_eq!(stats.files, 2);
    assert_eq!(std::fs::read(target.join("sub/b.txt")).expect("b"), b"new");
    assert_eq!(
        std::fs::read(target.join("extra.txt")).expect("extra"),
        b"keep"
    );
    assert!(target.join("a.txt").exists());

    // The target is created one level deep only.
    let deep = base.path().join("missing/target");
    assert!(copy_directory(&source, &deep).is_err());

    let err = copy_directory(&base.path().join("nothing"), &target).expect_err("missing");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

    #[cfg(unix)]
    {
        let outside = base.path().join("outside");
        touch(&outside.join("secret.txt"));
        std::os::unix::fs::symlink(&outside, source.join("link")).expect("symlink");
        let fresh = base.path().join("fresh");
        let stats = copy_directory(&source, &fresh).expect("copy");
        assert_eq!(stats.skipped_links, 1);
        assert_eq!(files_below(&fresh).len(), 2);
        assert!(!fresh.join("link").exists());

        // Links already in the target are never written through.
        std::fs::remove_file(source.join("link")).expect("unlink");
        let linked = base.path().join("linked");
        std::fs::create_dir(&linked).expect("mkdir");
        std::os::unix::fs::symlink(&outside, linked.join("sub")).expect("symlink");
        std::os::unix::fs::symlink(outside.join("secret.txt"), linked.join("a.txt"))
            .expect("symlink");
        let stats = copy_directory(&source, &linked).expect("copy");
        assert_eq!((stats.files, stats.skipped_links), (0, 2));
        assert_eq!(files_below(&outside), [outside.join("secret.txt")]);
        assert_eq!(
            std::fs::read(outside.join("secret.txt")).expect("secret"),
            b"x"
        );
    }
}
