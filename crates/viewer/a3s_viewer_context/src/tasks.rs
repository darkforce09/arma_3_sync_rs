use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::SystemTime;

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_repo_files::services::ServiceError;
use a3s_sync::{EventError, EventSender, Phase, RemoteError, SyncEvent, TransferProgress};
use a3s_transport::{CancellationToken, TransferCtx};
use parking_lot::{Mutex, RwLock};
use tokio::runtime::Handle;
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::JoinHandle;

use crate::background::Scheduler;
use crate::checker::RemoteChecker;
use crate::clock::Clock;
use crate::command::Command;
use crate::error::ContextError;
use crate::event::{AppEvent, EventBus, TaskEvent};
use crate::state::AppState;
use crate::sync::Synchronizer;

/// Identifies a task; ids increase from 1 and are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(pub u64);

/// Every kind of background operation of the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TaskKind {
    /// The periodic check of every repository.
    PeriodicCheck,
    /// A check of repositories asked for by the user.
    CheckRepositories,
    /// A comparison of a repository with its local folder, before a download.
    Compare,
    /// A synchronization started by the user.
    Download,
    /// An automatic synchronization after a periodic check.
    AutoUpdate,
    /// A repository build.
    Build,
    /// A content check of a built repository.
    AdminCheck,
    /// A repository upload.
    Upload,
    /// An upload of the events file.
    UploadEvents,
    /// An autoconfig import from a URL.
    ImportAutoConfig,
    /// A probe of partial transfer support.
    PartialTransferTest,
    /// A scan of the addon folders.
    AddonScan,
    /// A game launch.
    Launch,
    /// A bikey extraction.
    ExtractBikeys,
    /// Following the game's RPT log.
    RptTail,
    /// A TFAR or ACRE 2 plugin installation.
    InstallRadioPlugin,
    /// A check for a new version of the application.
    SelfUpdateCheck,
}

impl TaskKind {
    /// A short text for logs and the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::PeriodicCheck | Self::CheckRepositories => "Checking repositories",
            Self::Compare => "Comparing local files",
            Self::Download => "Synchronizing",
            Self::AutoUpdate => "Updating automatically",
            Self::Build => "Building the repository",
            Self::AdminCheck => "Checking the repository content",
            Self::Upload => "Uploading the repository",
            Self::UploadEvents => "Uploading the events",
            Self::ImportAutoConfig => "Importing the autoconfig",
            Self::PartialTransferTest => "Testing partial file transfer",
            Self::AddonScan => "Scanning addons",
            Self::Launch => "Launching the game",
            Self::ExtractBikeys => "Extracting keys",
            Self::RptTail => "Following the RPT log",
            Self::InstallRadioPlugin => "Installing the radio plugin",
            Self::SelfUpdateCheck => "Checking for application updates",
        }
    }
}

/// What a task works on; exclusive tasks with equal keys never run at the same time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TaskKey {
    /// The operation.
    pub kind: TaskKind,

    /// The repository it works on, if any.
    pub repository: Option<String>,
}

impl TaskKey {
    /// A key without a repository.
    pub fn new(kind: TaskKind) -> Self {
        Self {
            kind,
            repository: None,
        }
    }

    /// A key for one repository.
    pub fn repository(kind: TaskKind, repository: impl Into<String>) -> Self {
        Self {
            kind,
            repository: Some(repository.into()),
        }
    }
}

/// A registered task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskInfo {
    /// Its id.
    pub id: TaskId,

    /// Its key.
    pub key: TaskKey,

    /// When it started (the context clock).
    pub started: SystemTime,
}

/// How a task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskOutcome {
    /// It returned `Ok`.
    Completed,

    /// It was cancelled, or returned [`TaskError::Cancelled`].
    Cancelled,

    /// It returned an error.
    Failed(EventError),

    /// It panicked; the panic message.
    Panicked(String),
}

/// The last value of each high-rate progress event of a running task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskSnapshot {
    /// The current phase.
    pub phase: Option<Phase>,

    /// Percentage of a phase.
    pub progress: Option<(Phase, u8)>,

    /// Bytes and files to transfer.
    pub total: Option<(u64, u32)>,

    /// Transfer progress.
    pub transfer: Option<TransferProgress>,

    /// Throughput in bytes per second.
    pub speed: Option<u64>,

    /// Local write throughput in bytes per second.
    pub disk_usage: Option<u64>,

    /// Connections transferring.
    pub active_connections: Option<u32>,

    /// Seconds without throughput.
    pub waiting_seconds: Option<u32>,
}

