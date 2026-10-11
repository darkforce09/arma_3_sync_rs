//! Progress monitor: transfer snapshots, speed, disk usage and connection waiting (B30-B35).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use a3s_transport::CancellationToken;
use tokio::time::MissedTickBehavior;

use super::executor::Shared;
use super::{DISK_USAGE_INTERVAL, SPEED_INTERVAL, TRANSFER_EVENT_INTERVAL, WAITING_TICK};
use crate::{SyncEvent, TransferProgress};

/// Local write throughput from two free-space samples `elapsed` apart. More free space (files
/// being deleted) counts as 0.
// Mirrors service/synchronization/FilesSynchronizationDiskUsageListerner.java:43-61
pub fn disk_usage_rate(free_before: u64, free_after: u64, elapsed: Duration) -> u64 {
    if free_after >= free_before || elapsed.is_zero() {
        return 0;
    }
    let written = (free_before - free_after) as f64;
    (written / elapsed.as_secs_f64()) as u64
}

/// The running mean of the speed for the report: a 0 sample keeps `previous`, the first
/// non-zero sample is taken as is, later ones are averaged with the previous value.
// Mirrors service/synchronization/FilesSynchronizationProcessor.java:263-270
pub fn average_speed(previous: u64, sample: u64) -> u64 {
    if sample == 0 {
        previous
    } else if previous == 0 {
        sample
    } else {
        previous / 2 + sample / 2 + (previous % 2 + sample % 2) / 2
    }
}

/// `bytes * 100 / expected`, 0 when `expected` is 0, at most 100.
// Mirrors dao/connection/AbstractConnexionDAO.java:627-634
pub fn file_percent(bytes: u64, expected: u64) -> u8 {
    if expected == 0 {
        return 0;
    }
    (u128::from(bytes) * 100 / u128::from(expected)).min(100) as u8
}

/// Estimated remaining time: `(total - transferred) * elapsed / run_bytes`, `None` while
/// nothing was received in this run (DV7).
// Mirrors service/synchronization/FilesSynchronizationProcessor.java:211-222
pub fn remaining_time(
    total: u64,
    transferred: u64,
    run_bytes: u64,
    elapsed: Duration,
) -> Option<Duration> {
    if run_bytes == 0 {
        return None;
    }
    let remaining = u128::from(total.saturating_sub(transferred));
    let nanos = remaining * elapsed.as_nanos() / u128::from(run_bytes);
    Some(Duration::from_nanos(
        u64::try_from(nanos).unwrap_or(u64::MAX),
    ))
}

/// Seconds without visible progress (`observerConnectionWaiting`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaitingCounter {
    seconds: u32,
}

impl WaitingCounter {
    /// Advances the counter by one tick and returns its value. It resets when no connection is
    /// active, when unzip is busy, when the disk is being written, or when network bytes
    /// arrived since the previous tick (DV8); otherwise it adds one.
    // Mirrors service/synchronization/FilesSynchronizationDiskUsageListerner.java:80-102
    pub fn tick(
        &mut self,
        any_active: bool,
        unzip_idle: bool,
        disk_usage: u64,
        bytes_advanced: bool,
    ) -> u32 {
        if !any_active || !unzip_idle || disk_usage > 0 || bytes_advanced {
            self.seconds = 0;
        } else {
            self.seconds = self.seconds.saturating_add(1);
        }
        self.seconds
    }

    /// The current value.
    pub fn seconds(self) -> u32 {
        self.seconds
    }
}

/// The free space of `path`, read on the blocking pool.
async fn free_space(path: PathBuf) -> Option<u64> {
    let result = tokio::task::spawn_blocking(move || fs4::free_space(&path)).await;
    match result {
        Ok(Ok(free)) => Some(free),
        Ok(Err(err)) => {
            tracing::debug!("cannot read free disk space: {err}");
            None
        }
        Err(err) => {
            tracing::debug!("free disk space task failed: {err}");
            None
        }
    }
}

