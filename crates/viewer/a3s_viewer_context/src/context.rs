use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use a3s_repo_files::services::{ClientData, ServiceError};
use a3s_types::constants::CheckRepositoriesFrequency;
use a3s_types::local::Preferences;
use parking_lot::{Mutex, RwLockReadGuard};
use tokio::runtime::{Handle, Runtime};

use crate::checker::{NetworkChecker, RemoteChecker};
use crate::clock::{Clock, SystemClock};
use crate::command::{Command, dispatch};
use crate::error::ContextError;
use crate::event::{AppEvent, Change, EventBus, RepaintHook};
use crate::state::{AppFlags, AppState};
use crate::sync::{NetworkSynchronizer, Synchronizer};
use crate::tasks::{Cx, SharedParts, TaskId, TaskInfo, TaskKey, TaskSnapshot};

/// How a [`ViewerContext`] is built.
pub struct ContextOptions {
    /// Worker threads of the runtime the context builds.
    pub worker_threads: usize,

    /// An existing runtime to run on; the context then owns none.
    pub runtime: Option<Handle>,

    /// Fetches repository metadata.
    pub checker: Arc<dyn RemoteChecker>,

    /// Runs automatic updates in the background; `None` only emits
    /// [`crate::sync::Event::AutoUpdateRequested`].
    pub synchronizer: Option<Arc<dyn Synchronizer>>,

    /// Time for the periodic check.
    pub clock: Arc<dyn Clock>,

    /// Called after every event.
    pub repaint: RepaintHook,

    /// Development mode.
    pub dev_mode: bool,

    /// Started with the operating system.
    pub run_mode: bool,

    /// How long [`ViewerContext::prepare_exit`] waits for tasks, and the runtime shutdown
    /// waits for its threads.
    pub shutdown_timeout: Duration,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            worker_threads: 4,
            runtime: None,
            checker: Arc::new(NetworkChecker),
            synchronizer: Some(Arc::new(NetworkSynchronizer)),
            clock: Arc::new(SystemClock),
            repaint: RepaintHook::noop(),
            dev_mode: false,
            run_mode: false,
            shutdown_timeout: Duration::from_secs(2),
        }
    }
}

/// The one object the UI holds: state, commands, events, tasks and the periodic check.
///
/// Drop it on a plain thread (the UI thread), never inside an async context: dropping an owned
/// tokio runtime there panics.
// Mirrors ui/Facade.java
pub struct ViewerContext {
    runtime: Option<Runtime>,
    cx: Cx,
    events: Mutex<mpsc::Receiver<AppEvent>>,
    shutdown_timeout: Duration,
}

impl ViewerContext {
    /// A context over `client`, which the caller loaded (and whose warnings it showed). Starts
    /// the command dispatcher only; the periodic check starts with
    /// [`Self::start_background`].
    pub fn new(client: ClientData, options: ContextOptions) -> Result<Self, ContextError> {
        let (runtime, handle) = if let Some(handle) = options.runtime {
            (None, handle)
        } else {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(options.worker_threads.max(1))
                .thread_name("a3s-worker")
                .enable_all()
                .build()
                .map_err(ContextError::Runtime)?;
            let handle = runtime.handle().clone();
            (Some(runtime), handle)
        };
        let (events, receiver) = EventBus::new(options.repaint);
        let (commands, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let flags = AppFlags {
            dev_mode: options.dev_mode,
            run_mode: options.run_mode,
        };
        let cx = Cx::new(SharedParts {
            state: AppState::new(client, flags),
            events,
            checker: options.checker,
            synchronizer: options.synchronizer,
            clock: options.clock,
            handle: handle.clone(),
            commands,
        });
        handle.spawn(dispatch(cx.clone(), command_receiver));
        Ok(Self {
            runtime,
            cx,
            events: Mutex::new(receiver),
            shutdown_timeout: options.shutdown_timeout,
        })
    }

    /// Reads the state (the UI, every frame).
    pub fn read(&self) -> RwLockReadGuard<'_, AppState> {
        self.cx.state().read()
    }

    /// Runs `f` under the write lock; when it succeeds and `change` is set, emits
    /// [`AppEvent::Changed`].
    pub fn mutate<R, E>(
        &self,
        change: Option<Change>,
        f: impl FnOnce(&mut AppState) -> Result<R, E>,
    ) -> Result<R, E> {
        let result = f(&mut self.cx.state().write());
        if result.is_ok()
            && let Some(change) = change
        {
            self.cx.emit(AppEvent::Changed(change));
        }
        result
    }

    /// Queues a command for the background; fails with [`ContextError::Closed`] after shutdown.
    pub fn send(&self, command: Command) -> Result<(), ContextError> {
        match &*self.cx.shared.commands.lock() {
            Some(commands) => commands
                .send(command)
                .map_err(|_closed| ContextError::Closed),
            None => Err(ContextError::Closed),
        }
    }

    /// Every queued event, in send order, without blocking (once per frame).
    pub fn poll_events(&self) -> Vec<AppEvent> {
        self.events.lock().try_iter().collect()
    }

    /// The next event, waiting at most `timeout` (tests and hosts without a frame loop).
    pub fn next_event_timeout(&self, timeout: Duration) -> Option<AppEvent> {
        self.events.lock().recv_timeout(timeout).ok()
    }

    /// Replaces the repaint hook, e.g. once the UI framework handed out its context.
    pub fn set_repaint_hook(&self, hook: RepaintHook) {
        self.cx.shared.events.set_repaint_hook(hook);
    }

