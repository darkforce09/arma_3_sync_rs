//! System Settings controls: the labelled form row, the toggle switch, the segmented control
//! and the pop-up button.
//!
//! Put rows inside [`crate::frames::group`] and separate them with
//! [`crate::frames::group_separator`].

use std::hash::Hash;

use egui::{
    Align, Align2, Color32, ComboBox, CornerRadius, InnerResponse, Layout, Rect, Response, Sense,
    Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::theme;

/// Height of a form row.
pub const ROW_HEIGHT: f32 = 30.0;

/// Size of the toggle switch.
const TOGGLE_SIZE: egui::Vec2 = vec2(32.0, 18.0);

/// Horizontal padding of a segment around its label.
const SEGMENT_PADDING: f32 = 10.0;

/// Height of the segmented control.
const SEGMENT_HEIGHT: f32 = 22.0;

/// A form row: `label` on the left and `add_control` right-aligned, [`ROW_HEIGHT`] tall.
pub fn form_row<R>(
    ui: &mut Ui,
    label: &str,
    add_control: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), ROW_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(label);
            ui.with_layout(Layout::right_to_left(Align::Center), add_control)
                .inner
        },
    )
}

/// The macOS switch. A click flips `on` and marks the response changed.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(TOGGLE_SIZE, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let enabled = ui.is_enabled();
    let state = *on;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, state, ""));

    if ui.is_rect_visible(rect) {
        let palette = theme::palette(ui.ctx());
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let track = palette
            .surface_muted
            .lerp_to_gamma(palette.accent_primary, t);
        let radius = rect.height() / 2.0;
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(radius as u8), track);
        let knob_radius = radius - 2.0;
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        painter.circle(
            pos2(x, rect.center().y),
            knob_radius,
            Color32::WHITE,
            Stroke::new(0.5, Color32::from_black_alpha(40)),
        );
    }
    response
}

/// A segmented control with one segment per label. A click selects that segment and marks the
/// response changed.
pub fn segmented(ui: &mut Ui, selected: &mut usize, labels: &[&str]) -> Response {
    let palette = theme::palette(ui.ctx());
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galleys: Vec<_> = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), palette.text_primary)
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|g| g.size().x + 2.0 * SEGMENT_PADDING)
        .collect();
    let total = vec2(widths.iter().sum::<f32>() + 4.0, SEGMENT_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(total, Sense::hover());

    let painter = ui.painter().clone();
    painter.rect_filled(rect, CornerRadius::same(6), palette.surface_muted);
    let mut x = rect.left() + 2.0;
    for (i, (galley, width)) in galleys.into_iter().zip(widths).enumerate() {
        let segment =
            Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(width, rect.height() - 4.0));
        let segment_response = ui.interact(segment, response.id.with(i), Sense::click());
        let is_selected = *selected == i;
        segment_response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, true, is_selected, labels[i])
        });
        if segment_response.clicked() && !is_selected {
            *selected = i;
            response.mark_changed();
        }
        if *selected == i {
            painter.rect(
                segment,
                CornerRadius::same(5),
                palette.control,
                Stroke::new(0.5, palette.border),
                StrokeKind::Inside,
            );
        } else if segment_response.hovered() {
            painter.rect_filled(segment, CornerRadius::same(5), palette.state_hover_overlay);
        }
        let text_pos = Align2::CENTER_CENTER.anchor_size(segment.center(), galley.size());
        painter.galley(text_pos.min, galley, palette.text_primary);
        x += width;
    }
    response
}

/// A pop-up button: shows the label of the selected option and opens a menu of all `options`.
/// Choosing another option sets `selected` and marks the response changed.
pub fn popup_button<T: PartialEq + Clone>(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug,
    selected: &mut T,
    options: &[(T, &str)],
) -> Response {
    let current = options
        .iter()
        .find(|(value, _)| value == selected)
        .map_or("", |(_, label)| *label);
    let mut changed = false;
    let mut response = ComboBox::from_id_salt(id_salt)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (value, label) in options {
                if ui.selectable_label(value == selected, *label).clicked() && value != selected {
                    *selected = value.clone();
                    changed = true;
                }
            }
        })
        .response;
    if changed {
        response.mark_changed();
    }
    response
}
