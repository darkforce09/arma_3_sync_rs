//! zsync partial download over HTTP (B17-B22).

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use a3s_transport::{TransferCtx, Transport};
use a3s_zsync::{BlockMatches, ControlFile, Plan};
use tokio::sync::mpsc;

use super::error::{DownloadError, absolute, is_connection_class};
use super::executor::{TaskEnd, Worker};
use super::file::{control_path, fetch_error, finalize, part_path, race, remove_if_exists};
use crate::DownloadTask;

/// Range bodies buffered between the network and the assembler.
const CHANNEL_CAPACITY: usize = 4;

/// How a zsync attempt ended.
#[derive(Debug)]
pub(crate) enum ZsyncEnd {
    /// The task ended (done, failed, lost or cancelled).
    End(TaskEnd),
    /// Partial transfer cannot be used; download the whole file (B22).
    Fallback(String),
}

/// A cancellation flag for blocking work, set when the run token is cancelled.
struct CancelFlag {
    flag: Arc<AtomicBool>,
    watcher: tokio::task::JoinHandle<()>,
}

impl CancelFlag {
    fn new(worker: &Worker) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let run = worker.shared.run.clone();
        let set = Arc::clone(&flag);
        let watcher = tokio::spawn(async move {
            run.cancelled().await;
            set.store(true, Ordering::Relaxed);
        });
        Self { flag, watcher }
    }
}

impl Drop for CancelFlag {
    fn drop(&mut self) {
        self.watcher.abort();
    }
}

/// Reads the concatenated range bodies from the async side.
struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    current: Vec<u8>,
    position: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position >= self.current.len() {
            match self.rx.blocking_recv() {
                Some(chunk) => {
                    self.current = chunk;
                    self.position = 0;
                }
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "remote data stream ended",
                    ));
                }
            }
        }
        let n = buf.len().min(self.current.len() - self.position);
        buf[..n].copy_from_slice(&self.current[self.position..self.position + n]);
        self.position += n;
        Ok(n)
    }
}

/// A writer that fails once the run is cancelled, so `assemble` stops quickly.
struct CancelWriter<W> {
    inner: W,
    cancel: Arc<AtomicBool>,
}

impl<W: Write> Write for CancelWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Patches the local file of `task` with zsync (B17-B21).
// Mirrors dao/connection/AbstractConnexionDAO.java:319-338, dao/connection/http/HttpDAO.java:492-524
pub(crate) async fn run(worker: &mut Worker, index: usize, task: &DownloadTask) -> ZsyncEnd {
    // B17
    if !matches!(worker.transport, Some(Transport::Http(_))) {
        return ZsyncEnd::Fallback("partial transfer requires HTTP".to_owned());
    }
    let crate::DownloadTaskKind::File { size, .. } = task.kind else {
        return ZsyncEnd::Fallback("not a file".to_owned());
    };
    if let Err(err) = tokio::fs::create_dir_all(&task.destination_dir).await {
        return ZsyncEnd::End(TaskEnd::Failed(DownloadError::create_directory(
            &task.destination_dir,
            err,
        )));
    }
    worker.shared.set_running(index);

    let mut idle_reconnect_used = false;
    loop {
        let reused = worker.reused() && !idle_reconnect_used;
        match attempt(worker, task, size).await {
            Attempt::End(end) => return end,
            Attempt::Transport {
                err,
                remote,
                received,
            } => {
                if worker.shared.run.is_cancelled()
                    || matches!(err, a3s_transport::Error::Cancelled)
                {
                    return ZsyncEnd::End(TaskEnd::Cancelled);
                }
                if matches!(err, a3s_transport::Error::Unsupported { .. }) {
                    return ZsyncEnd::Fallback(err.to_string());
                }
                if is_connection_class(&err) {
                    if received == 0 && reused {
                        idle_reconnect_used = true;
                        tracing::warn!("WARNING: {}", fetch_error(worker, &remote, err));
                        if let Err(end) = worker.reconnect().await {
                            return ZsyncEnd::End(end);
                        }
                        continue;
                    }
                    return ZsyncEnd::End(TaskEnd::ConnectionLost(fetch_error(
                        worker, &remote, err,
                    )));
                }
                return ZsyncEnd::End(TaskEnd::Failed(fetch_error(worker, &remote, err)));
            }
        }
    }
}

/// Result of one zsync attempt.
enum Attempt {
    End(ZsyncEnd),
    /// A transport error on `remote` after `received` payload bytes.
    Transport {
        err: a3s_transport::Error,
        remote: String,
        received: u64,
    },
}

impl From<ZsyncEnd> for Attempt {
    fn from(end: ZsyncEnd) -> Self {
        Self::End(end)
    }
}

