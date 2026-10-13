//! The tool windows: server keys, radio plugin and game log. Each keeps its own state and
//! toasts while open; the work runs in the context's `tools` domain.

mod keys;
mod log;
mod radio;

use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_viewer_context::{Cx, TaskId, ViewerContext};
use egui::{Frame, Id, RichText, Ui};

use super::SecondaryWindow;
use crate::toasts::Toaster;

/// The state of the tool windows; a window's state is reset when it closes.
#[derive(Default)]
pub(super) struct ToolWindows {
    keys: keys::KeysWindow,
    radio: radio::RadioWindow,
    log: log::LogWindow,
}

impl ToolWindows {
    /// Shows the tool window `window`.
    pub(super) fn show(&mut self, ui: &mut Ui, window: SecondaryWindow, viewer: &ViewerContext) {
        Frame::new().inner_margin(16).show(ui, |ui| match window {
            SecondaryWindow::ServerKeys => self.keys.show(ui, viewer),
            SecondaryWindow::RadioPlugin => self.radio.show(ui, viewer),
            SecondaryWindow::GameLog => self.log.show(ui, viewer),
            SecondaryWindow::Settings => {}
        });
    }

    /// Forgets the state of `window`, which closed, and stops its work.
    pub(super) fn close(&mut self, window: SecondaryWindow, viewer: &ViewerContext) {
        match window {
            SecondaryWindow::ServerKeys => self.keys = keys::KeysWindow::default(),
            SecondaryWindow::RadioPlugin => self.radio = radio::RadioWindow::default(),
            SecondaryWindow::GameLog => {
                a3s_viewer_context::tools::stop_log(viewer.cx());
                self.log = log::LogWindow::default();
            }
            SecondaryWindow::Settings => {}
        }
    }
}

/// The toasts of the tool window `name`.
fn window_toasts(name: &str) -> Toaster {
    Toaster::with_id(Id::new(("a3s_viewer::tool_toasts", name)))
}

/// Whether task `id` has finished (it is no longer registered).
fn finished(cx: &Cx, id: TaskId) -> bool {
    cx.task_snapshot(id).is_none()
}

/// One line under the title bar saying what the tool does.
fn intro(ui: &mut Ui, line: &str) {
    let palette = theme::palette(ui.ctx());
    ui.label(RichText::new(line).color(palette.text_secondary));
    ui.add_space(12.0);
}

/// A warning icon and `text`.
fn warning_row(ui: &mut Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(icons::WARNING).color(palette.status_warning));
        ui.label(text);
    });
}

/// The last component of `path`, or the whole path.
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
