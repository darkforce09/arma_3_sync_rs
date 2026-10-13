//! The Game log window: the newest RPT log followed live, coloured by line type, with a filter.

use a3s_foundation::error::UserFacing as _;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::{button, icon_text_button, popup_button, toggle};
use a3s_viewer_context::tools::{self, LineKind, LogChoice, LogLine, LogState};
use a3s_viewer_context::{Cx, ViewerContext};
use egui::{Align, Label, Layout, RichText, TextEdit, TextStyle, TextWrapMode, Ui};

use super::{file_name, window_toasts};
use crate::picker::FolderPicker;
use crate::screens::count_text;
use crate::toasts::Toaster;

pub(super) struct LogWindow {
    /// The log was followed since the window opened.
    following: bool,
    choice: LogChoice,
    filter: String,
    errors_only: bool,
    rows: Option<Rows>,
    picker: FolderPicker,
    toasts: Toaster,
}

impl Default for LogWindow {
    fn default() -> Self {
        Self {
            following: false,
            choice: LogChoice::Newest,
            filter: String::new(),
            errors_only: false,
            rows: None,
            picker: FolderPicker::default(),
            toasts: window_toasts("log"),
        }
    }
}

/// The lines that pass the filter, for one revision of the log.
struct Rows {
    revision: u64,
    filter: String,
    errors_only: bool,
    indexes: Vec<usize>,
}