fn ticker(period: Duration) -> tokio::time::Interval {
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

/// A transfer snapshot without the remaining time, which changes on every tick.
fn comparable(progress: &TransferProgress) -> TransferProgress {
    TransferProgress {
        remaining: None,
        ..progress.clone()
    }
}

/// Emits the periodic events until `stop` is cancelled.
// Mirrors service/synchronization/FilesSynchronizationDiskUsageListerner.java:32-111,
// service/synchronization/FilesSynchronizationProcessor.java:247-272
pub(crate) async fn run(shared: Arc<Shared>, stop: CancellationToken) {
    let mut transfer_tick = ticker(TRANSFER_EVENT_INTERVAL);
    let mut speed_tick = ticker(SPEED_INTERVAL);
    let mut disk_tick = ticker(DISK_USAGE_INTERVAL);
    let mut waiting_tick = ticker(WAITING_TICK);

    let disk_path = shared.request.disk_usage_path.clone();
    let mut last_free = match &disk_path {
        Some(path) => free_space(path.clone()).await.map(|f| (Instant::now(), f)),
        None => None,
    };
    let mut disk_usage = 0u64;

    let mut last_transfer: Option<TransferProgress> = None;
    let mut last_speed = (Instant::now(), shared.snapshot().run_bytes);
    let mut last_waiting_bytes = last_speed.1;
    let mut waiting = WaitingCounter::default();

    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => break,
            _ = transfer_tick.tick() => {
                let progress = shared.snapshot().progress;
                let key = comparable(&progress);
                if last_transfer.as_ref() != Some(&key) {
                    last_transfer = Some(key);
                    shared.events.emit(SyncEvent::Transfer(progress));
                }
            }
            _ = speed_tick.tick() => {
                let now = Instant::now();
                let bytes = shared.snapshot().run_bytes;
                let elapsed = now.saturating_duration_since(last_speed.0);
                let delta = bytes.saturating_sub(last_speed.1);
                let speed = if elapsed.is_zero() {
                    0
                } else {
                    (delta as f64 / elapsed.as_secs_f64()) as u64
                };
                last_speed = (now, bytes);
                shared.events.emit(SyncEvent::Speed { bytes_per_second: speed });
                let average = average_speed(shared.average_speed.load(Ordering::Relaxed), speed);
                shared.average_speed.store(average, Ordering::Relaxed);
            }
            _ = disk_tick.tick(), if disk_path.is_some() => {
                let Some(path) = disk_path.clone() else { continue };
                if let Some(free) = free_space(path).await {
                    let now = Instant::now();
                    if let Some((then, before)) = last_free {
                        disk_usage = disk_usage_rate(before, free, now.saturating_duration_since(then));
                        shared.events.emit(SyncEvent::DiskUsage { bytes_per_second: disk_usage });
                    }
                    last_free = Some((now, free));
                }
            }
            _ = waiting_tick.tick() => {
                let snapshot = shared.snapshot();
                let advanced = snapshot.run_bytes != last_waiting_bytes;
                last_waiting_bytes = snapshot.run_bytes;
                let seconds = waiting.tick(
                    snapshot.active > 0,
                    shared.zips.is_idle(),
                    disk_usage,
                    advanced,
                );
                shared.events.emit(SyncEvent::ConnectionWaiting { seconds });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a3_disk_usage_rate() {
        assert_eq!(disk_usage_rate(1000, 400, Duration::from_millis(500)), 1200);
        assert_eq!(disk_usage_rate(400, 1000, Duration::from_millis(500)), 0);
        assert_eq!(disk_usage_rate(400, 400, Duration::from_millis(500)), 0);
        assert_eq!(disk_usage_rate(1000, 400, Duration::ZERO), 0);
    }

    #[test]
    fn a3_average_speed_sequence() {
        let mut avg = 0;
        avg = average_speed(avg, 100);
        assert_eq!(avg, 100);
        avg = average_speed(avg, 300);
        assert_eq!(avg, 200);
        avg = average_speed(avg, 0);
        assert_eq!(avg, 200);
        assert_eq!(average_speed(u64::MAX, u64::MAX), u64::MAX);
        assert_eq!(average_speed(3, 4), 3);
    }

    #[test]
    fn a3_file_percent() {
        assert_eq!(file_percent(10, 0), 0);
        assert_eq!(file_percent(1, 3), 33);
        assert_eq!(file_percent(2, 3), 66);
        assert_eq!(file_percent(50, 100), 50);
        assert_eq!(file_percent(150, 100), 100);
        assert_eq!(file_percent(u64::MAX, 1), 100);
    }

    #[test]
    fn a3_remaining_time() {
        assert_eq!(remaining_time(100, 0, 0, Duration::from_secs(5)), None);
        assert_eq!(
            remaining_time(1000, 250, 250, Duration::from_secs(10)),
            Some(Duration::from_secs(30))
        );
        // Resumed bytes count as transferred but not as received in this run.
        assert_eq!(
            remaining_time(1000, 600, 100, Duration::from_secs(1)),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            remaining_time(10, 20, 5, Duration::from_secs(1)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a3_waiting_counter_branches() {
        let mut c = WaitingCounter::default();
        // Idle ticks count up.
        assert_eq!(c.tick(true, true, 0, false), 1);
        assert_eq!(c.tick(true, true, 0, false), 2);
        assert_eq!(c.tick(true, true, 0, false), 3);
        assert_eq!(c.seconds(), 3);
        // No active connection.
        assert_eq!(c.tick(false, true, 0, false), 0);
        assert_eq!(c.tick(true, true, 0, false), 1);
        // Unzip busy.
        assert_eq!(c.tick(true, false, 0, false), 0);
        assert_eq!(c.tick(true, true, 0, false), 1);
        // Disk being written.
        assert_eq!(c.tick(true, true, 10, false), 0);
        assert_eq!(c.tick(true, true, 0, false), 1);
        // Network bytes arrived (DV8).
        assert_eq!(c.tick(true, true, 0, true), 0);
    }
}
