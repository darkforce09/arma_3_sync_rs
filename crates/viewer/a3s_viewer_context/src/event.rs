use std::fmt;
use std::sync::Arc;
use std::sync::mpsc;

use a3s_sync::{EventError, SyncEvent};
use parking_lot::RwLock;

use crate::tasks::{TaskId, TaskInfo, TaskOutcome};
use crate::{admin, launch, profiles, repositories, sync, tools};

/// What changed in the [`crate::AppState`], so the UI refreshes the views that show it. The
/// Java `UIConstants.OP_*` value is given for each variant.
// Mirrors ui/UIConstants.java:14-20
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Change {
    /// Profiles (`OP_PROFILE_CHANGED` = 1); the UI also rebuilds its profiles menu.
    Profile,

    /// Addon files on disk (`OP_ADDON_FILES_CHANGED` = 2).
    AddonFiles,

    /// Addon priorities (`OP_ADDON_PRIORITY_CHANGED` = 3).
    AddonPriority,

    /// Addon selection (`OP_ADDON_SELECTION_CHANGED` = 4).
    AddonSelection,

    /// Online settings (`OP_ONLINE_CHANGED` = 5).
    Online,

    /// Repositories, their status or their runtime state (`OP_REPOSITORY_CHANGED` = 6).
    Repository,

    /// Addon groups (`OP_GROUP_CHANGED` = 7).
    Group,
}

/// A notification from the background to the UI, drained with
/// [`crate::ViewerContext::poll_events`].
///
/// It replaces the Java `controller/Observer*` interfaces. Progress and outcome of an operation
/// travel as [`SyncEvent`]s inside [`TaskEvent::Sync`] (`ObserverText` is `Phase`,
/// `ObserverCountInt` is `Progress`, `ObserverCountLong`/`ObserverDownload`/`ObserverUpload` are
/// `TransferTotal`, `Transfer`, `Speed`, `DiskUsage`, `ActiveConnections`, `LastIndexTransferred`
/// and `Finished`, `ObserverConnectionLost` is `ConnectionLost`); the high-rate ones are
/// coalesced into the task's [`crate::TaskSnapshot`]. `ObserverStart` and `ObserverEnd` are
/// [`TaskEvent::Started`] and [`TaskEvent::Finished`], `ObserverError` is
/// [`TaskOutcome::Failed`] or `Outcome::CompletedWithErrors`. `ObserverProceed` is plain
/// sequencing (`await`) and has no event.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AppEvent {
    /// Part of the state changed.
    Changed(Change),

    /// A background task started, progressed or finished.
    Task(TaskEvent),

    /// A background failure that is not the outcome of a task, e.g. a failed save after a check
    /// or a rejected command.
    Error {
        /// What was being done.
        context: String,
        /// The error.
        error: EventError,
    },

    /// Repository checks.
    Repositories(repositories::Event),

    /// Synchronization.
    Sync(sync::Event),

    /// Repository administration.
    Admin(admin::Event),

    /// Profiles and addons.
    Profiles(profiles::Event),

    /// Game launch.
    Launch(launch::Event),

    /// Game tools.
    Tools(tools::Event),
}

/// The life of a background task.
#[derive(Clone, Debug)]
pub enum TaskEvent {
    /// The task was registered and starts.
    Started(TaskInfo),

    /// A queued (not coalesced) event of the task.
    Sync {
        /// The task.
        id: TaskId,
        /// The event.
        event: SyncEvent,
    },

    /// The task ended and is no longer listed as running.
    Finished {
        /// The task.
        info: TaskInfo,
        /// How it ended.
        outcome: TaskOutcome,
    },
}

/// Called after every queued event and every snapshot update, so the UI redraws and polls.
///
/// With egui: `RepaintHook::from_fn(move || ctx.request_repaint())`.
#[derive(Clone)]
pub struct RepaintHook(Option<Arc<dyn Fn() + Send + Sync>>);

impl RepaintHook {
    /// A hook that does nothing.
    pub fn noop() -> Self {
        Self(None)
    }

    /// A hook calling `f`, on whichever thread emitted the event; `f` must not block.
    pub fn from_fn(f: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(f)))
    }

    fn call(&self) {
        if let Some(f) = &self.0 {
            f();
        }
    }
}

impl Default for RepaintHook {
    fn default() -> Self {
        Self::noop()
    }
}

impl fmt::Debug for RepaintHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RepaintHook")
            .field(&if self.0.is_some() { "fn" } else { "noop" })
            .finish()
    }
}

/// The sending half of the event channel, shared by every background part.
pub(crate) struct EventBus {
    sender: mpsc::Sender<AppEvent>,
    repaint: RwLock<RepaintHook>,
}

impl EventBus {
    pub(crate) fn new(repaint: RepaintHook) -> (Self, mpsc::Receiver<AppEvent>) {
        let (sender, receiver) = mpsc::channel();
        let bus = Self {
            sender,
            repaint: RwLock::new(repaint),
        };
        (bus, receiver)
    }

    /// Queues `event` and requests a repaint. Never blocks; without a receiver the event is
    /// dropped.
    pub(crate) fn emit(&self, event: AppEvent) {
        // The receiver is gone only while the context is being dropped.
        self.sender.send(event).ok();
        self.repaint();
    }

    pub(crate) fn repaint(&self) {
        // Cloned out so a hook may replace itself without deadlocking.
        let hook = self.repaint.read().clone();
        hook.call();
    }

    pub(crate) fn set_repaint_hook(&self, hook: RepaintHook) {
        *self.repaint.write() = hook;
    }
}
