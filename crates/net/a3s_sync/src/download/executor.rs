//! Run orchestration: task queue, workers, run-level policy and final state.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use a3s_transport::{CancellationToken, TransferCtx, Transport};
use parking_lot::Mutex;

use super::error::{DownloadError, FileError};
use super::file;
use super::monitor::{self, file_percent, remaining_time};
use super::sink::{DownloadedZip, ZipSink};
use super::{DownloadEnd, DownloadReport, DownloadRequest};
use crate::{
    DownloadStatus, DownloadTask, DownloadTaskKind, EventSender, Phase, RemoteError, SyncEvent,
    TransferProgress,
};

/// How long workers get to wind down after the caller cancelled, before they are aborted.
const CANCEL_GRACE: Duration = Duration::from_millis(600);

/// How long closing a transport may take.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);

/// How long the monitor may take to stop.
const MONITOR_STOP_TIMEOUT: Duration = Duration::from_millis(200);

/// Runs `tasks` and updates their statuses.
///
/// Never panics on bad input: every failure ends up in the report. Emits `Phase(Downloading)`,
/// `TransferTotal`, `Transfer`, `Speed`, `DiskUsage`, `ActiveConnections`, `ConnectionWaiting`,
/// `FileStarted`, `FileDone`, `FileFailed` and `ConnectionLost`, never `Finished`.
///
/// Cancelling `cancel` makes the call return [`DownloadEnd::Cancelled`] within a second; tasks
/// in flight keep their `Running` status and partial files, so a later call resumes them.
// Mirrors service/ConnectionService.java:332-345, service/connection/ConnectionDownloadProcessor.java:44-76,
// service/synchronization/FilesSynchronizationProcessor.java:188-194
pub async fn download(
    request: &DownloadRequest,
    tasks: &mut [DownloadTask],
    zips: Arc<dyn ZipSink>,
    cancel: &CancellationToken,
    events: &EventSender,
) -> DownloadReport {
    let started = Instant::now();
    let run = cancel.child_token();
    // Dropping this future cancels the run, which stops blocking zsync work.
    let _run_guard = run.clone().drop_guard();

    events.emit(SyncEvent::Phase(Phase::Downloading));
    // Sizes come from the remote tree: sums saturate instead of overflowing.
    let total = tasks
        .iter()
        .map(DownloadTask::download_size)
        .fold(0u64, u64::saturating_add);
    let files = tasks.iter().filter(|t| is_file(t)).count();
    events.emit(SyncEvent::TransferTotal {
        bytes: total,
        files: u32::try_from(files).unwrap_or(u32::MAX),
    });
    tracing::info!(
        "Downloading from repository: {} on url: {}",
        request.repository_name,
        a3s_types::local::protocol::host_url(&request.protocol)
    );

    let shared = Arc::new(Shared::new(
        request.clone(),
        tasks.to_vec(),
        zips,
        run.clone(),
        events.clone(),
        started,
        total,
    ));
    shared.emit_transfer();

    // Directories first, sequentially.
    for (index, task) in tasks.iter().enumerate() {
        if shared.should_stop() {
            break;
        }
        if matches!(task.kind, DownloadTaskKind::Directory) && task.status != DownloadStatus::Done {
            let end = file::create_directory(task).await;
            shared.finish_directory(index, end);
        }
    }

    // Pending files, taken last-first.
    let queue: Vec<usize> = (0..tasks.len())
        .filter(|&i| is_file(&tasks[i]) && tasks[i].status != DownloadStatus::Done)
        .collect();
    if !queue.is_empty() && !shared.should_stop() {
        let workers = (request.connections.max(1) as usize).min(queue.len());
        shared.start_workers(queue, workers);
        run_workers(&shared, workers).await;
    }

    shared.emit_transfer();
    events.emit(SyncEvent::ActiveConnections { active: 0 });
    shared.finish_report(tasks, cancel, started)
}

