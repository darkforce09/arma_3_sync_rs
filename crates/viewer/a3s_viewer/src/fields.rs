//! Form rows whose control is labelled by the row's label, so screen readers (and tests) find
//! the control by its label.

use a3s_ui::widgets::form::ROW_HEIGHT;
use a3s_ui::widgets::toggle;
use egui::{Align, Layout, Response, TextEdit, Ui, WidgetInfo, WidgetType, vec2};

/// A form row: `label` on the left and the control `add_control` adds right-aligned, labelled
/// by `label`.
pub(crate) fn labelled_row(
    ui: &mut Ui,
    label: &str,
    add_control: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            let label = ui.label(label);
            ui.with_layout(Layout::right_to_left(Align::Center), add_control)
                .inner
                .labelled_by(label.id)
        },
    )
    .inner
}

/// A form row with a single-line text field.
pub(crate) fn text_row(ui: &mut Ui, label: &str, text: &mut String, password: bool) -> Response {
    labelled_row(ui, label, |ui| {
        ui.add(
            TextEdit::singleline(text)
                .password(password)
                .desired_width(240.0),
        )
    })
}

/// A form row with a toggle switch, announced as a checkbox named `label`.
pub(crate) fn toggle_row(ui: &mut Ui, label: &str, on: &mut bool) -> Response {
    labelled_row(ui, label, |ui| {
        let response = toggle(ui, on);
        let (enabled, state) = (ui.is_enabled(), *on);
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, state, label));
        response
    })
}