impl TaskSnapshot {
    /// Takes in a coalesced event; returns `false` for an event that must be queued instead.
    fn absorb(&mut self, event: &SyncEvent) -> bool {
        match event {
            SyncEvent::Progress { phase, percent } => self.progress = Some((*phase, *percent)),
            SyncEvent::TransferTotal { bytes, files } => self.total = Some((*bytes, *files)),
            SyncEvent::Transfer(progress) => self.transfer = Some(progress.clone()),
            SyncEvent::Speed { bytes_per_second } => self.speed = Some(*bytes_per_second),
            SyncEvent::DiskUsage { bytes_per_second } => {
                self.disk_usage = Some(*bytes_per_second);
            }
            SyncEvent::ActiveConnections { active } => self.active_connections = Some(*active),
            SyncEvent::ConnectionWaiting { seconds } => self.waiting_seconds = Some(*seconds),
            SyncEvent::Phase(phase) => {
                // Queued as well: phases are rare and the UI reacts to each one.
                self.phase = Some(*phase);
                return false;
            }
            _ => return false,
        }
        true
    }
}

/// Why a task failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TaskError {
    /// The task stopped because it was cancelled.
    #[error("Cancelled")]
    Cancelled,

    /// A remote operation failed.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// A client data operation failed.
    #[error(transparent)]
    Service(#[from] ServiceError),

    /// Any other failure, already projected for the UI.
    #[error("{0}")]
    Message(EventError),
}

impl TaskError {
    /// The projection carried by [`TaskOutcome::Failed`].
    pub fn to_event_error(&self) -> EventError {
        match self {
            Self::Remote(err) => EventError::from_remote(err),
            Self::Message(err) => err.clone(),
            other => EventError::new(other.kind(), other.user_message()),
        }
    }
}

impl UserFacing for TaskError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Cancelled => ErrorKind::Cancelled,
            Self::Remote(err) => err.kind(),
            Self::Service(err) => err.kind(),
            Self::Message(err) => err.kind,
        }
    }

    fn user_message(&self) -> String {
        match self {
            Self::Service(err) => err.user_message(),
            other => other.to_string(),
        }
    }
}

/// A handle to the context for background code and command handlers. Cloning is cheap.
#[derive(Clone)]
pub struct Cx {
    pub(crate) shared: Arc<Shared>,
}

/// Everything the background parts share.
pub(crate) struct Shared {
    pub(crate) state: Arc<RwLock<AppState>>,
    pub(crate) events: EventBus,
    registry: Mutex<Registry>,

    /// Parent of every task token; cancelled only by shutdown.
    pub(crate) root: CancellationToken,
    pub(crate) checker: Arc<dyn RemoteChecker>,
    pub(crate) synchronizer: Option<Arc<dyn Synchronizer>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) handle: Handle,

    /// At most this many repository checks run at once in the whole process.
    pub(crate) check_permits: Semaphore,

    /// One lock per repository, so a repository is never checked twice at once.
    repository_locks: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub(crate) scheduler: Scheduler,

    /// `None` once shutdown started.
    pub(crate) commands: Mutex<Option<mpsc::UnboundedSender<Command>>>,
}

pub(crate) struct SharedParts {
    pub(crate) state: AppState,
    pub(crate) events: EventBus,
    pub(crate) checker: Arc<dyn RemoteChecker>,
    pub(crate) synchronizer: Option<Arc<dyn Synchronizer>>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) handle: Handle,
    pub(crate) commands: mpsc::UnboundedSender<Command>,
}

#[derive(Default)]
struct Registry {
    last_id: u64,
    closed: bool,
    tasks: BTreeMap<TaskId, TaskEntry>,
}

struct TaskEntry {
    info: TaskInfo,
    cancel: CancellationToken,
    snapshot: TaskSnapshot,

    /// Closed when the task has finished.
    done: watch::Receiver<()>,
}

