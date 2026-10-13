use a3s_sync::EventSender;
use a3s_sync::status::{CheckedRepository, decide_check_cycle};
use a3s_transport::CancellationToken;
use a3s_types::constants::CheckRepositoriesFrequency;
use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::checker::{CheckTrigger, apply_results, check_many};
use crate::error::ContextError;
use crate::event::AppEvent;
use crate::schedule::CheckSchedule;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskKey, TaskKind};
use crate::{repositories, sync};

/// The periodic repository check: its schedule and the loop that runs it.
#[derive(Debug, Default)]
pub(crate) struct Scheduler {
    state: Mutex<SchedulerState>,

    /// Wakes the loop after the schedule changed.
    wake: Notify,
}

#[derive(Debug, Default)]
struct SchedulerState {
    schedule: Option<CheckSchedule>,

    /// The token of the running loop; `None` when stopped.
    stop: Option<CancellationToken>,
}

impl Cx {
    /// Starts the periodic check loop with the preferred frequency: a check at once, then one
    /// per period. A no-op while the loop runs or after shutdown.
    // Mirrors ui/main/MainPanel.java:584-596, 1073-1087
    pub(crate) fn start_background(&self) {
        if self.shared.root.is_cancelled() {
            return;
        }
        let frequency = self
            .state()
            .read()
            .client
            .preferences()
            .check_repositories_frequency;
        let stop = {
            let mut scheduler = self.shared.scheduler.state.lock();
            if scheduler.stop.is_some() {
                return;
            }
            scheduler.schedule = Some(CheckSchedule::at_startup(frequency, self.clock().now()));
            let stop = self.shared.root.child_token();
            scheduler.stop = Some(stop.clone());
            stop
        };
        self.shared.handle.spawn(run_loop(self.clone(), stop));
    }

    /// Stops the loop and cancels a running periodic check.
    // Mirrors ui/main/tasks/TasksManager.java:30-33
    pub(crate) fn stop_background(&self) {
        if let Some(stop) = self.shared.scheduler.state.lock().stop.take() {
            stop.cancel();
        }
    }

    /// Replaces the schedule after a frequency change: the next check comes one period from
    /// now (never when disabled). A running check finishes normally.
    // Mirrors ui/help/PreferencesDialog.java:418-427
    pub(crate) fn reschedule_checks(&self, frequency: CheckRepositoriesFrequency) {
        self.shared.scheduler.state.lock().schedule =
            Some(CheckSchedule::after_change(frequency, self.clock().now()));
        self.shared.scheduler.wake.notify_one();
    }
}

async fn run_loop(cx: Cx, stop: CancellationToken) {
    let scheduler = &cx.shared.scheduler;
    loop {
        let due = scheduler
            .state
            .lock()
            .schedule
            .and_then(|schedule| schedule.next_due());
        let wake = scheduler.wake.notified();
        match due {
            None => tokio::select! {
                biased;
                () = stop.cancelled() => return,
                () = wake => continue,
            },
            Some(due) => {
                let sleep = cx.clock().sleep_until(due);
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return,
                    () = wake => continue,
                    () = sleep => {}
                }
            }
        }
        if let Some(schedule) = scheduler.state.lock().schedule.as_mut() {
            schedule.on_cycle_started(cx.clock().now());
        }
        run_cycle(&cx, &stop).await;
    }
}

/// Starts one periodic check and waits for its end (cycles never overlap).
async fn run_cycle(cx: &Cx, stop: &CancellationToken) {
    loop {
        let key = TaskKey::new(TaskKind::PeriodicCheck);
        match cx.spawn_child(key, true, stop, periodic_cycle) {
            Ok((_, supervisor)) => {
                tokio::select! {
                    biased;
                    () = stop.cancelled() => {}
                    _ = supervisor => {}
                }
                return;
            }
            // The cycle of a stopped loop is still finishing.
            Err(ContextError::TaskBusy { id, .. }) => tokio::select! {
                biased;
                () = stop.cancelled() => return,
                () = cx.wait_task(id) => {}
            },
            Err(_closed) => return,
        }
    }
}

/// One periodic check of every repository: check, apply, then auto-update and notify.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:38-171
async fn periodic_cycle(task: TaskCtx) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    tracing::info!("Checking repositories...");
    let (targets, mut results) = check_many(
        &cx,
        None,
        CheckTrigger::Periodic,
        &task,
        &EventSender::noop(),
    )
    .await;
    if task.cancel.is_cancelled() {
        tracing::info!("Checking repositories has been anormaly interrupted.");
        return Err(TaskError::Cancelled);
    }
    tracing::info!("Checking repositories done.");
    apply_results(&cx, CheckTrigger::Periodic, &mut results);

    let decision = {
        let mut guard = cx.state().write();
        let state = &mut *guard;
        let checked: Vec<CheckedRepository> = targets
            .iter()
            .zip(&results)
            .filter_map(|(target, result)| {
                Some(CheckedRepository {
                    name: target.name.clone(),
                    notify: target.notify,
                    auto: target.auto,
                    status: result.status?,
                    server_revision: state
                        .repositories
                        .runtime
                        .get(&target.name)
                        .and_then(|runtime| runtime.metadata.server_revision()),
                })
            })
            .collect();
        decide_check_cycle(&checked, &mut state.repositories.tracker)
    };
    if let Some(message) = decision.console_message() {
        tracing::info!("{message}");
    }
    if !decision.auto_update.is_empty() {
        for name in &decision.auto_update {
            tracing::info!("Auto updating repository: {name}");
        }
        sync::auto_update_due(&cx, &decision.auto_update);
    }
    if !decision.notify.is_empty() {
        cx.emit(AppEvent::Repositories(
            repositories::Event::UpdatesAvailable {
                repositories: decision.notify,
            },
        ));
    }
    Ok(())
}
