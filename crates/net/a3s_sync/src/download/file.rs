//! One task: directory creation, full download with resume, finalization (B5, B7-B16), and the
//! local path checks of decision D-15.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use a3s_transport::{CancellationToken, Transport};

use super::error::{Attempt, DownloadError, RetryState};
use super::executor::{TaskEnd, Worker};
use super::sink::DownloadedZip;
use super::{PART_EXTENSION, ZIP_EXTENSION, ZSYNC_EXTENSION, zsync};
use crate::remote::RemoteFile;
use crate::{DownloadStatus, DownloadTask, DownloadTaskKind, RemoteError};

/// `path` with [`PART_EXTENSION`] appended to its file name.
pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(PART_EXTENSION);
    PathBuf::from(name)
}

/// The remote path of a full download: `relative_path`, or `"{relative_path}.zip"` for
/// compressed leaves.
// Mirrors dao/connection/AbstractConnexionDAO.java:266-280
pub(crate) fn remote_path(task: &DownloadTask) -> String {
    match task.kind {
        DownloadTaskKind::File {
            compressed: true, ..
        } => format!("{}{ZIP_EXTENSION}", task.relative_path),
        _ => task.relative_path.clone(),
    }
}

/// The remote path of the zsync control file of a leaf.
// Mirrors dao/connection/http/HttpDAO.java:497-498
pub(crate) fn control_path(task: &DownloadTask) -> String {
    format!("{}{ZSYNC_EXTENSION}", task.relative_path)
}

/// The resume offset (B9): the length of the partial file iff the task is `Running` and
/// `0 < part_len < expected`.
// Mirrors dao/connection/AbstractConnexionDAO.java:282-293
pub(crate) fn resume_offset(
    status: DownloadStatus,
    part_len: Option<u64>,
    expected: u64,
) -> Option<u64> {
    let len = part_len?;
    (status == DownloadStatus::Running && len > 0 && len < expected).then_some(len)
}

/// The folder of the task's top-level tree ancestor: `destination_dir` without the trailing
/// folders that are the task's parent path segments.
fn destination_root(task: &DownloadTask) -> PathBuf {
    let mut root = task.destination_dir.clone();
    for segment in task.parent_relative_path().rsplit('/') {
        if root.file_name() == Some(OsStr::new(segment)) && root.pop() {
            continue;
        }
        break;
    }
    root
}

/// Checks that the task's local path stays inside its destination (decision D-15): every tree
/// segment is a plain name, the destination has no `..`, and no existing folder on the way is a
/// symbolic link leading out of the destination root.
// Mirrors none: Java joins names unchecked (service/RepositoryService.java:487-508)
/// The error is the reason of the rejection.
pub(crate) fn check_local_path(task: &DownloadTask) -> Result<(), &'static str> {
    if !task.relative_path.split('/').all(crate::plan::is_safe_name) {
        return Err("unsafe remote name");
    }
    if task
        .destination_dir
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("destination contains '..'");
    }
    let root = destination_root(task);
    let Ok(canonical_root) = std::fs::canonicalize(&root) else {
        // Nothing exists yet: every folder will be created fresh.
        return Ok(());
    };
    let mut probe = task.destination_dir.clone();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(&probe) {
            if !canonical.starts_with(&canonical_root) {
                return Err("a symbolic link leads out of the destination");
            }
            return Ok(());
        }
        if !probe.pop() {
            return Ok(());
        }
    }
}

/// [`check_local_path`] on the blocking pool; `Some(error)` when the task is rejected.
async fn check_local_path_async(task: &DownloadTask) -> Option<DownloadError> {
    let probe = task.clone();
    let reason = tokio::task::spawn_blocking(move || check_local_path(&probe).err())
        .await
        .unwrap_or(Some("path check failed"))?;
    Some(DownloadError::unsafe_path(&task.local_path(), reason))
}

