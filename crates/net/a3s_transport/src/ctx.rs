use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Global download speed limit shared by all connections of a synchronization.
#[derive(Debug, Default)]
pub struct Throttle {
    /// Bytes per second; 0 means unlimited.
    limit: AtomicU64,
    window: Mutex<Option<(Instant, u64)>>,
}

impl Throttle {
    pub fn new(bytes_per_second: u64) -> Self {
        Self {
            limit: AtomicU64::new(bytes_per_second),
            window: Mutex::new(None),
        }
    }

    pub fn set_limit(&self, bytes_per_second: u64) {
        self.limit.store(bytes_per_second, Ordering::Relaxed);
        *self.window.lock() = None;
    }

    /// Accounts for `n` transferred bytes, sleeping if we are ahead of the limit.
    pub async fn consume(&self, n: usize) {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 {
            return;
        }
        let wait = {
            let mut window = self.window.lock();
            let (start, bytes) = window.get_or_insert_with(|| (Instant::now(), 0));
            if start.elapsed() > Duration::from_secs(2) {
                *start = Instant::now();
                *bytes = 0;
            }
            *bytes += n as u64;
            let due = Duration::from_secs_f64(*bytes as f64 / limit as f64);
            due.saturating_sub(start.elapsed())
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Per-transfer context: cancellation, progress counter and throttling.
#[derive(Clone, Debug, Default)]
pub struct TransferCtx {
    pub cancel: CancellationToken,

    /// Incremented with every byte received or sent.
    pub progress: Arc<AtomicU64>,
    pub throttle: Option<Arc<Throttle>>,

    /// Where a file download actually started writing, set once the destination is opened.
    pub start_offset: StartOffset,
}

/// The offset a download actually resumed at.
///
/// The requested offset, or 0 when the server or protocol restarted the file (HTTP `200` to a
/// range request, FTP without `REST`, WebDAV). Unset until the destination file is opened;
/// clones share the value.
#[derive(Clone, Debug)]
pub struct StartOffset(Arc<AtomicU64>);

impl StartOffset {
    /// Marker of "not opened yet".
    const UNSET: u64 = u64::MAX;

    /// The offset, or `None` before the destination was opened.
    pub fn get(&self) -> Option<u64> {
        let value = self.0.load(Ordering::Acquire);
        (value != Self::UNSET).then_some(value)
    }

    /// Records the offset; a later call (a restart within the same transfer) replaces it.
    pub(crate) fn set(&self, offset: u64) {
        self.0.store(offset.min(Self::UNSET - 1), Ordering::Release);
    }
}

impl Default for StartOffset {
    fn default() -> Self {
        Self(Arc::new(AtomicU64::new(Self::UNSET)))
    }
}

impl TransferCtx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Counts `n` transferred bytes into the progress and, with a throttle, waits until they
    /// are within its limit, in steps of at most [`ACCOUNT_STEP`]. Cancellation ends the wait.
    pub(crate) async fn account(&self, n: usize) -> Result<()> {
        let Some(throttle) = &self.throttle else {
            self.progress.fetch_add(n as u64, Ordering::Relaxed);
            return Ok(());
        };
        let mut left = n;
        while left > 0 {
            let step = left.min(ACCOUNT_STEP);
            left -= step;
            self.progress.fetch_add(step as u64, Ordering::Relaxed);
            tokio::select! {
                () = self.cancel.cancelled() => return Err(Error::Cancelled),
                () = throttle.consume(step) => {}
            }
        }
        Ok(())
    }
}

/// Largest share of a read accounted at once under a throttle. HTTP bodies arrive in chunks of
/// up to several hundred KiB (hyper grows its read buffer): accounted whole, progress jumps by
/// a chunk and the throttle then sleeps for all of it, so the reported speed swings between
/// zero and a multiple of the limit.
const ACCOUNT_STEP: usize = 16 * 1024;

/// Chunk size of transfers (also the SFTP write size, `SftpDAO.SFTP_WRITE_BUFFER_SIZE`).
pub(crate) const COPY_BUFFER: usize = 64 * 1024;

/// Copies `reader` to `writer`, honouring cancellation, read timeout, throttle and progress.
pub(crate) async fn copy<R, W>(
    reader: &mut R,
    writer: &mut W,
    ctx: &TransferCtx,
    read_timeout: Option<Duration>,
) -> Result<u64>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    copy_with_timeouts(reader, writer, ctx, read_timeout, None).await
}

/// Copies a local `reader` to a remote `writer` (uploads): the timeout guards each remote
/// write instead of the local reads. Progress counts bytes handed to the remote side
/// (`AbstractConnexionDAO.UploadProgressListener`).
pub(crate) async fn copy_upload<R, W>(
    reader: &mut R,
    writer: &mut W,
    ctx: &TransferCtx,
    write_timeout: Option<Duration>,
) -> Result<u64>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    copy_with_timeouts(reader, writer, ctx, None, write_timeout).await
}

async fn copy_with_timeouts<R, W>(
    reader: &mut R,
    writer: &mut W,
    ctx: &TransferCtx,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
) -> Result<u64>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
{
    let mut buf = vec![0u8; COPY_BUFFER];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buf);
        let n = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            res = maybe_timeout(read_timeout, read) => res?,
        };
        if n == 0 {
            break;
        }
        let write = writer.write_all(&buf[..n]);
        tokio::select! {
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            res = maybe_timeout(write_timeout, write) => res?,
        }
        total += n as u64;
        ctx.account(n).await?;
    }
    let flush = writer.flush();
    tokio::select! {
        () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
        res = maybe_timeout(write_timeout, flush) => res?,
    }
    Ok(total)
}

pub(crate) async fn maybe_timeout<T>(
    timeout: Option<Duration>,
    fut: impl Future<Output = std::io::Result<T>>,
) -> Result<T> {
    match timeout {
        Some(t) => tokio::time::timeout(t, fut)
            .await
            .map_err(|_elapsed| Error::Timeout)?
            .map_err(Error::from),
        None => fut.await.map_err(Error::from),
    }
}
