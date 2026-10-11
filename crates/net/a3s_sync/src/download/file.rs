//! One task: directory creation, full download with resume, finalization, and the
//! local path checks that keep every write inside the destination.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_transport::{CancellationToken, Transport};

use super::error::{Attempt, DownloadError, RetryState};
use super::executor::{TaskEnd, Worker};
use super::sink::DownloadedZip;
use super::{PART_EXTENSION, ZIP_EXTENSION, ZSYNC_EXTENSION, zsync};
use crate::remote::RemoteFile;
use crate::{DownloadStatus, DownloadTask, DownloadTaskKind, RemoteError, remote_relative_path};

/// `path` with [`PART_EXTENSION`] appended to its file name.
pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(PART_EXTENSION);
    PathBuf::from(name)
}

/// The remote path of a full download: the task's remote path, or `"{remote path}.zip"` for
/// compressed leaves.
// Mirrors dao/connection/AbstractConnexionDAO.java:266-280
pub(crate) fn remote_path(task: &DownloadTask) -> String {
    let remote = remote_relative_path(&task.relative_path);
    match task.kind {
        DownloadTaskKind::File {
            compressed: true, ..
        } => format!("{remote}{ZIP_EXTENSION}"),
        _ => remote.to_owned(),
    }
}

/// The remote path of the zsync control file of a leaf.
// Mirrors dao/connection/http/HttpDAO.java:497-498
pub(crate) fn control_path(task: &DownloadTask) -> String {
    format!(
        "{}{ZSYNC_EXTENSION}",
        remote_relative_path(&task.relative_path)
    )
}

/// The resume offset: the length of the partial file iff the task is `Running` and
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

/// Checks the task's tree path before anything is written: every segment
/// must be a plain name ([`crate::plan::is_safe_name`]), so the local path cannot leave the
/// destination folder through `..`, an absolute path or a separator. Symbolic links the user
/// created inside the destination (e.g. an `@mod` folder on another disk) are followed: a remote
/// repository cannot create local links. Returns the error for a rejected task.
// Mirrors none: Java joins names unchecked (service/RepositoryService.java:487-508)
pub(crate) fn check_local_path(task: &DownloadTask) -> Option<DownloadError> {
    if task.relative_path.split('/').all(crate::plan::is_safe_name) {
        None
    } else {
        Some(DownloadError::unsafe_path(
            &task.local_path(),
            &task.relative_path,
        ))
    }
}