impl LogWindow {
    pub(super) fn show(&mut self, ui: &mut Ui, viewer: &ViewerContext) {
        let cx = viewer.cx();
        if !self.following {
            self.following = true;
            self.follow(cx);
        }
        if let Some(folder) = self.picker.poll() {
            match tools::set_rpt_folder(cx, &folder) {
                Ok(()) => {
                    self.choice = LogChoice::Newest;
                    self.follow(cx);
                }
                Err(err) => self.toasts.error(err.user_message()),
            }
        }
        // One guard for the whole frame: the row indexes stay valid while the lines are shown.
        // Nothing below may write the state; a new follow starts once the guard is gone.
        let state = cx.state().read();
        let log = &state.tools.log;
        let choice_changed = self.bar(ui, log);
        ui.add_space(8.0);
        match (&log.folder, &log.file) {
            (None, _) => self.no_folder(ui),
            (Some(folder), None) => {
                let palette = theme::palette(ui.ctx());
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(icons::FILE_TEXT)
                            .size(32.0)
                            .color(palette.text_secondary),
                    );
                    ui.label(format!("No game log in {} yet.", folder.display()));
                    ui.label(
                        RichText::new("It appears here when the game starts.")
                            .color(palette.text_secondary),
                    );
                });
            }
            (Some(_), Some(file)) => {
                let status = format!(
                    "{} · {}",
                    file_name(file),
                    count_text(log.lines.len(), "line", "lines")
                );
                self.lines(ui, log);
                ui.add_space(4.0);
                ui.label(RichText::new(status).small().weak());
            }
        }
        drop(state);
        if choice_changed {
            self.follow(cx);
        }
        self.toasts.show(ui, viewer);
    }

    fn follow(&mut self, cx: &Cx) {
        if let Err(err) = tools::follow_log(cx, self.choice.clone()) {
            self.toasts.error(err.user_message());
        }
    }

    /// File pop-up, filter field, the errors-only switch and Show in folder; returns whether
    /// another log was chosen.
    fn bar(&mut self, ui: &mut Ui, log: &LogState) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            let newest = log.files.first().map_or_else(
                || "Newest log".to_owned(),
                |file| format!("Newest ({})", file_name(file)),
            );
            let mut options: Vec<(LogChoice, String)> = vec![(LogChoice::Newest, newest)];
            options.extend(
                log.files
                    .iter()
                    .map(|file| (LogChoice::File(file.clone()), file_name(file))),
            );
            let labels: Vec<(LogChoice, &str)> = options
                .iter()
                .map(|(choice, label)| (choice.clone(), label.as_str()))
                .collect();
            changed =
                popup_button(ui, "a3s_viewer::log::file", &mut self.choice, &labels).changed();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let show = ui.add_enabled_ui(log.file.is_some(), |ui| {
                    icon_text_button(ui, icons::FOLDER_OPEN, "Show in folder")
                });
                if show.inner.clicked()
                    && let Some(file) = &log.file
                    && let Err(err) = a3s_platform::reveal::show_in_folder(file)
                {
                    self.toasts.error(format!("Cannot open the folder: {err}"));
                }
                let switch = toggle(ui, &mut self.errors_only);
                // The label flips the switch too, as a checkbox label does.
                let label = ui.add(Label::new("Errors only").sense(egui::Sense::click()));
                if label.clicked() {
                    self.errors_only = !self.errors_only;
                }
                switch.labelled_by(label.id);
                ui.add(
                    TextEdit::singleline(&mut self.filter)
                        .hint_text(format!("{} Filter", icons::MAGNIFYING_GLASS))
                        .desired_width(ui.available_width().min(240.0)),
                );
            });
        });
        changed
    }

    fn no_folder(&mut self, ui: &mut Ui) {
        let palette = theme::palette(ui.ctx());
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(icons::FILE_TEXT)
                    .size(32.0)
                    .color(palette.text_secondary),
            );
            ui.label("The game's log folder was not found.");
            if button(ui, "Choose folder…").clicked() {
                self.picker.open(ui.ctx(), "Game log folder", None);
            }
        });
    }

    /// The filtered lines, monospace, coloured by type; the view follows the end unless the
    /// user scrolled up.
    fn lines(&mut self, ui: &mut Ui, log: &LogState) {
        let stale = self.rows.as_ref().is_none_or(|rows| {
            rows.revision != log.revision
                || rows.filter != self.filter
                || rows.errors_only != self.errors_only
        });
        if stale {
            self.rows = Some(Rows {
                revision: log.revision,
                filter: self.filter.clone(),
                errors_only: self.errors_only,
                indexes: matching(log, &self.filter, self.errors_only),
            });
        }
        let Some(rows) = &self.rows else {
            return;
        };
        let palette = theme::palette(ui.ctx());
        let row_height = ui.text_style_height(&TextStyle::Monospace);
        let height = (ui.available_height() - 24.0).max(row_height);
        egui::Frame::new()
            .fill(palette.surface)
            .stroke(egui::Stroke::new(1.0, palette.border_muted))
            .corner_radius(6)
            .inner_margin(6)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("a3s_viewer::log::lines")
                    .max_height(height)
                    .auto_shrink(false)
                    .stick_to_bottom(true)
                    .show_rows(ui, row_height, rows.indexes.len(), |ui, range| {
                        for &index in &rows.indexes[range] {
                            if let Some(line) = log.lines.get(index) {
                                line_label(ui, line);
                            }
                        }
                    });
            });
    }
}

fn line_label(ui: &mut Ui, line: &LogLine) {
    let palette = theme::palette(ui.ctx());
    let color = match line.kind {
        LineKind::Error => palette.status_danger,
        LineKind::Warning => palette.status_warning,
        LineKind::Info => palette.text_primary,
    };
    ui.add(
        Label::new(RichText::new(&line.text).monospace().color(color))
            .wrap_mode(TextWrapMode::Extend),
    );
}

/// The indexes of the lines that contain `filter` (ignoring case) and, with `errors_only`, are
/// errors.
fn matching(log: &LogState, filter: &str, errors_only: bool) -> Vec<usize> {
    let filter = filter.trim().to_lowercase();
    log.lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !errors_only || line.kind == LineKind::Error)
        .filter(|(_, line)| filter.is_empty() || line.text.to_lowercase().contains(&filter))
        .map(|(index, _)| index)
        .collect()
}
