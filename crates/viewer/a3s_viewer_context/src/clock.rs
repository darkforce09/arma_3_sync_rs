use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::FutureExt as _;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tokio::sync::{Notify, oneshot};

/// Wall-clock time and sleeping, injectable so schedules are testable.
pub trait Clock: Send + Sync + 'static {
    /// The current time.
    fn now(&self) -> SystemTime;

    /// Resolves once [`Self::now`] has reached `deadline`; at once if it already has.
    fn sleep_until(&self, deadline: SystemTime) -> BoxFuture<'static, ()>;
}

/// The system clock.
///
/// Sleeps in slices of at most 30 s and re-reads the wall clock after each one, so a deadline
/// that passed while the machine was suspended is noticed within 30 s of resuming, as with
/// Java's wall-clock `Timer` (tokio's timers are monotonic and stop during suspend on Linux).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl SystemClock {
    const MAX_SLICE: Duration = Duration::from_secs(30);
}

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn sleep_until(&self, deadline: SystemTime) -> BoxFuture<'static, ()> {
        async move {
            loop {
                match deadline.duration_since(SystemTime::now()) {
                    Ok(remaining) if !remaining.is_zero() => {
                        tokio::time::sleep(remaining.min(Self::MAX_SLICE)).await;
                    }
                    _ => return,
                }
            }
        }
        .boxed()
    }
}

/// A clock that moves only when told to, for tests of schedules.
#[derive(Debug)]
pub struct ManualClock {
    inner: Mutex<ManualInner>,
    sleepers_changed: Notify,
}

#[derive(Debug)]
struct ManualInner {
    now: SystemTime,
    sleepers: Vec<(SystemTime, oneshot::Sender<()>)>,
}

impl ManualClock {
    /// A clock standing at `start`.
    pub fn new(start: SystemTime) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ManualInner {
                now: start,
                sleepers: Vec::new(),
            }),
            sleepers_changed: Notify::new(),
        })
    }

    /// Moves the clock forward by `duration` and wakes every sleeper whose deadline is reached.
    pub fn advance(&self, duration: Duration) {
        let mut inner = self.inner.lock();
        let now = inner.now + duration;
        Self::move_to(&mut inner, now);
    }

    /// Sets the clock to `time` and wakes every sleeper whose deadline is reached.
    pub fn set(&self, time: SystemTime) {
        Self::move_to(&mut self.inner.lock(), time);
    }

    /// The number of sleeping futures still waiting (dropped ones are not counted).
    pub fn sleepers(&self) -> usize {
        let mut inner = self.inner.lock();
        inner.sleepers.retain(|(_, waker)| !waker.is_closed());
        inner.sleepers.len()
    }

    /// Resolves once at least `count` futures are sleeping, so a test advances the clock only
    /// after the code under test went to sleep.
    pub async fn wait_for_sleepers(&self, count: usize) {
        loop {
            let changed = self.sleepers_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.sleepers() >= count {
                return;
            }
            changed.await;
        }
    }

    fn move_to(inner: &mut ManualInner, now: SystemTime) {
        inner.now = now;
        let (due, waiting) = std::mem::take(&mut inner.sleepers)
            .into_iter()
            .partition::<Vec<_>, _>(|(deadline, _)| *deadline <= now);
        inner.sleepers = waiting;
        for (_, waker) in due {
            // The sleeper may have been dropped meanwhile.
            waker.send(()).ok();
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        self.inner.lock().now
    }

    fn sleep_until(&self, deadline: SystemTime) -> BoxFuture<'static, ()> {
        let receiver = {
            let mut inner = self.inner.lock();
            if deadline <= inner.now {
                return futures::future::ready(()).boxed();
            }
            let (sender, receiver) = oneshot::channel();
            inner.sleepers.push((deadline, sender));
            receiver
        };
        self.sleepers_changed.notify_waiters();
        async move {
            // A dropped sender means the clock is gone: nothing will ever wake this sleeper.
            if receiver.await.is_err() {
                futures::future::pending::<()>().await;
            }
        }
        .boxed()
    }
}