/// Runs the workers and the monitor until every worker ended, or the run was cancelled (by the
/// caller or by a stop) and the grace period expired.
async fn run_workers(shared: &Arc<Shared>, workers: usize) {
    let mut set = tokio::task::JoinSet::new();
    for id in 0..workers {
        set.spawn(worker(Arc::clone(shared), id));
    }
    let stop_monitor = CancellationToken::new();
    let mut monitor = AbortOnDrop(tokio::spawn(monitor::run(
        Arc::clone(shared),
        stop_monitor.clone(),
    )));

    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            biased;
            () = shared.run.cancelled(), if deadline.is_none() => {
                deadline = Some(tokio::time::Instant::now() + CANCEL_GRACE);
            }
            () = sleep_until(deadline), if deadline.is_some() => {
                tracing::warn!("download workers did not stop in time; aborting them");
                set.abort_all();
                break;
            }
            joined = set.join_next() => match joined {
                None => break,
                Some(Err(err)) if err.is_panic() => {
                    tracing::error!("download worker panicked: {err}");
                    shared.fail(DownloadError::Internal {
                        message: format!("A download worker failed unexpectedly: {err}"),
                    });
                }
                Some(_) => {}
            },
        }
    }
    drop(set);

    stop_monitor.cancel();
    if tokio::time::timeout(MONITOR_STOP_TIMEOUT, &mut monitor.0)
        .await
        .is_err()
    {
        monitor.0.abort();
    }
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Aborts a spawned task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn is_file(task: &DownloadTask) -> bool {
    matches!(task.kind, DownloadTaskKind::File { .. })
}

/// How a task ended.
#[derive(Debug)]
pub(crate) enum TaskEnd {
    /// Finalized; compressed leaves carry their `.zip`.
    Done(Option<DownloadedZip>),
    /// A file error.
    Failed(DownloadError),
    /// The connection is lost.
    ConnectionLost(DownloadError),
    /// Interrupted by a cancel or a stop of the run.
    Cancelled,
}

/// Why the run stopped before every task was attempted.
#[derive(Debug)]
enum Stop {
    TooManyErrors,
    ConnectionLost(DownloadError),
    Failed(DownloadError),
}

/// What a worker is transferring.
#[derive(Debug, Default)]
struct Slot {
    /// Index of the task.
    task: Option<usize>,
    /// Bytes on disk before this attempt.
    offset: u64,
    /// Bytes of this attempt.
    progress: Arc<AtomicU64>,
    /// Denominator of the file percentage.
    percent_base: u64,
    /// Download size of the task (cap of its contribution to `transferred`).
    cap: u64,
}

impl Slot {
    fn attempt_bytes(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default)]
struct State {
    queue: Vec<usize>,
    statuses: Vec<DownloadStatus>,
    errors: Vec<FileError>,
    zips: Vec<DownloadedZip>,
    files_done: u32,
    /// Σ download size of `Done` file tasks.
    done_size: u64,
    /// Payload bytes of finished attempts.
    finished_bytes: u64,
    slots: Vec<Slot>,
    reporter: Option<usize>,
    active: u32,
    max_active: u32,
    connecting: usize,
    connected: usize,
    /// The first connection failure of a worker.
    first_connect_error: Option<DownloadError>,
    stop: Option<Stop>,
}

/// Something to do once the state lock is released: events and zip hand-offs never run under
/// the lock.
#[derive(Debug)]
enum Effect {
    Emit(SyncEvent),
    Submit(DownloadedZip),
}

/// Effects collected under the lock.
#[derive(Debug, Default)]
struct Effects(Vec<Effect>);

impl Effects {
    fn emit(&mut self, event: SyncEvent) {
        self.0.push(Effect::Emit(event));
    }

    fn submit(&mut self, zip: DownloadedZip) {
        self.0.push(Effect::Submit(zip));
    }
}

/// State shared by the workers, the monitor and [`download`].
pub(crate) struct Shared {
    pub(crate) request: DownloadRequest,
    pub(crate) tasks: Vec<DownloadTask>,
    pub(crate) zips: Arc<dyn ZipSink>,
    /// Child of the caller's token; cancelled on stop too.
    pub(crate) run: CancellationToken,
    pub(crate) events: EventSender,
    pub(crate) started: Instant,
    pub(crate) total: u64,
    pub(crate) average_speed: AtomicU64,
    state: Mutex<State>,
}

/// Snapshot of the counters used by the monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) progress: TransferProgress,
    pub(crate) run_bytes: u64,
    pub(crate) active: u32,
}

impl Shared {
    fn new(
        request: DownloadRequest,
        tasks: Vec<DownloadTask>,
        zips: Arc<dyn ZipSink>,
        run: CancellationToken,
        events: EventSender,
        started: Instant,
        total: u64,
    ) -> Self {
        let done_size = crate::plan::resumed_size(&tasks);
        let state = State {
            statuses: tasks.iter().map(|t| t.status).collect(),
            done_size,
            ..Default::default()
        };
        Self {
            request,
            tasks,
            zips,
            run,
            events,
            started,
            total,
            average_speed: AtomicU64::new(0),
            state: Mutex::new(state),
        }
    }

