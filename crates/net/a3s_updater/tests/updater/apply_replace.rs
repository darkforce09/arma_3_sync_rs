//! Replacing the running executable, the Windows rename-away fallback and leftover cleanup.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

use a3s_updater::FileReplacer as _;
use a3s_updater::apply::replace::{FsOps, SelfReplacer, cleanup_leftovers};
use a3s_updater::elevation::is_permission_failure;
use parking_lot::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Copy(PathBuf, PathBuf),
    Rename(PathBuf, PathBuf),
    Remove(PathBuf),
}

/// File operations that only record what they are asked to do; renames answer with scripted
/// results (then `Ok`).
struct ScriptedFs {
    ops: Mutex<Vec<Op>>,
    renames: Mutex<VecDeque<io::Result<()>>>,
}

impl ScriptedFs {
    fn new(renames: Vec<io::Result<()>>) -> Self {
        Self {
            ops: Mutex::new(Vec::new()),
            renames: Mutex::new(renames.into()),
        }
    }
}

/// Shares one [`ScriptedFs`] between the test and the replacer.
struct Shared(std::sync::Arc<ScriptedFs>);

impl FsOps for Shared {
    fn copy(&self, from: &Path, to: &Path) -> io::Result<u64> {
        self.0
            .ops
            .lock()
            .push(Op::Copy(from.to_path_buf(), to.to_path_buf()));
        Ok(0)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.0
            .ops
            .lock()
            .push(Op::Rename(from.to_path_buf(), to.to_path_buf()));
        self.0.renames.lock().pop_front().unwrap_or(Ok(()))
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.0.ops.lock().push(Op::Remove(path.to_path_buf()));
        Ok(())
    }

    fn set_mode(&self, _path: &Path, _mode: u32) -> io::Result<()> {
        Ok(())
    }

    fn exists(&self, _path: &Path) -> bool {
        true
    }
}

fn denied() -> io::Error {
    io::Error::from(io::ErrorKind::PermissionDenied)
}

/// Runs one scripted replacement of `dir/arma3sync.exe`; returns the result, the operations and
/// the new and old temporary paths that were used.
fn replace(renames: Vec<io::Result<()>>) -> (io::Result<()>, Vec<Op>, PathBuf, PathBuf) {
    let fs = std::sync::Arc::new(ScriptedFs::new(renames));
    let replacer = SelfReplacer::with_fs(Box::new(Shared(fs.clone())), true);
    let dir = Path::new("installation");
    let result = replacer.replace(Path::new("staged"), &dir.join("arma3sync.exe"), None);
    let pid = std::process::id();
    let new = dir.join(format!(".a3s-new-{pid}-0-arma3sync.exe"));
    let old = dir.join(format!(".a3s-old-{pid}-0-arma3sync.exe"));
    let ops = fs.ops.lock().clone();
    (result, ops, new, old)
}

#[test]
fn locked_target_is_renamed_away_then_replaced() {
    let (result, ops, new, old) = replace(vec![Err(denied())]);
    result.expect("replaced");
    let target = Path::new("installation").join("arma3sync.exe");
    assert_eq!(
        ops,
        vec![
            Op::Copy(PathBuf::from("staged"), new.clone()),
            Op::Rename(new.clone(), target.clone()),
            Op::Rename(target.clone(), old.clone()),
            Op::Rename(new, target),
            Op::Remove(old),
        ]
    );
}

#[test]
fn failed_move_in_restores_the_old_file() {
    let (result, ops, new, old) = replace(vec![Err(denied()), Ok(()), Err(denied())]);
    assert!(result.is_err());
    let target = Path::new("installation").join("arma3sync.exe");
    assert_eq!(
        ops[1..],
        [
            Op::Rename(new.clone(), target.clone()),
            Op::Rename(target.clone(), old.clone()),
            Op::Rename(new.clone(), target.clone()),
            Op::Rename(old, target),
            Op::Remove(new),
        ]
    );
}

