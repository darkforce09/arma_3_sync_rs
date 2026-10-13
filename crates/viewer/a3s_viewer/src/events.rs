//! What the window does with the context's events: toasts, the last check time, and desktop
//! notifications while hidden. The views themselves read the state every frame.

use std::time::SystemTime;

use a3s_viewer_context::repositories::{self, UPDATES_AVAILABLE_MESSAGE};
use a3s_viewer_context::{AppEvent, CheckOutcome, TaskEvent, TaskOutcome, sync};

use crate::notify;
use crate::screens::count_text;
use crate::sheets::{Sheet, Sheets, UpdateIssues};
use crate::toasts::Toaster;

/// The parts of the window an event changes.
pub(crate) struct EventSink<'a> {
    pub(crate) toasts: &'a mut Toaster,
    pub(crate) sheets: &'a mut Sheets,
    pub(crate) last_check: &'a mut Option<SystemTime>,
    pub(crate) hidden: bool,
}

impl EventSink<'_> {
    pub(crate) fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Repositories(event) => self.repositories(event),
            AppEvent::Sync(sync::Event::AutoUpdateRequested { repositories }) => {
                self.toasts.info(format!(
                    "Updating {} in the background",
                    repositories.join(", ")
                ));
            }
            AppEvent::Sync(sync::Event::DownloadFinished { repository, result }) => {
                self.download_finished(repository, result);
            }
            AppEvent::Error { error, .. } => self.toasts.error(error.message),
            AppEvent::Task(TaskEvent::Finished { info, outcome }) => match outcome {
                TaskOutcome::Failed(error) => {
                    self.toasts.error(format!(
                        "{} failed: {}",
                        info.key.kind.label(),
                        error.message
                    ));
                }
                TaskOutcome::Panicked(message) => {
                    self.toasts.error(format!(
                        "{} stopped unexpectedly: {message}",
                        info.key.kind.label()
                    ));
                }
                TaskOutcome::Completed | TaskOutcome::Cancelled => {}
            },
            _ => {}
        }
    }

    /// A download ended: a toast, and the issues sheet when files failed.
    fn download_finished(&mut self, repository: String, result: sync::DownloadResult) {
        match result {
            sync::DownloadResult::Success { trashed } => {
                self.toasts.success(format!("{repository} is up to date"));
                if trashed > 0 {
                    self.toasts.info(format!(
                        "Moved {} to the Trash",
                        count_text(trashed, "mod", "mods")
                    ));
                }
            }
            sync::DownloadResult::Issues(errors) => {
                self.toasts.warning(format!(
                    "{repository}: {} not updated",
                    count_text(errors.len(), "file", "files")
                ));
                self.sheets
                    .push(Sheet::UpdateIssues(UpdateIssues::new(repository)));
            }
            sync::DownloadResult::Failed(error) => {
                self.toasts
                    .error(format!("Could not update {repository}: {}", error.message));
            }
        }
    }

    fn repositories(&mut self, event: repositories::Event) {
        match event {
            repositories::Event::UpdatesAvailable { repositories } => {
                self.toasts
                    .warning(format!("Updates available: {}", repositories.join(", ")));
                if self.hidden {
                    notify::desktop(UPDATES_AVAILABLE_MESSAGE);
                }
            }
            repositories::Event::CheckCompleted { trigger, results } => {
                *self.last_check = Some(SystemTime::now());
                let failed: Vec<&str> = results
                    .iter()
                    .filter(|result| matches!(result.outcome, CheckOutcome::Failed(_)))
                    .map(|result| result.name.as_str())
                    .collect();
                if trigger == a3s_viewer_context::CheckTrigger::Manual && !failed.is_empty() {
                    self.toasts
                        .error(format!("Could not check {}", failed.join(", ")));
                }
            }
            _ => {}
        }
    }
}