    /// Whether no new task may start.
    fn should_stop(&self) -> bool {
        self.run.is_cancelled() || self.state.lock().stop.is_some()
    }

    fn start_workers(&self, mut queue: Vec<usize>, workers: usize) {
        let mut state = self.state.lock();
        // `pop` takes the last task first, like Java's `Stack`.
        queue.shrink_to_fit();
        state.queue = queue;
        state.slots = (0..workers).map(|_| Slot::default()).collect();
        state.connecting = workers;
    }

    /// The current status of task `index`.
    pub(crate) fn status(&self, index: usize) -> DownloadStatus {
        self.state.lock().statuses[index]
    }

    /// Marks task `index` as `Running`.
    pub(crate) fn set_running(&self, index: usize) {
        self.state.lock().statuses[index] = DownloadStatus::Running;
    }

    /// Starts an attempt of worker `id`: `offset` bytes are already on disk and the file
    /// percentage is relative to `percent_base`. Returns the attempt's progress counter.
    pub(crate) fn begin_attempt(
        &self,
        id: usize,
        offset: u64,
        percent_base: u64,
    ) -> Arc<AtomicU64> {
        let mut state = self.state.lock();
        let slot = &mut state.slots[id];
        slot.offset = offset;
        slot.percent_base = percent_base;
        slot.progress = Arc::new(AtomicU64::new(0));
        Arc::clone(&slot.progress)
    }

    /// Ends the current attempt of worker `id`; returns the bytes it received.
    pub(crate) fn end_attempt(&self, id: usize) -> u64 {
        let mut state = self.state.lock();
        let received = state.slots[id].attempt_bytes();
        state.slots[id].progress = Arc::new(AtomicU64::new(0));
        state.finished_bytes = state.finished_bytes.saturating_add(received);
        received
    }

    /// Corrects the offset of worker `id`'s attempt when the transport restarted from 0.
    pub(crate) fn set_attempt_offset(&self, id: usize, offset: u64) {
        self.state.lock().slots[id].offset = offset;
    }

    /// Runs effects collected under the lock.
    fn apply(&self, effects: Effects) {
        for effect in effects.0 {
            match effect {
                Effect::Emit(event) => self.events.emit(event),
                Effect::Submit(zip) => self.zips.submit(zip),
            }
        }
    }

    fn connected(&self) {
        let mut state = self.state.lock();
        state.connecting -= 1;
        state.connected += 1;
    }

    /// A worker could not connect (`None`: cancelled). When no worker connected, the run
    /// ends with the first failure.
    fn connect_failed(&self, error: Option<DownloadError>) {
        let mut effects = Effects::default();
        {
            let mut state = self.state.lock();
            state.connecting -= 1;
            if let Some(error) = error {
                tracing::warn!("Download connection failed: {error}");
                if state.first_connect_error.is_none() {
                    state.first_connect_error = Some(error);
                }
            }
            let all_failed = state.connecting == 0 && state.connected == 0;
            if all_failed
                && state.stop.is_none()
                && !self.run.is_cancelled()
                && let Some(error) = state.first_connect_error.take()
            {
                if error.is_connection_lost() {
                    state.stop = Some(Stop::ConnectionLost(error));
                    effects.emit(SyncEvent::ConnectionLost);
                } else {
                    state.stop = Some(Stop::Failed(error));
                }
                self.run.cancel();
            }
        }
        self.apply(effects);
    }

    /// Ends the run with `error` (a worker failed unexpectedly).
    fn fail(&self, error: DownloadError) {
        let mut state = self.state.lock();
        if state.stop.is_none() && !self.run.is_cancelled() {
            state.stop = Some(Stop::Failed(error));
        }
        self.run.cancel();
    }

    /// Takes the next task for worker `id`, marks the worker active and takes the
    /// reporter slot if it is free.
    fn take_next(&self, id: usize) -> Option<usize> {
        let mut effects = Effects::default();
        let index = {
            let mut state = self.state.lock();
            if state.stop.is_some() || self.run.is_cancelled() {
                return None;
            }
            let index = state.queue.pop()?;
            let task = &self.tasks[index];
            let slot = &mut state.slots[id];
            slot.task = Some(index);
            slot.offset = 0;
            slot.percent_base = 0;
            slot.cap = task.download_size();
            slot.progress = Arc::new(AtomicU64::new(0));
            state.active += 1;
            state.max_active = state.max_active.max(state.active);
            if state.reporter.is_none() {
                state.reporter = Some(id);
            }
            effects.emit(SyncEvent::ActiveConnections {
                active: state.active,
            });
            effects.emit(SyncEvent::FileStarted {
                relative_path: task.relative_path.clone(),
            });
            index
        };
        self.apply(effects);
        Some(index)
    }