#[test]
fn other_rename_errors_do_not_fall_back() {
    let (result, ops, new, _) = replace(vec![Err(io::Error::from(io::ErrorKind::NotFound))]);
    assert_eq!(result.expect_err("fails").kind(), io::ErrorKind::NotFound);
    assert_eq!(ops.len(), 3);
    assert_eq!(ops[2], Op::Remove(new));
}

#[test]
fn failed_rename_away_returns_the_original_permission_error() {
    let (result, ops, new, _) = replace(vec![
        Err(denied()),
        Err(io::Error::other("the file is busy")),
    ]);
    let err = result.expect_err("fails");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert!(is_permission_failure(&err));
    assert_eq!(ops.len(), 4);
    assert_eq!(ops[3], Op::Remove(new));
}

#[cfg(target_os = "linux")]
#[test]
fn running_binary_is_replaced_without_text_file_busy() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("arma3sync");
    std::fs::copy("/bin/sleep", &target).expect("copy sleep");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    // A concurrent fork in another test can briefly hold the copy open for writing.
    let mut attempts = 0;
    let mut running = loop {
        match std::process::Command::new(&target).arg("5").spawn() {
            Err(err) if err.raw_os_error() == Some(26) && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            result => break result.expect("start"),
        }
    };
    let staged = temp.path().join("staged");
    std::fs::write(&staged, b"new binary").expect("staged");

    let result = SelfReplacer::new().replace(&staged, &target, Some(0o755));
    let alive = running.try_wait().expect("try_wait").is_none();
    running.kill().expect("kill");
    running.wait().expect("wait");

    result.expect("replaced");
    assert!(alive);
    assert_eq!(std::fs::read(&target).expect("read"), b"new binary");
    let mode = std::fs::metadata(&target)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    let leftovers: Vec<_> = std::fs::read_dir(temp.path())
        .expect("list")
        .map(|e| e.expect("entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with(".a3s-new-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn cleanup_removes_only_update_leftovers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let write = |relative: &str| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, b"x").expect("write");
    };
    write(".a3s-old-1-0-arma3sync.exe");
    write(".a3s-new-1-1-LICENSE.txt");
    write("sub/.a3s-old-1-2-lib.dll");
    write(".a3s-stage-abc/entry-1.tmp");
    write("a3s-old-x");
    write("arma3sync-rs-data/.a3s-old-1-3-x");
    write("a/b/c/d/e/.a3s-old-deep");

    assert_eq!(cleanup_leftovers(root), 4);
    assert!(!root.join(".a3s-old-1-0-arma3sync.exe").exists());
    assert!(!root.join(".a3s-new-1-1-LICENSE.txt").exists());
    assert!(!root.join("sub/.a3s-old-1-2-lib.dll").exists());
    assert!(!root.join(".a3s-stage-abc").exists());
    assert!(root.join("a3s-old-x").exists());
    assert!(root.join("arma3sync-rs-data/.a3s-old-1-3-x").exists());
    assert!(root.join("a/b/c/d/e/.a3s-old-deep").exists());
    assert_eq!(cleanup_leftovers(&root.join("missing")), 0);
}

#[cfg(unix)]
#[test]
fn cleanup_does_not_follow_links_or_fail_on_unreadable_directories() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside");
    let root = temp.path().join("installation");
    std::fs::create_dir_all(&outside).expect("mkdir");
    std::fs::create_dir_all(root.join("locked")).expect("mkdir");
    std::fs::write(outside.join(".a3s-old-1-0-x"), b"x").expect("write");
    std::os::unix::fs::symlink(&outside, root.join("link")).expect("symlink");
    std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o000))
        .expect("chmod");

    let removed = cleanup_leftovers(&root);
    std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod back");
    assert_eq!(removed, 0);
    assert!(outside.join(".a3s-old-1-0-x").exists());
}
