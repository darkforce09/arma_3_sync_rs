//! Aggregated progress of the upload workers: `Transfer` and `Speed` events.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use a3s_sync::{EventSender, RateLimiter, SyncEvent, TransferProgress};
use a3s_transport::{CancellationToken, TransferCtx};
use parking_lot::Mutex;

/// Interval of `Transfer` events while files are in flight.
const TRANSFER_INTERVAL: Duration = Duration::from_millis(100);

/// Minimum interval of `Speed` events.
// Mirrors service/administration/RepositoryUploadProcessor.java:227-242
const SPEED_INTERVAL: Duration = Duration::from_millis(500);

/// The file a worker is uploading.
#[derive(Default)]
struct Slot {
    /// Bytes of the current file sent so far (the worker's [`TransferCtx::progress`]).
    sent: Arc<AtomicU64>,
    current: Mutex<Option<(String, u64)>>,
}

struct Window {
    /// Worker that started a file last; its file is the "current file".
    last_started: Option<usize>,
    speed: RateLimiter,
    speed_sample: Option<(Instant, u64)>,
}

/// Progress of all upload workers. Uploads are never throttled.
pub(crate) struct Meter {
    total: u64,
    start: Instant,
    finished: AtomicU64,
    slots: Vec<Slot>,
    window: Mutex<Window>,
    events: EventSender,
}

impl Meter {
    pub(crate) fn new(total: u64, workers: usize, events: &EventSender) -> Self {
        Self {
            total,
            start: Instant::now(),
            finished: AtomicU64::new(0),
            slots: (0..workers.max(1)).map(|_| Slot::default()).collect(),
            window: Mutex::new(Window {
                last_started: None,
                speed: RateLimiter::new(SPEED_INTERVAL),
                speed_sample: None,
            }),
            events: events.clone(),
        }
    }

    /// The transfer context of worker `worker`.
    pub(crate) fn worker_ctx(&self, worker: usize, cancel: CancellationToken) -> TransferCtx {
        TransferCtx {
            cancel,
            progress: Arc::clone(&self.slots[worker].sent),
            throttle: None,
            ..TransferCtx::default()
        }
    }

    /// Worker `worker` starts uploading `path` (`len` bytes).
    pub(crate) fn start_file(&self, worker: usize, path: &str, len: u64) {
        let slot = &self.slots[worker];
        slot.sent.store(0, Ordering::Relaxed);
        *slot.current.lock() = Some((path.to_owned(), len));
        self.window.lock().last_started = Some(worker);
    }

    /// Worker `worker`'s file is complete.
    pub(crate) fn finish_file(&self, worker: usize, len: u64) {
        self.finished.fetch_add(len, Ordering::Relaxed);
        self.abandon_file(worker);
        self.emit();
    }

    /// Worker `worker`'s file failed; its bytes no longer count.
    pub(crate) fn abandon_file(&self, worker: usize) {
        let slot = &self.slots[worker];
        slot.sent.store(0, Ordering::Relaxed);
        *slot.current.lock() = None;
    }

    /// Bytes of finished files plus the bytes of the files in flight, at most the total.
    fn transferred(&self) -> u64 {
        let in_flight: u64 = self
            .slots
            .iter()
            .map(|s| s.sent.load(Ordering::Relaxed))
            .sum();
        (self.finished.load(Ordering::Relaxed) + in_flight).min(self.total)
    }

    /// Emits `Transfer`, and `Speed` when the speed window elapsed.
    // Mirrors service/administration/RepositoryUploadProcessor.java:200-242, 339-399
    pub(crate) fn emit(&self) {
        let transferred = self.transferred();
        let now = Instant::now();
        let mut window = self.window.lock();
        let (current_file, current_file_percent) = window
            .last_started
            .and_then(|worker| {
                let slot = &self.slots[worker];
                let current = slot.current.lock().clone()?;
                let sent = slot.sent.load(Ordering::Relaxed);
                Some((current.0, file_percent(sent, current.1)))
            })
            .map_or((None, 0), |(path, percent)| (Some(path), percent));
        self.events.emit(SyncEvent::Transfer(TransferProgress {
            transferred,
            total: self.total,
            current_file,
            current_file_percent,
            remaining: remaining(now.duration_since(self.start), transferred, self.total),
        }));
        if window.speed.ready(now) {
            if let Some((at, bytes)) = window.speed_sample {
                let seconds = now.duration_since(at).as_secs_f64();
                if seconds > 0.0 {
                    let bytes_per_second =
                        (transferred.saturating_sub(bytes) as f64 / seconds) as u64;
                    self.events.emit(SyncEvent::Speed { bytes_per_second });
                }
            }
            window.speed_sample = Some((now, transferred));
        }
    }

    /// Runs `work`, emitting `Transfer` events every 100 ms until it completes.
    pub(crate) async fn drive<F: Future>(&self, work: F) -> F::Output {
        let mut work = std::pin::pin!(work);
        let mut ticks = tokio::time::interval(TRANSFER_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticks.tick().await;
        loop {
            tokio::select! {
                output = &mut work => return output,
                _ = ticks.tick() => self.emit(),
            }
        }
    }
}

/// `sent * 100 / len`, 0 for an empty file.
// Mirrors dao/connection/AbstractConnexionDAO.java:584-592
fn file_percent(sent: u64, len: u64) -> u8 {
    if len == 0 {
        0
    } else {
        (u128::from(sent.min(len)) * 100 / u128::from(len)) as u8
    }
}

/// `elapsed * (total - transferred) / transferred`, whole seconds; unknown before any byte.
// Mirrors service/administration/RepositoryUploadProcessor.java:209-219
fn remaining(elapsed: Duration, transferred: u64, total: u64) -> Option<Duration> {
    if transferred == 0 {
        return None;
    }
    let seconds =
        elapsed.as_secs_f64() * total.saturating_sub(transferred) as f64 / transferred as f64;
    Some(Duration::from_secs(seconds as u64))
}