    /// Records the end of a task taken by worker `id`.
    // Mirrors service/connection/ConnectionDownloadProcessor.java:89-126
    fn finish_task(&self, id: usize, index: usize, end: TaskEnd) {
        let mut effects = Effects::default();
        {
            let mut state = self.state.lock();
            let slot = &mut state.slots[id];
            slot.task = None;
            slot.offset = 0;
            state.active -= 1;
            effects.emit(SyncEvent::ActiveConnections {
                active: state.active,
            });
            let failed = !matches!(end, TaskEnd::Done(_));
            self.record(&mut state, &mut effects, index, end);
            if failed && state.reporter == Some(id) {
                release_reporter(&mut state);
            }
        }
        self.apply(effects);
    }

    /// Records the end of a directory task.
    fn finish_directory(&self, index: usize, end: TaskEnd) {
        let mut effects = Effects::default();
        effects.emit(SyncEvent::FileStarted {
            relative_path: self.tasks[index].relative_path.clone(),
        });
        {
            let mut state = self.state.lock();
            self.record(&mut state, &mut effects, index, end);
        }
        self.apply(effects);
    }

    fn record(&self, state: &mut State, effects: &mut Effects, index: usize, end: TaskEnd) {
        let task = &self.tasks[index];
        match end {
            TaskEnd::Done(zip) => {
                state.statuses[index] = DownloadStatus::Done;
                state.files_done += 1;
                if is_file(task) {
                    state.done_size = state.done_size.saturating_add(task.download_size());
                }
                if let Some(zip) = zip {
                    effects.submit(zip.clone());
                    state.zips.push(zip);
                }
                effects.emit(SyncEvent::FileDone {
                    relative_path: task.relative_path.clone(),
                });
            }
            TaskEnd::Failed(error) => {
                // Errors raised after a stop or a cancel are not recorded.
                if state.stop.is_some() || self.run.is_cancelled() {
                    tracing::debug!("Ignored after stop: {}: {error}", task.relative_path);
                    return;
                }
                let error = FileError {
                    relative_path: task.relative_path.clone(),
                    error,
                };
                tracing::warn!("Download failed: {}: {}", error.relative_path, error.error);
                effects.emit(SyncEvent::FileFailed {
                    relative_path: error.relative_path.clone(),
                    error: error.to_event_error(),
                });
                state.errors.push(error);
                // Mirrors service/connection/ConnectionDownloadProcessor.java:225-227
                if state.errors.len() >= self.request.max_errors.max(1) {
                    state.stop = Some(Stop::TooManyErrors);
                    self.run.cancel();
                }
            }
            TaskEnd::ConnectionLost(error) => {
                if state.stop.is_some() || self.run.is_cancelled() {
                    return;
                }
                tracing::warn!("Connection lost: {error}");
                state.stop = Some(Stop::ConnectionLost(error));
                effects.emit(SyncEvent::ConnectionLost);
                self.run.cancel();
            }
            TaskEnd::Cancelled => {}
        }
    }

    fn worker_exit(&self, id: usize) {
        let mut state = self.state.lock();
        if state.reporter == Some(id) {
            release_reporter(&mut state);
        }
    }

    /// The current counters.
    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        let mut transferred = state.done_size;
        let mut run_bytes = state.finished_bytes;
        for slot in &state.slots {
            let bytes = slot.attempt_bytes();
            run_bytes = run_bytes.saturating_add(bytes);
            if slot.task.is_some() {
                transferred =
                    transferred.saturating_add(slot.offset.saturating_add(bytes).min(slot.cap));
            }
        }
        let current = state
            .reporter
            .and_then(|id| state.slots.get(id))
            .and_then(|slot| slot.task.map(|index| (index, slot)));
        let (current_file, current_file_percent) = match current {
            Some((index, slot)) => (
                Some(self.tasks[index].relative_path.clone()),
                file_percent(
                    slot.offset.saturating_add(slot.attempt_bytes()),
                    slot.percent_base,
                ),
            ),
            None => (None, 0),
        };
        let remaining = remaining_time(self.total, transferred, run_bytes, self.started.elapsed());
        Snapshot {
            progress: TransferProgress {
                transferred,
                total: self.total,
                current_file,
                current_file_percent,
                remaining,
            },
            run_bytes,
            active: state.active,
        }
    }

    /// Emits a `Transfer` event with the current counters.
    pub(crate) fn emit_transfer(&self) {
        self.events
            .emit(SyncEvent::Transfer(self.snapshot().progress));
    }

    /// Builds the report and writes the statuses back into `tasks`.
    fn finish_report(
        &self,
        tasks: &mut [DownloadTask],
        cancel: &CancellationToken,
        started: Instant,
    ) -> DownloadReport {
        let snapshot = self.snapshot();
        let mut state = self.state.lock();
        for (task, status) in tasks.iter_mut().zip(&state.statuses) {
            task.status = *status;
        }
        let end = if cancel.is_cancelled() {
            DownloadEnd::Cancelled
        } else {
            match state.stop.take() {
                Some(Stop::ConnectionLost(error)) => DownloadEnd::ConnectionLost { error },
                Some(Stop::Failed(error)) => DownloadEnd::Failed { error },
                Some(Stop::TooManyErrors) => DownloadEnd::TooManyErrors {
                    limit: self.request.max_errors.max(1),
                },
                None => DownloadEnd::Completed,
            }
        };
        DownloadReport {
            end,
            errors: std::mem::take(&mut state.errors),
            downloaded_zips: std::mem::take(&mut state.zips),
            files_done: state.files_done,
            bytes_received: snapshot.run_bytes,
            average_speed: self.average_speed.load(Ordering::Relaxed),
            max_active_connections: state.max_active,
            elapsed: started.elapsed(),
        }
    }
}

