//! Status display: the tinted status pill and the numbered step row of a step list.

use std::sync::Arc;

use egui::{
    Align2, CornerRadius, FontId, Galley, Pos2, Rect, Response, Sense, Ui, Vec2, pos2, vec2,
};

use crate::icon::icons;
use crate::theme::{self, ProgressTone};

/// Height of a step row.
pub const STEP_ROW_HEIGHT: f32 = 36.0;

/// Diameter of the step number circle.
const STEP_MARKER_SIZE: f32 = 22.0;

/// A rounded pill with `text` in the colour of `tone` on a light tint of it.
pub fn status_pill(ui: &mut Ui, text: &str, tone: ProgressTone) -> Response {
    let size = pill_galley(ui, text, tone).size() + PILL_PADDING;
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    paint_status_pill(ui, rect.right_center(), text, tone);
    response
}

/// Room around the pill text.
const PILL_PADDING: Vec2 = vec2(14.0, 4.0);

fn pill_galley(ui: &Ui, text: &str, tone: ProgressTone) -> Arc<Galley> {
    let color = theme::palette(ui.ctx()).status_color(tone);
    let font = egui::TextStyle::Small.resolve(ui.style());
    ui.painter().layout_no_wrap(text.to_owned(), font, color)
}

/// Paints a [`status_pill`] whose right edge is centred on `right_center`, without allocating
/// space; returns its rectangle.
pub(crate) fn paint_status_pill(
    ui: &Ui,
    right_center: Pos2,
    text: &str,
    tone: ProgressTone,
) -> Rect {
    let color = theme::palette(ui.ctx()).status_color(tone);
    let galley = pill_galley(ui, text, tone);
    let rect = Align2::RIGHT_CENTER.anchor_size(right_center, galley.size() + PILL_PADDING);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(9), color.gamma_multiply(0.18));
        painter.galley(
            Align2::CENTER_CENTER
                .anchor_size(rect.center(), galley.size())
                .min,
            galley,
            color,
        );
    }
    rect
}

/// Where a step of a step list stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    /// Finished: a check mark in a success circle.
    Done,
    /// Running: the number in an accent circle.
    Active,
    /// Not started: the number in a muted circle, muted title.
    Pending,
}

/// One row of a step list: the step marker, `title`, and `detail` right-aligned.
pub fn step_row(
    ui: &mut Ui,
    number: usize,
    state: StepState,
    title: &str,
    detail: &str,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), STEP_ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let palette = theme::palette(ui.ctx());
    let painter = ui.painter();
    let marker = Rect::from_center_size(
        pos2(rect.left() + STEP_MARKER_SIZE / 2.0, rect.center().y),
        vec2(STEP_MARKER_SIZE, STEP_MARKER_SIZE),
    );
    let (fill, mark_color, mark, title_color) = match state {
        StepState::Done => (
            palette.status_success.gamma_multiply(0.18),
            palette.status_success,
            icons::CHECK.to_owned(),
            palette.text_primary,
        ),
        StepState::Active => (
            palette.accent_primary.gamma_multiply(0.18),
            palette.accent_primary,
            number.to_string(),
            palette.text_primary,
        ),
        StepState::Pending => (
            palette.surface_muted,
            palette.text_secondary,
            number.to_string(),
            palette.text_secondary,
        ),
    };
    painter.rect_filled(
        marker,
        CornerRadius::same((STEP_MARKER_SIZE / 2.0) as u8),
        fill,
    );
    painter.text(
        marker.center(),
        Align2::CENTER_CENTER,
        mark,
        FontId::proportional(12.0),
        mark_color,
    );
    painter.text(
        pos2(marker.right() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        egui::TextStyle::Body.resolve(ui.style()),
        title_color,
    );
    painter.text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        detail,
        egui::TextStyle::Small.resolve(ui.style()),
        palette.text_secondary,
    );
    response
}