impl Cx {
    pub(crate) fn new(parts: SharedParts) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Arc::new(RwLock::new(parts.state)),
                events: parts.events,
                registry: Mutex::new(Registry::default()),
                root: CancellationToken::new(),
                checker: parts.checker,
                synchronizer: parts.synchronizer,
                clock: parts.clock,
                handle: parts.handle,
                check_permits: Semaphore::new(a3s_sync::remote::MAX_CONCURRENT_REPOSITORY_CHECKS),
                repository_locks: Mutex::new(BTreeMap::new()),
                scheduler: Scheduler::default(),
                commands: Mutex::new(Some(parts.commands)),
            }),
        }
    }

    /// The shared state. Never hold a guard across `.await`, and never do network or recursive
    /// file I/O under the lock; the small writes of the client data services are fine.
    pub fn state(&self) -> &Arc<RwLock<AppState>> {
        &self.shared.state
    }

    /// Queues `event` for the UI and requests a repaint.
    pub fn emit(&self, event: AppEvent) {
        self.shared.events.emit(event);
    }

    /// The context clock.
    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.shared.clock
    }

    /// The runtime background work runs on.
    pub fn handle(&self) -> &Handle {
        &self.shared.handle
    }

    /// Registers and starts a task running `f`.
    ///
    /// With `exclusive`, fails with [`ContextError::TaskBusy`] while a task with an equal key
    /// runs. Emits [`TaskEvent::Started`]; when the task ends it is unregistered, then
    /// [`TaskEvent::Finished`] is emitted. A panic in the task becomes
    /// [`TaskOutcome::Panicked`]; the context stays usable.
    pub fn spawn_task<F, Fut>(
        &self,
        key: TaskKey,
        exclusive: bool,
        f: F,
    ) -> Result<TaskId, ContextError>
    where
        F: FnOnce(TaskCtx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), TaskError>> + Send + 'static,
    {
        self.spawn_child(key, exclusive, &self.shared.root, f)
            .map(|(id, _)| id)
    }

    /// [`Self::spawn_task`] with a token that is a child of `parent`; also returns the handle
    /// of the supervisor, which ends after [`TaskEvent::Finished`] was emitted.
    pub(crate) fn spawn_child<F, Fut>(
        &self,
        key: TaskKey,
        exclusive: bool,
        parent: &CancellationToken,
        f: F,
    ) -> Result<(TaskId, JoinHandle<()>), ContextError>
    where
        F: FnOnce(TaskCtx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), TaskError>> + Send + 'static,
    {
        let (done_sender, done) = watch::channel(());
        let (info, cancel) = {
            let mut registry = self.shared.registry.lock();
            if registry.closed {
                return Err(ContextError::Closed);
            }
            if exclusive && let Some(running) = registry.tasks.values().find(|t| t.info.key == key)
            {
                return Err(ContextError::TaskBusy {
                    id: running.info.id,
                    key,
                });
            }
            registry.last_id += 1;
            let info = TaskInfo {
                id: TaskId(registry.last_id),
                key,
                started: self.shared.clock.now(),
            };
            let cancel = parent.child_token();
            registry.tasks.insert(
                info.id,
                TaskEntry {
                    info: info.clone(),
                    cancel: cancel.clone(),
                    snapshot: TaskSnapshot::default(),
                    done,
                },
            );
            (info, cancel)
        };
        self.emit(AppEvent::Task(TaskEvent::Started(info.clone())));

        let id = info.id;
        let task = TaskCtx {
            id,
            cancel: cancel.clone(),
            cx: self.clone(),
        };
        let cx = self.clone();
        let supervisor = self.shared.handle.spawn(async move {
            // `f` itself runs inside the inner task, so a panic while building the future is
            // caught too.
            let joined = tokio::spawn(async move { f(task).await }).await;
            let outcome = match joined {
                Ok(Ok(())) => TaskOutcome::Completed,
                Ok(Err(_)) if cancel.is_cancelled() => TaskOutcome::Cancelled,
                Ok(Err(TaskError::Cancelled)) => TaskOutcome::Cancelled,
                Ok(Err(err)) => TaskOutcome::Failed(err.to_event_error()),
                Err(join) if join.is_panic() => {
                    let message = panic_message(join.into_panic());
                    tracing::error!(
                        "Task {} ({}) panicked: {message}",
                        info.id.0,
                        info.key.kind.label()
                    );
                    TaskOutcome::Panicked(message)
                }
                // The runtime is shutting down.
                Err(_) => TaskOutcome::Cancelled,
            };
            cx.shared.registry.lock().tasks.remove(&info.id);
            cx.emit(AppEvent::Task(TaskEvent::Finished { info, outcome }));
            drop(done_sender);
        });
        Ok((id, supervisor))
    }

    /// Resolves once task `id` has finished (at once if it is not running).
    pub(crate) async fn wait_task(&self, id: TaskId) {
        let done = self
            .shared
            .registry
            .lock()
            .tasks
            .get(&id)
            .map(|t| t.done.clone());
        if let Some(mut done) = done {
            // Errs once the supervisor dropped the sender, i.e. the task finished.
            while done.changed().await.is_ok() {}
        }
    }

    /// The running tasks, in id order.
    pub fn running_tasks(&self) -> Vec<TaskInfo> {
        self.shared
            .registry
            .lock()
            .tasks
            .values()
            .map(|t| t.info.clone())
            .collect()
    }

    /// The running task with an equal key, if any.
    pub fn is_running(&self, key: &TaskKey) -> Option<TaskId> {
        self.shared
            .registry
            .lock()
            .tasks
            .values()
            .find(|t| t.info.key == *key)
            .map(|t| t.info.id)
    }

    /// The progress snapshot of a running task.
    pub fn task_snapshot(&self, id: TaskId) -> Option<TaskSnapshot> {
        self.shared
            .registry
            .lock()
            .tasks
            .get(&id)
            .map(|t| t.snapshot.clone())
    }

    /// Cancels a running task; `false` if it is unknown or already finished. The task stops on
    /// its own and reports through [`TaskEvent::Finished`].
    pub fn cancel_task(&self, id: TaskId) -> bool {
        let registry = self.shared.registry.lock();
        match registry.tasks.get(&id) {
            Some(task) if !task.cancel.is_cancelled() => {
                task.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// Cancels every running task keyed with `repository`; returns how many were cancelled.
    pub fn cancel_repository_tasks(&self, repository: &str) -> usize {
        self.cancel_where(|key| key.repository.as_deref() == Some(repository))
    }

    /// Cancels every running task. Tasks started afterwards run normally.
    pub fn cancel_all(&self) -> usize {
        self.cancel_where(|_| true)
    }

    fn cancel_where(&self, matches: impl Fn(&TaskKey) -> bool) -> usize {
        let registry = self.shared.registry.lock();
        let mut count = 0;
        for task in registry.tasks.values() {
            if matches(&task.info.key) && !task.cancel.is_cancelled() {
                task.cancel.cancel();
                count += 1;
            }
        }
        count
    }

    /// Stops accepting tasks: [`Self::spawn_task`] fails with [`ContextError::Closed`].
    pub(crate) fn close_registry(&self) {
        self.shared.registry.lock().closed = true;
    }

    /// The check lock of `repository`.
    pub(crate) fn repository_lock(&self, repository: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.shared
                .repository_locks
                .lock()
                .entry(repository.to_owned())
                .or_default(),
        )
    }

    /// Routes an event of task `id`: high-rate progress updates its snapshot, the rest is
    /// queued.
    fn task_event(&self, id: TaskId, event: SyncEvent) {
        let absorbed = self
            .shared
            .registry
            .lock()
            .tasks
            .get_mut(&id)
            .is_some_and(|t| t.snapshot.absorb(&event));
        if absorbed {
            self.shared.events.repaint();
        } else {
            self.emit(AppEvent::Task(TaskEvent::Sync { id, event }));
        }
    }
}

/// What a running task receives.
pub struct TaskCtx {
    /// Its id.
    pub id: TaskId,

    /// Cancelled when the task must stop.
    pub cancel: CancellationToken,

    /// The context.
    pub cx: Cx,
}

impl TaskCtx {
    /// A transfer context with this task's token, a fresh progress counter and no throttle.
    pub fn transfer_ctx(&self) -> TransferCtx {
        TransferCtx {
            cancel: self.cancel.clone(),
            ..TransferCtx::default()
        }
    }

    /// An event sender for this task: progress goes to its [`TaskSnapshot`], everything else
    /// is queued as [`TaskEvent::Sync`].
    pub fn sync_events(&self) -> EventSender {
        let cx = self.cx.clone();
        let id = self.id;
        EventSender::from_fn(move |event| cx.task_event(id, event))
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map_or_else(|| "task panicked".to_owned(), |s| (*s).to_owned()),
    }
}