/// Gives the reporter slot to the lowest-numbered active worker, if any.
// Mirrors service/connection/ConnectionDownloadProcessor.java:182-188
fn release_reporter(state: &mut State) {
    state.reporter = state.slots.iter().position(|slot| slot.task.is_some());
}

/// One download connection.
pub(crate) struct Worker {
    pub(crate) shared: Arc<Shared>,
    pub(crate) id: usize,
    pub(crate) transport: Option<Transport>,
    /// Files this worker's current transport completed.
    pub(crate) completed: u32,
}

impl Worker {
    /// Context of a payload transfer: run token, fresh progress counter, shared throttle.
    pub(crate) fn transfer_ctx(&self, progress: Arc<AtomicU64>) -> TransferCtx {
        TransferCtx {
            cancel: self.shared.run.clone(),
            progress,
            throttle: Some(Arc::clone(&self.shared.request.throttle)),
        }
    }

    /// Whether the transport already completed a file .
    pub(crate) fn reused(&self) -> bool {
        self.completed > 0
    }

    /// Opens a new connection, raced against the run token.
    async fn connect(&self) -> Result<Transport, Option<DownloadError>> {
        let request = &self.shared.request;
        let connect = Transport::connect(&request.protocol, &request.transport_options);
        let result = tokio::select! {
            biased;
            () = self.shared.run.cancelled() => return Err(None),
            result = connect => result,
        };
        result.map_err(|err| match RemoteError::connect(&request.protocol, err) {
            RemoteError::Cancelled => None,
            other => Some(DownloadError::Remote(other)),
        })
    }

    /// Replaces the transport with a new connection.
    pub(crate) async fn reconnect(&mut self) -> Result<(), TaskEnd> {
        if let Some(old) = self.transport.take() {
            close(old).await;
        }
        match self.connect().await {
            Ok(transport) => {
                self.transport = Some(transport);
                self.completed = 0;
                Ok(())
            }
            Err(Some(error)) => Err(TaskEnd::ConnectionLost(error)),
            Err(None) => Err(TaskEnd::Cancelled),
        }
    }
}

async fn close(transport: Transport) {
    if tokio::time::timeout(CLOSE_TIMEOUT, transport.close())
        .await
        .is_err()
    {
        tracing::debug!("closing a download connection timed out");
    }
}

/// The worker loop (Java `ConnectionDownloadProcessor.download`).
// Mirrors service/connection/ConnectionDownloadProcessor.java:78-137
async fn worker(shared: Arc<Shared>, id: usize) {
    let mut worker = Worker {
        shared: Arc::clone(&shared),
        id,
        transport: None,
        completed: 0,
    };
    match worker.connect().await {
        Ok(transport) => {
            shared.connected();
            worker.transport = Some(transport);
        }
        Err(error) => {
            shared.connect_failed(error);
            return;
        }
    }

    while let Some(index) = shared.take_next(id) {
        let end = file::run_task(&mut worker, index).await;
        if matches!(end, TaskEnd::Done(_)) {
            worker.completed += 1;
        }
        shared.finish_task(id, index, end);
        if worker.transport.is_none() {
            break;
        }
    }

    shared.worker_exit(id);
    if let Some(transport) = worker.transport.take() {
        close(transport).await;
    }
}