/// Creates a directory task's folder.
// Mirrors dao/connection/AbstractConnexionDAO.java:339-347
pub(crate) async fn create_directory(task: &DownloadTask) -> TaskEnd {
    if let Some(error) = check_local_path(task) {
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
    if let Some(error) = check_local_path(&task) {
        return TaskEnd::Failed(error);
    }
    if complete > 0.0 {
        // A resumable full download (from an earlier fallback) wins over zsync, which would
        // otherwise reuse the same `.part` name.
        let (_, part, expected) = full_target(&task);
        let status = worker.shared.status(index);
        if resume_offset(status, regular_file_len(&part).await, expected).is_some() {
            tracing::info!(
                "Resuming the full download of {} instead of patching it",
                task.relative_path
            );
        } else {
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
    }
    full_download(worker, index, &task).await
}

/// The local target, its `.part` and the expected size of a full download.
// Mirrors dao/connection/AbstractConnexionDAO.java:266-280
fn full_target(task: &DownloadTask) -> (PathBuf, PathBuf, u64) {
    let (target, expected) = match task.kind {
        DownloadTaskKind::File {
            compressed: true,
            compressed_size,
            ..
        } => (
            task.destination_dir
                .join(format!("{}{ZIP_EXTENSION}", task.name())),
            compressed_size,
        ),
        DownloadTaskKind::File { size, .. } => (task.local_path(), size),
        DownloadTaskKind::Directory => (task.local_path(), 0),
    };
    let part = part_path(&target);
    (target, part, expected)
}

/// The `RemoteFile` of a remote path below the repository root.
pub(crate) fn remote_file(remote: &str) -> RemoteFile {
    let (parent, name) = remote.rsplit_once('/').unwrap_or(("", remote));
    RemoteFile::new(name, parent, false)
}

/// A transport error on `remote` as a download error.
pub(crate) fn fetch_error(
    worker: &Worker,
    remote: &str,
    err: a3s_transport::Error,
) -> DownloadError {
    match RemoteError::fetch(&worker.shared.request.protocol, &remote_file(remote), err) {
        RemoteError::Cancelled => DownloadError::Cancelled,
        other => DownloadError::Remote(other),
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

/// Syncs `part` to disk and renames it to `target`, replacing any existing file.
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

/// Whether the transport can continue a partial file (WebDAV always restarts from 0).
// Mirrors dao/connection/webdav/WebdavDAO.java:188
fn can_resume(transport: &Transport) -> bool {
    !matches!(transport, Transport::Webdav(_))
}

/// A full download with resume.
// Mirrors dao/connection/AbstractConnexionDAO.java:243-318
#[allow(clippy::too_many_lines)]
async fn full_download(worker: &mut Worker, index: usize, task: &DownloadTask) -> TaskEnd {
    let DownloadTaskKind::File {
        size,
        compressed,
        ref sha1,
        ..
    } = task.kind
    else {
        return TaskEnd::Cancelled;
    };

    if let Err(err) = tokio::fs::create_dir_all(&task.destination_dir).await {
        return TaskEnd::Failed(DownloadError::create_directory(&task.destination_dir, err));
    }

    let (target, part, expected) = full_target(task);
    let remote = remote_path(task);
    let shared = Arc::clone(&worker.shared);

    let mut retry = RetryState::default();
    loop {
        let resumable = worker.transport.as_ref().is_some_and(can_resume);
        let status = shared.status(index);
        let part_len = regular_file_len(&part).await;
        let offset =
            if let Some(offset) = resume_offset(status, part_len, expected).filter(|_| resumable) {
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

        shared.set_running(index);
        let ctx = shared.begin_attempt(worker.id, offset, expected);
        let Some(transport) = worker.transport.as_mut() else {
            return TaskEnd::Cancelled;
        };
        let result = tokio::select! {
            biased;
            () = shared.run.cancelled() => Err(a3s_transport::Error::Cancelled),
            result = transport.download_file(&remote, &part, offset, &ctx) => result,
        };
        let received = shared.end_attempt(worker.id);

        // A short file is incomplete on every protocol; a longer one is kept.
        let result = result.and_then(|len| {
            if len < expected {
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
            if shared.run.is_cancelled() {
                return TaskEnd::Cancelled;
            }
            // Empty remote files still produce an (empty) file.
            if regular_file_len(&part).await.is_none()
                && let Err(err) = tokio::fs::File::create(&part).await
            {
                return TaskEnd::Failed(DownloadError::local_io(&part, err));
            }
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

        if shared.run.is_cancelled() {
            return TaskEnd::Cancelled;
        }
        // The server has fewer bytes than the partial file: resuming cannot help.
        if offset > 0 && matches!(err, a3s_transport::Error::HttpStatus { status: 416, .. }) {
            if let Err(err) = remove_if_exists(&part).await {
                tracing::warn!("cannot delete {}: {err}", part.display());
            }
            return TaskEnd::Failed(DownloadError::incomplete(&target, expected, offset));
        }
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
    fn resume_offset_table() {
        use DownloadStatus::{Pending, Running};
        assert_eq!(resume_offset(Running, Some(40), 100), Some(40));
        assert_eq!(resume_offset(Running, Some(0), 100), None);
        assert_eq!(resume_offset(Running, Some(100), 100), None);
        assert_eq!(resume_offset(Running, Some(150), 100), None);
        assert_eq!(resume_offset(Pending, Some(40), 100), None);
        assert_eq!(resume_offset(Running, None, 100), None);
    }

    #[test]
    fn part_and_remote_paths() {
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
        // Below a folder named `racine` the remote path restarts at the root, like Java.
        let nested = file_task("@a/racine/x.pbo", true);
        assert_eq!(remote_path(&nested), "x.pbo.zip");
        assert_eq!(control_path(&nested), "x.pbo.zsync");

        let file = remote_file("@a/addons/x.pbo.zip");
        assert_eq!(file.relative_file_path(), "/@a/addons/x.pbo.zip");
        assert_eq!(remote_file("x").relative_file_path(), "/x");
    }

    #[test]
    fn check_local_path_rejects_hostile_tasks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("dest");
        std::fs::create_dir_all(dest.join("@a")).expect("mkdir");
        let task = |relative: &str, destination: PathBuf| DownloadTask {
            destination_dir: destination,
            ..file_task(relative, false)
        };
        assert!(check_local_path(&task("@a/x.pbo", dest.join("@a"))).is_none());
        assert!(check_local_path(&task("@new/sub/x.pbo", dest.join("@new/sub"))).is_none());
        for (relative, destination) in [
            ("..", dest.clone()),
            ("@a/..", dest.join("@a")),
            ("../x", dest.clone()),
            ("@a/a\\b", dest.join("@a")),
            ("/x", dest.clone()),
            ("", dest.clone()),
            ("@a//x", dest.join("@a")),
        ] {
            let err = check_local_path(&task(relative, destination)).expect(relative);
            assert!(matches!(err, DownloadError::UnsafePath { .. }), "{err:?}");
            assert_eq!(err.kind(), a3s_foundation::error::ErrorKind::InvalidData);
        }

        // Symbolic links the user created inside the destination are followed.
        #[cfg(unix)]
        {
            let outside = dir.path().join("outside");
            std::fs::create_dir_all(&outside).expect("mkdir");
            std::os::unix::fs::symlink(&outside, dest.join("@link")).expect("symlink");
            assert!(check_local_path(&task("@link/x.pbo", dest.join("@link"))).is_none());
            assert!(check_local_path(&task("@link/a/x.pbo", dest.join("@link/a"))).is_none());
        }
    }
}