    /// The running tasks, in id order.
    pub fn running_tasks(&self) -> Vec<TaskInfo> {
        self.cx.running_tasks()
    }

    /// The running task with an equal key, if any.
    pub fn is_running(&self, key: &TaskKey) -> Option<TaskId> {
        self.cx.is_running(key)
    }

    /// The progress snapshot of a running task.
    pub fn task_snapshot(&self, id: TaskId) -> Option<TaskSnapshot> {
        self.cx.task_snapshot(id)
    }

    /// Cancels a running task; `false` if it is unknown or finished.
    pub fn cancel_task(&self, id: TaskId) -> bool {
        self.cx.cancel_task(id)
    }

    /// Cancels every task of `repository`; returns how many were cancelled.
    pub fn cancel_repository_tasks(&self, repository: &str) -> usize {
        self.cx.cancel_repository_tasks(repository)
    }

    /// Cancels every running task; returns how many were cancelled.
    pub fn cancel_all(&self) -> usize {
        self.cx.cancel_all()
    }

    /// Starts the periodic repository check with the preferred frequency: one check at once,
    /// then one per period. A no-op while it runs.
    pub fn start_background(&self) {
        self.cx.start_background();
    }

    /// Stops the periodic check and cancels a running one. It may be started again.
    pub fn stop_background(&self) {
        self.cx.stop_background();
    }

    /// Reschedules the periodic check for a new frequency: the next check comes one period
    /// from now (never when disabled); a running check finishes normally.
    pub fn reschedule_checks(&self, frequency: CheckRepositoriesFrequency) {
        self.cx.reschedule_checks(frequency);
    }

    /// Cancels the repository's tasks, then removes it with its runtime state and notification
    /// memory; whether a file was deleted.
    pub fn remove_repository(&self, name: &str) -> Result<bool, ServiceError> {
        self.cx.cancel_repository_tasks(name);
        let removed = {
            let mut guard = self.cx.state().write();
            let state = &mut *guard;
            let removed = state.client.remove_repository(name)?;
            state.repositories.runtime.remove(name);
            state.repositories.tracker.forget(name);
            removed
        };
        self.cx.emit(AppEvent::Changed(Change::Repository));
        Ok(removed)
    }

    /// Renames a repository and moves its runtime state. Running tasks keep their key.
    pub fn rename_repository(&self, old: &str, new: &str) -> Result<(), ServiceError> {
        {
            let mut guard = self.cx.state().write();
            let state = &mut *guard;
            state.client.rename_repository(old, new)?;
            if let Some(runtime) = state.repositories.runtime.remove(old) {
                state.repositories.runtime.insert(new.to_owned(), runtime);
            }
            state.repositories.tracker.forget(old);
        }
        self.cx.emit(AppEvent::Changed(Change::Repository));
        Ok(())
    }

    /// Stores the preferences; reschedules the periodic check when its frequency changed.
    pub fn set_preferences(&self, preferences: Preferences) -> Result<(), ServiceError> {
        let frequency = preferences.check_repositories_frequency;
        let changed = {
            let mut state = self.cx.state().write();
            let previous = state.client.preferences().check_repositories_frequency;
            state.client.set_preferences(preferences)?;
            previous != frequency
        };
        if changed {
            self.cx.reschedule_checks(frequency);
        }
        Ok(())
    }

    /// The first exit step: stops the periodic check, cancels every task and waits up to the
    /// shutdown timeout for them to end. Returns the tasks still running (also logged).
    ///
    /// The UI then calls [`Self::save_on_exit`]; when that fails it asks (in a sheet) whether
    /// to exit anyway, and if the user stays, calls [`Self::start_background`] again. Then
    /// [`Self::shutdown`].
    // Mirrors ui/main/MainPanel.java:877-900
    pub fn prepare_exit(&self) -> Vec<TaskInfo> {
        self.cx.stop_background();
        self.cx.cancel_all();
        let deadline = Instant::now() + self.shutdown_timeout;
        loop {
            let running = self.cx.running_tasks();
            if running.is_empty() {
                return running;
            }
            if Instant::now() >= deadline {
                for task in &running {
                    tracing::warn!(
                        "Task still running at exit: {} (repository: {})",
                        task.key.kind.label(),
                        task.key.repository.as_deref().unwrap_or("-")
                    );
                }
                return running;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Stores the window size and writes the configuration, preferences and active profile.
    // Mirrors service/CommonService.java:41-52
    pub fn save_on_exit(&self, height: i32, width: i32) -> Result<(), ServiceError> {
        self.cx.state().write().client.save_on_exit(height, width)
    }

    /// Closes the command channel, refuses new tasks, cancels everything and shuts an owned
    /// runtime down (waiting up to the shutdown timeout). Idempotent; called on drop.
    pub fn shutdown(&mut self) {
        self.cx.close_registry();
        self.cx.shared.commands.lock().take();
        self.cx.stop_background();
        self.cx.shared.root.cancel();
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(self.shutdown_timeout);
        }
    }

    /// The context handle for domain code and tests.
    pub fn cx(&self) -> &Cx {
        &self.cx
    }

    /// The runtime background work runs on.
    pub fn handle(&self) -> &Handle {
        self.cx.handle()
    }
}

impl Drop for ViewerContext {
    fn drop(&mut self) {
        self.shutdown();
    }
}
