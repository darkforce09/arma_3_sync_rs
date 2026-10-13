//! The Publishing screen of a repository built on this computer: Publish (status, the step
//! list, what players get), Events and Settings, with one primary button.

mod events;
mod publish;
mod settings;

use std::sync::Arc;

use a3s_foundation::error::UserFacing as _;
use a3s_types::local::Repository;
use a3s_ui::icon::icons;
use a3s_ui::widgets::{primary_button, segmented};
use a3s_viewer_context::admin::{self, PublishStep, Publishing, Remote};
use a3s_viewer_context::{Command, TaskId, TaskKey, TaskKind, ViewerContext};
use egui::{Align, Id, Layout, Ui};
use parking_lot::Mutex;

use super::ScreenCx;

/// The segments of the screen, in order.
const TABS: [&str; 3] = ["Publish", "Events", "Settings"];

/// What the screen shows, copied out of the state so no guard is held while it runs actions.
struct View {
    repository: Repository,
    publishing: Publishing,
    publish_task: Option<TaskId>,
    uploading_events: bool,
    refreshing: bool,
}

impl View {
    fn take(viewer: &ViewerContext, name: &str) -> Option<Self> {
        let (repository, publishing) = {
            let state = viewer.read();
            (
                state.client.repository(name).ok()?.clone(),
                state.admin.publishing(name),
            )
        };
        let running = |kind| viewer.is_running(&TaskKey::repository(kind, name));
        Some(Self {
            repository,
            publishing,
            publish_task: running(TaskKind::Publish),
            uploading_events: running(TaskKind::UploadEvents).is_some(),
            refreshing: running(TaskKind::PublishingStatus).is_some(),
        })
    }

    fn name(&self) -> &str {
        &self.repository.name
    }
}

/// The screen's own state, kept in the egui memory per repository.
#[derive(Default)]
struct ScreenState {
    tab: usize,

    /// The frame the screen was last shown in; a gap means the user came back to it.
    last_frame: Option<u64>,
    events: events::EventsState,
    settings: settings::SettingsState,
}

pub(super) fn show(ui: &mut Ui, name: &str, cx: &mut ScreenCx<'_>) {
    let Some(view) = View::take(cx.viewer, name) else {
        super::empty_state(
            ui,
            icons::CLOUD_ARROW_UP,
            name,
            "This repository no longer exists.",
        );
        return;
    };
    let shared = ui.data_mut(|data| {
        Arc::clone(
            data.get_temp_mut_or_default::<Arc<Mutex<ScreenState>>>(Id::new((
                "a3s_viewer::publishing",
                name,
            ))),
        )
    });
    let mut state = shared.lock();
    refresh_when_needed(ui, &view, &mut state, cx);

    ui.horizontal(|ui| {
        segmented(ui, &mut state.tab, &TABS);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            primary_action(ui, &view, cx);
        });
    });
    ui.add_space(12.0);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| match state.tab {
            0 => publish::show(ui, &view, cx),
            1 => events::show(ui, &view, &mut state.events, cx),
            _ => settings::show(ui, &view, &mut state.settings, cx),
        });
}

/// Reads the server the first time (or after a failure) the screen opens, and compares the
/// source folder again each time the user comes back or a setting made the comparison stale.
fn refresh_when_needed(ui: &Ui, view: &View, state: &mut ScreenState, cx: &mut ScreenCx<'_>) {
    let frame = ui.ctx().cumulative_frame_nr();
    let entered = state.last_frame.is_none_or(|last| frame > last + 1);
    let returned = state.last_frame.is_some() && entered;
    state.last_frame = Some(frame);
    if view.publish_task.is_some() || view.refreshing {
        return;
    }
    let remote = match view.publishing.remote {
        Remote::NotLoaded => true,
        Remote::Failed(_) => entered,
        Remote::Loading | Remote::Loaded(_) => false,
    };
    if remote || returned || view.publishing.local.is_none() {
        send(
            cx,
            admin::Command::Refresh {
                repository: view.name().to_owned(),
                remote,
            },
        );
    }
}

/// The one primary button: publish, upload the events alone, or nothing to do.
fn primary_action(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    if view.publish_task.is_some() {
        ui.add_enabled(false, egui::Button::new("Publishing…"));
        return;
    }
    if view.uploading_events {
        ui.add_enabled(false, egui::Button::new("Uploading events…"));
        return;
    }
    let publishing = &view.publishing;
    let changed_files = publishing
        .local
        .as_ref()
        .and_then(|local| local.as_ref().ok())
        .map(|local| local.changed_files);
    let published = publishing.published_revision().is_some();
    match changed_files {
        Some(0) if published && publishing.events_changed => {
            if primary_button(ui, "Upload events").clicked() {
                send(
                    cx,
                    admin::Command::UploadEvents {
                        repository: view.name().to_owned(),
                    },
                );
            }
        }
        Some(0) if published => {
            ui.add_enabled(
                false,
                egui::Button::new(format!("{} Published", icons::CHECK_CIRCLE)),
            );
        }
        _ => {
            let label = format!("Publish revision {}", publishing.next_revision());
            if primary_button(ui, &label).clicked() {
                send(
                    cx,
                    admin::Command::Publish {
                        repository: view.name().to_owned(),
                        from: PublishStep::Scan,
                    },
                );
            }
        }
    }
}

fn send(cx: &mut ScreenCx<'_>, command: admin::Command) {
    if let Err(err) = cx.viewer.send(Command::Admin(command)) {
        cx.toasts.error(err.user_message());
    }
}
