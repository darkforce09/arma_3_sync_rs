//! Toasts in the bottom right corner: info, success, warning and error notes, and notes with an
//! Undo button that runs a callback on the context.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_viewer_context::ViewerContext;
use egui::{Align2, Frame, Id, Margin, Order, RichText, Ui};
use egui_toast::{Toast, ToastKind, ToastOptions, ToastStyle, Toasts};
use parking_lot::Mutex;

/// What Undo does; it runs on the UI thread, at most once.
pub type UndoAction = Box<dyn FnOnce(&ViewerContext) + Send>;

/// How long a plain note stays.
const NOTE_DURATION: Duration = Duration::from_secs(5);

/// How long an error or an undoable note stays.
const LONG_DURATION: Duration = Duration::from_secs(8);

/// Where egui-toast keeps the live toasts in the context memory.
fn toasts_id() -> Id {
    Id::new("a3s_viewer::toasts")
}

/// The toasts of the main window. Toasts added before the next frame are shown then.
#[derive(Default)]
pub struct Toaster {
    pending: Vec<Toast>,

    /// Undo actions by the custom toast kind of their toast.
    undo: BTreeMap<u32, UndoAction>,

    /// Kinds whose Undo button was clicked, filled by the toast contents.
    clicked: Arc<Mutex<Vec<u32>>>,
    next_kind: u32,
}

impl Toaster {
    /// A neutral note.
    pub fn info(&mut self, text: impl Into<String>) {
        self.note(ToastKind::Info, text.into(), NOTE_DURATION);
    }

    /// A note that something finished well.
    pub fn success(&mut self, text: impl Into<String>) {
        self.note(ToastKind::Success, text.into(), NOTE_DURATION);
    }

    /// A note that needs attention, e.g. updates are available.
    pub fn warning(&mut self, text: impl Into<String>) {
        self.note(ToastKind::Warning, text.into(), LONG_DURATION);
    }

    /// A failure.
    pub fn error(&mut self, text: impl Into<String>) {
        self.note(ToastKind::Error, text.into(), LONG_DURATION);
    }

    /// A note with an Undo button; a click runs `undo` and closes the note. Once the note is
    /// gone, `undo` is dropped without running.
    pub fn undoable(
        &mut self,
        text: impl Into<String>,
        undo: impl FnOnce(&ViewerContext) + Send + 'static,
    ) {
        let kind = self.next_kind;
        self.next_kind = self.next_kind.wrapping_add(1);
        self.undo.insert(kind, Box::new(undo));
        self.pending.push(
            Toast::new()
                .kind(ToastKind::Custom(kind))
                .text(text.into())
                .options(options(LONG_DURATION)),
        );
    }

    fn note(&mut self, kind: ToastKind, text: String, duration: Duration) {
        self.pending.push(
            Toast::new()
                .kind(kind)
                .text(text)
                .options(options(duration))
                .style(style()),
        );
    }

    /// Shows the toasts, then runs the Undo actions clicked in this frame and forgets those
    /// whose note is gone.
    pub(crate) fn show(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let mut toasts = Toasts::with_id(toasts_id())
            .anchor(Align2::RIGHT_BOTTOM, (-16.0, -16.0))
            .direction(egui::Direction::BottomUp)
            .order(Order::Tooltip);
        for &kind in self.undo.keys() {
            toasts = toasts.custom_contents(ToastKind::Custom(kind), {
                let clicked = Arc::clone(&self.clicked);
                move |ui: &mut Ui, toast: &mut Toast| undo_contents(ui, toast, kind, &clicked)
            });
        }
        for toast in self.pending.drain(..) {
            toasts.add(toast);
        }
        toasts.show(ui);

        let clicked = std::mem::take(&mut *self.clicked.lock());
        for kind in clicked {
            if let Some(undo) = self.undo.remove(&kind) {
                undo(viewer);
            }
        }
        let live: Vec<ToastKind> = ui.data(|data| {
            data.get_temp::<Vec<Toast>>(toasts_id())
                .unwrap_or_default()
                .iter()
                .map(|toast| toast.kind)
                .collect()
        });
        self.undo
            .retain(|kind, _| live.contains(&ToastKind::Custom(*kind)));
    }
}

fn options(duration: Duration) -> ToastOptions {
    ToastOptions::default()
        .duration(duration)
        .show_progress(false)
        .show_icon(true)
}

fn style() -> ToastStyle {
    ToastStyle {
        info_icon: icons::INFO.into(),
        warning_icon: icons::WARNING.into(),
        error_icon: icons::X_CIRCLE.into(),
        success_icon: icons::CHECK_CIRCLE.into(),
        ..ToastStyle::default()
    }
}

fn undo_contents(
    ui: &mut Ui,
    toast: &mut Toast,
    kind: u32,
    clicked: &Mutex<Vec<u32>>,
) -> egui::Response {
    let palette = theme::palette(ui.ctx());
    Frame::window(ui.style())
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::ARROW_COUNTER_CLOCKWISE)
                        .color(palette.status_color(ProgressTone::Info)),
                );
                ui.label(toast.text.clone());
                if ui.button("Undo").clicked() {
                    clicked.lock().push(kind);
                    toast.close();
                }
            });
        })
        .response
}