/// One attempt: control file, match, ranges, assembly, rename.
async fn attempt(worker: &mut Worker, task: &DownloadTask, size: u64) -> Attempt {
    // B18
    let control_remote = control_path(task);
    let control_ctx = TransferCtx {
        cancel: worker.shared.run.clone(),
        progress: Arc::new(AtomicU64::new(0)),
        throttle: None,
    };
    let Some(transport) = worker.transport.as_mut() else {
        return ZsyncEnd::End(TaskEnd::Cancelled).into();
    };
    let bytes = match race(
        &worker.shared.run,
        transport.fetch(&control_remote, &control_ctx),
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(a3s_transport::Error::NotFound(_)) => {
            return ZsyncEnd::Fallback(format!("{control_remote} not found")).into();
        }
        Err(err) => {
            return Attempt::Transport {
                err,
                remote: control_remote,
                received: 0,
            };
        }
    };
    let control = match ControlFile::parse(&bytes) {
        Ok(control) => Arc::new(control),
        Err(err) => return ZsyncEnd::Fallback(err.to_string()).into(),
    };
    if control.header.length != size {
        return ZsyncEnd::Fallback(format!(
            "control file length {} differs from the file size {size}",
            control.header.length
        ))
        .into();
    }

    // B19
    let local = task.local_path();
    let matches = match match_local(worker, &control, &local).await {
        Ok(matches) => Arc::new(matches),
        Err(MatchError::Cancelled) => return ZsyncEnd::End(TaskEnd::Cancelled).into(),
        Err(MatchError::Unreadable(reason)) => return ZsyncEnd::Fallback(reason).into(),
    };
    let plan = Plan::new(&control, &matches, Plan::MAX_BLOCKS_PER_RANGE);

    // B20, B21
    let part = part_path(&local);
    if let Err(err) = remove_if_exists(&part).await {
        return ZsyncEnd::End(TaskEnd::Failed(DownloadError::local_io(&part, err))).into();
    }
    let flag = CancelFlag::new(worker);
    let (tx, rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAPACITY);
    let assembler = spawn_assemble(
        Arc::clone(&control),
        Arc::clone(&matches),
        local.clone(),
        part.clone(),
        rx,
        Arc::clone(&flag.flag),
    );

    let progress = worker
        .shared
        .begin_attempt(worker.id, 0, plan.download_size());
    let ctx = worker.transfer_ctx(progress);
    let part_display = absolute(&part);
    let mut transport_error = None;
    if let Some(transport) = worker.transport.as_mut() {
        for range in &plan.ranges {
            tracing::info!(
                "Downloading partial file: {} at range [{}-{}]",
                part_display.display(),
                range.start,
                range.end - 1
            );
            let fetched = race(
                &worker.shared.run,
                transport.fetch_range(&task.relative_path, range.clone(), &ctx),
            )
            .await;
            match fetched {
                Ok(body) => {
                    let sent = tokio::select! {
                        biased;
                        () = worker.shared.run.cancelled() => false,
                        sent = tx.send(body) => sent.is_ok(),
                    };
                    if !sent {
                        // Cancelled, or the assembler stopped early (its error follows).
                        break;
                    }
                }
                Err(err) => {
                    transport_error = Some(err);
                    break;
                }
            }
        }
    }
    drop(tx);
    let assembled = match assembler.await {
        Ok(result) => result,
        Err(err) => Err(a3s_zsync::Error::Io(io::Error::other(err))),
    };
    let received = worker.shared.end_attempt(worker.id);
    drop(flag);

    let delete_part = || async {
        if let Err(err) = remove_if_exists(&part).await {
            tracing::warn!("cannot delete {}: {err}", part.display());
        }
    };
    if let Some(err) = transport_error {
        delete_part().await;
        return Attempt::Transport {
            err,
            remote: task.relative_path.clone(),
            received,
        };
    }
    if worker.shared.run.is_cancelled() {
        delete_part().await;
        return ZsyncEnd::End(TaskEnd::Cancelled).into();
    }
    match assembled {
        Ok(()) => match finalize(&part, &local).await {
            Ok(()) => ZsyncEnd::End(TaskEnd::Done(None)).into(),
            Err(error) => {
                delete_part().await;
                ZsyncEnd::End(TaskEnd::Failed(error)).into()
            }
        },
        Err(err @ a3s_zsync::Error::ChecksumMismatch { .. }) => {
            delete_part().await;
            ZsyncEnd::Fallback(err.to_string()).into()
        }
        Err(err) => {
            delete_part().await;
            ZsyncEnd::End(TaskEnd::Failed(DownloadError::zsync(
                &task.relative_path,
                err,
            )))
            .into()
        }
    }
}

enum MatchError {
    Cancelled,
    Unreadable(String),
}

/// Matches the local file against the control file on the blocking pool (B19).
async fn match_local(
    worker: &Worker,
    control: &Arc<ControlFile>,
    local: &Path,
) -> Result<BlockMatches, MatchError> {
    let flag = CancelFlag::new(worker);
    let cancel = Arc::clone(&flag.flag);
    let control = Arc::clone(control);
    let local = local.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&local).map_err(a3s_zsync::Error::Io)?;
        a3s_zsync::match_blocks(&control, io::BufReader::new(file), Some(&cancel))
    })
    .await;
    match result {
        Ok(Ok(matches)) => Ok(matches),
        Ok(Err(a3s_zsync::Error::Cancelled)) => Err(MatchError::Cancelled),
        Ok(Err(err)) if worker.shared.run.is_cancelled() => {
            tracing::debug!("zsync match interrupted: {err}");
            Err(MatchError::Cancelled)
        }
        Ok(Err(err)) => Err(MatchError::Unreadable(format!(
            "local file unreadable: {err}"
        ))),
        Err(err) => Err(MatchError::Unreadable(format!("match task failed: {err}"))),
    }
}

/// Assembles the new file into `part` on the blocking pool (B21).
fn spawn_assemble(
    control: Arc<ControlFile>,
    matches: Arc<BlockMatches>,
    local: PathBuf,
    part: PathBuf,
    rx: mpsc::Receiver<Vec<u8>>,
    cancel: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<a3s_zsync::Result<()>> {
    tokio::task::spawn_blocking(move || {
        let local = io::BufReader::new(std::fs::File::open(&local)?);
        let out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        let mut writer = CancelWriter {
            inner: io::BufWriter::new(out),
            cancel,
        };
        let remote = ChannelReader {
            rx,
            current: Vec::new(),
            position: 0,
        };
        a3s_zsync::assemble(&control, &matches, local, remote, &mut writer)?;
        let out = writer
            .inner
            .into_inner()
            .map_err(|err| a3s_zsync::Error::Io(err.into_error()))?;
        out.sync_all()?;
        Ok(())
    })
}