/// Creates a directory task's folder (B5).
// Mirrors dao/connection/AbstractConnexionDAO.java:339-347
pub(crate) async fn create_directory(task: &DownloadTask) -> TaskEnd {
    if let Some(error) = check_local_path_async(task).await {
        return TaskEnd::Failed(error);
    }
    let path = task.local_path();
    match tokio::fs::create_dir_all(&path).await {
        Ok(()) => TaskEnd::Done(None),
        Err(err) => TaskEnd::Failed(DownloadError::create_directory(&path, err)),
    }
}

/// Runs one file task: zsync patching when it has a completion, else (or on fallback) a full
/// download.
// Mirrors dao/connection/AbstractConnexionDAO.java:241-360
pub(crate) async fn run_task(worker: &mut Worker, index: usize) -> TaskEnd {
    let task = worker.shared.tasks[index].clone();
    let DownloadTaskKind::File { complete, .. } = task.kind else {
        return create_directory(&task).await;
    };
    if let Some(error) = check_local_path_async(&task).await {
        return TaskEnd::Failed(error);
    }
    if complete > 0.0 {
        match zsync::run(worker, index, &task).await {
            zsync::ZsyncEnd::End(end) => return end,
            zsync::ZsyncEnd::Fallback(reason) => {
                tracing::warn!(
                    "Partial transfer failed for {}: {reason}; downloading the whole file",
                    task.relative_path
                );
            }
        }
    }
    full_download(worker, index, &task).await
}

/// The `RemoteFile` of a remote path below the repository root.
pub(crate) fn remote_file(remote: &str) -> RemoteFile {
    let (parent, name) = remote.rsplit_once('/').unwrap_or(("", remote));
    RemoteFile::new(name, parent, false)
}

/// A transport error on `remote` as a download error (WP07 B13 messages).
pub(crate) fn fetch_error(
    worker: &Worker,
    remote: &str,
    err: a3s_transport::Error,
) -> DownloadError {
    let err = normalize_not_found(err, remote);
    match RemoteError::fetch(&worker.shared.request.protocol, &remote_file(remote), err) {
        RemoteError::Cancelled => DownloadError::Cancelled,
        other => DownloadError::Remote(other),
    }
}

/// Some FTP servers accept `RETR` of a missing file and report `550` only when the transfer
/// ends; the transport then returns the raw reply. Java checks the file before `RETR`
/// (`FtpDAO.java:190-207`) and reports it as not found, so this does too.
fn normalize_not_found(err: a3s_transport::Error, remote: &str) -> a3s_transport::Error {
    match err {
        a3s_transport::Error::Ftp(suppaftp::FtpError::UnexpectedResponse(response))
            if response.status == suppaftp::Status::FileUnavailable =>
        {
            a3s_transport::Error::NotFound(remote.to_owned())
        }
        other => other,
    }
}

/// Races `fut` against the run token.
pub(crate) async fn race<T>(
    run: &CancellationToken,
    fut: impl Future<Output = a3s_transport::Result<T>>,
) -> a3s_transport::Result<T> {
    tokio::select! {
        biased;
        () = run.cancelled() => Err(a3s_transport::Error::Cancelled),
        result = fut => result,
    }
}

/// Removes `path` if it exists (a symbolic link is removed, not followed).
pub(crate) async fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Length of `path` if it is a regular file (not a symbolic link).
async fn regular_file_len(path: &Path) -> Option<u64> {
    tokio::fs::symlink_metadata(path)
        .await
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map(|m| m.len())
}

/// Syncs `part` to disk and renames it to `target`, replacing any existing file (B13, DV3).
pub(crate) async fn finalize(part: &Path, target: &Path) -> Result<(), DownloadError> {
    let sync = async {
        let file = tokio::fs::OpenOptions::new().write(true).open(part).await?;
        file.sync_all().await
    };
    sync.await
        .map_err(|err| DownloadError::local_io(part, err))?;
    tokio::fs::rename(part, target)
        .await
        .map_err(|err| DownloadError::local_io(target, err))
}

/// Whether the transport reports no reliable size itself (FTP, SFTP), so the final length is
/// checked against the expected size (B12, DV13).
fn checks_size_locally(transport: &Transport) -> bool {
    matches!(transport, Transport::Ftp(_) | Transport::Sftp(_))
}

