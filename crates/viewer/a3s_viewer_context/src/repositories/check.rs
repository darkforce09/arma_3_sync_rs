//! The check of repositories asked for by the user.

use crate::checker::{CheckTrigger, apply_results, check_many};
use crate::error::ContextError;
use crate::event::AppEvent;
use crate::tasks::{Cx, TaskError, TaskKey, TaskKind};

fn key() -> TaskKey {
    TaskKey::new(TaskKind::CheckRepositories)
}

/// Starts the check of `names`. Neither notifies nor updates automatically.
// Mirrors ui/repository/dialogs/progress/ProgressSynchronizationDialog.java:30-154
pub(super) fn start(cx: &Cx, names: Vec<String>) -> Result<(), ContextError> {
    if names.is_empty() {
        tracing::info!("No repository to synchronize with.");
        cx.emit(AppEvent::Repositories(super::Event::CheckCompleted {
            trigger: CheckTrigger::Manual,
            results: Vec::new(),
        }));
        return Ok(());
    }
    cx.spawn_task(key(), true, move |task| async move {
        let cx = task.cx.clone();
        if let [name] = names.as_slice() {
            tracing::info!("Synchronization with repository: {name}");
        } else {
            tracing::info!("Synchronization with repositories...");
        }
        let events = task.sync_events();
        let (_, mut results) =
            check_many(&cx, Some(&names), CheckTrigger::Manual, &task, &events).await;
        apply_results(&cx, CheckTrigger::Manual, &mut results);
        if task.cancel.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        tracing::info!("Synchronization with repositories done.");
        Ok(())
    })?;
    Ok(())
}

/// Cancels the running check, if any.
// Mirrors ui/repository/dialogs/progress/ProgressSynchronizationDialog.java:156-166
pub(super) fn cancel(cx: &Cx) {
    if let Some(id) = cx.is_running(&key())
        && cx.cancel_task(id)
    {
        tracing::info!("Synchronization with repositories canceled.");
    }
}