/// A full download with resume (B7-B16).
// Mirrors dao/connection/AbstractConnexionDAO.java:243-318
#[allow(clippy::too_many_lines)]
async fn full_download(worker: &mut Worker, index: usize, task: &DownloadTask) -> TaskEnd {
    let DownloadTaskKind::File {
        size,
        compressed,
        compressed_size,
        ref sha1,
        ..
    } = task.kind
    else {
        return TaskEnd::Cancelled;
    };

    // B7
    if let Err(err) = tokio::fs::create_dir_all(&task.destination_dir).await {
        return TaskEnd::Failed(DownloadError::create_directory(&task.destination_dir, err));
    }

    // B8
    let target = if compressed {
        task.destination_dir
            .join(format!("{}{ZIP_EXTENSION}", task.name()))
    } else {
        task.local_path()
    };
    let expected = if compressed { compressed_size } else { size };
    let part = part_path(&target);
    let remote = remote_path(task);

    let mut retry = RetryState::default();
    loop {
        // B9
        let status = worker.shared.status(index);
        let part_len = regular_file_len(&part).await;
        let offset = if let Some(offset) = resume_offset(status, part_len, expected) {
            tracing::info!(
                "Resuming file: {} at offset: {offset}",
                super::error::absolute(&part).display()
            );
            offset
        } else {
            if let Err(err) = remove_if_exists(&part).await {
                return TaskEnd::Failed(DownloadError::local_io(&part, err));
            }
            tracing::info!(
                "Downloading file: {}",
                super::error::absolute(&part).display()
            );
            0
        };

        // B10, B11
        worker.shared.set_running(index);
        let progress = worker.shared.begin_attempt(worker.id, offset, expected);
        let ctx = worker.transfer_ctx(progress);
        let Some(transport) = worker.transport.as_mut() else {
            return TaskEnd::Cancelled;
        };
        let local_check = checks_size_locally(transport);
        let result = tokio::select! {
            biased;
            () = worker.shared.run.cancelled() => Err(a3s_transport::Error::Cancelled),
            result = transport.download_file(&remote, &part, offset, &ctx) => result,
        };
        let received = worker.shared.end_attempt(worker.id);

        // B12
        let result = result.and_then(|len| {
            if local_check && len < expected {
                Err(a3s_transport::Error::Incomplete {
                    expected,
                    actual: len,
                })
            } else {
                if len > expected {
                    tracing::warn!(
                        "{remote}: received {len} bytes, expected {expected}; keeping the file"
                    );
                }
                Ok(len)
            }
        });

        let Err(err) = result else {
            if worker.shared.run.is_cancelled() {
                return TaskEnd::Cancelled;
            }
            // Empty remote files still produce an (empty) file.
            if regular_file_len(&part).await.is_none()
                && let Err(err) = tokio::fs::File::create(&part).await
            {
                return TaskEnd::Failed(DownloadError::local_io(&part, err));
            }
            // B13
            if let Err(error) = finalize(&part, &target).await {
                return TaskEnd::Failed(error);
            }
            let zip = compressed.then(|| DownloadedZip {
                relative_path: task.relative_path.clone(),
                zip_path: target.clone(),
                target_path: task.local_path(),
                sha1: sha1.clone(),
                size,
            });
            return TaskEnd::Done(zip);
        };

        if worker.shared.run.is_cancelled() {
            return TaskEnd::Cancelled;
        }
        // B14, B15
        match retry.decide(&err, received, worker.reused()) {
            Attempt::Retry { reconnect } => {
                let message = match &err {
                    a3s_transport::Error::Incomplete { expected, actual } => {
                        DownloadError::incomplete(&target, *expected, *actual).to_string()
                    }
                    _ => fetch_error(worker, &remote, err).to_string(),
                };
                tracing::warn!("WARNING: {message}");
                if reconnect && let Err(end) = worker.reconnect().await {
                    return end;
                }
            }
            Attempt::ConnectionLost => {
                return TaskEnd::ConnectionLost(fetch_error(worker, &remote, err));
            }
            Attempt::FileError => {
                return TaskEnd::Failed(match err {
                    a3s_transport::Error::Incomplete { expected, actual } => {
                        DownloadError::incomplete(&target, expected, actual)
                    }
                    other => fetch_error(worker, &remote, other),
                });
            }
            Attempt::Cancelled => return TaskEnd::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_task(relative_path: &str, compressed: bool) -> DownloadTask {
        DownloadTask {
            relative_path: relative_path.to_owned(),
            destination_dir: PathBuf::from("/dest/@a/addons"),
            kind: DownloadTaskKind::File {
                size: 100,
                compressed,
                compressed_size: 60,
                sha1: None,
                complete: 0.0,
            },
            status: DownloadStatus::Pending,
        }
    }

    #[test]
    fn a2_resume_offset_table() {
        use DownloadStatus::{Pending, Running};
        assert_eq!(resume_offset(Running, Some(40), 100), Some(40));
        assert_eq!(resume_offset(Running, Some(0), 100), None);
        assert_eq!(resume_offset(Running, Some(100), 100), None);
        assert_eq!(resume_offset(Running, Some(150), 100), None);
        assert_eq!(resume_offset(Pending, Some(40), 100), None);
        assert_eq!(resume_offset(Running, None, 100), None);
    }

    #[test]
    fn a2_part_and_remote_paths() {
        assert_eq!(part_path(Path::new("x.pbo")), PathBuf::from("x.pbo.part"));
        assert_eq!(
            part_path(Path::new("/d/x.pbo.zip")),
            PathBuf::from("/d/x.pbo.zip.part")
        );
        let plain = file_task("@a/addons/x.pbo", false);
        assert_eq!(remote_path(&plain), "@a/addons/x.pbo");
        let compressed = file_task("@a/addons/x.pbo", true);
        assert_eq!(remote_path(&compressed), "@a/addons/x.pbo.zip");
        assert_eq!(control_path(&compressed), "@a/addons/x.pbo.zsync");

        let file = remote_file("@a/addons/x.pbo.zip");
        assert_eq!(file.relative_file_path(), "/@a/addons/x.pbo.zip");
        assert_eq!(remote_file("x").relative_file_path(), "/x");
    }

    #[test]
    fn a2_destination_root_strips_tree_folders() {
        let task = file_task("@a/addons/x.pbo", false);
        assert_eq!(destination_root(&task), PathBuf::from("/dest"));
        let mut redirected = task.clone();
        redirected.destination_dir = PathBuf::from("/other/@a/addons");
        assert_eq!(destination_root(&redirected), PathBuf::from("/other"));
        let mut root_level = task;
        root_level.relative_path = "x.pbo".to_owned();
        root_level.destination_dir = PathBuf::from("/dest");
        assert_eq!(destination_root(&root_level), PathBuf::from("/dest"));
    }

    #[test]
    fn a2_check_local_path_rejects_hostile_tasks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("dest");
        std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
        let task = |relative: &str, destination: PathBuf| DownloadTask {
            destination_dir: destination,
            ..file_task(relative, false)
        };
        assert!(check_local_path(&task("@a/x.pbo", dest.join("@a"))).is_ok());
        assert!(check_local_path(&task("@new/sub/x.pbo", dest.join("@new/sub"))).is_ok());
        for (relative, destination) in [
            ("..", dest.clone()),
            ("@a/..", dest.join("@a")),
            ("../x", dest.clone()),
            ("@a/a\\b", dest.join("@a")),
            ("/x", dest.clone()),
            ("@a/x", dest.join("@a/../..")),
        ] {
            check_local_path(&task(relative, destination)).expect_err(relative);
        }

        #[cfg(unix)]
        {
            let outside = dir.path().join("outside");
            std::fs::create_dir_all(&outside).expect("mkdir");
            std::os::unix::fs::symlink(&outside, dest.join("@link")).expect("symlink");
            let reason = check_local_path(&task("@link/x.pbo", dest.join("@link")))
                .expect_err("symlink escapes");
            assert!(reason.contains("symbolic link"), "{reason}");
            // A missing folder below the link escapes too.
            assert!(check_local_path(&task("@link/a/x.pbo", dest.join("@link/a"))).is_err());
        }
    }
}
