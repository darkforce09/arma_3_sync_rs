//! Containers of the macOS layout: grouped box, unified toolbar, sidebar, status bar, pane
//! divider and the borderless table scroll frame.
//!
//! Colours are read from the context's current theme on every call, so a theme switch needs no
//! re-styling.

use egui::{CornerRadius, Frame, InnerResponse, Margin, Painter, Rect, Stroke, Ui};

use crate::theme::{self, ThemeDefinition};

/// Corner radius of a grouped box.
pub const GROUP_RADIUS: u8 = 10;

/// Height of the unified toolbar.
pub const TOOLBAR_HEIGHT: f32 = 46.0;

/// Default width of the sidebar.
pub const SIDEBAR_WIDTH: f32 = 200.0;

fn margin(value: i32) -> i8 {
    value.clamp(0, i32::from(i8::MAX)) as i8
}

/// A grouped box as in System Settings: `surface` fill, 1 px `border`, rounded corners.
/// Separate its rows with [`group_separator`].
pub fn group_frame(def: &ThemeDefinition) -> Frame {
    let m = &def.metrics;
    Frame::new()
        .fill(def.palette.surface)
        .stroke(Stroke::new(1.0, def.palette.border_muted))
        .corner_radius(CornerRadius::same(GROUP_RADIUS))
        .inner_margin(Margin::symmetric(
            margin(m.spacing_md),
            margin(m.spacing_xs),
        ))
}

/// Shows `add_contents` inside a [`group_frame`] that fills the available width.
pub fn group<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    group_frame(theme::current(ui.ctx())).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add_contents(ui)
    })
}

/// A 1 px `border_muted` line across the current group, between two rows.
pub fn group_separator(ui: &mut Ui) {
    let def = theme::current(ui.ctx());
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, def.palette.border_muted),
    );
}

/// The unified toolbar frame without its bottom line: `sidebar` fill, padding 12 / 6.
pub fn toolbar_frame(def: &ThemeDefinition) -> Frame {
    let m = &def.metrics;
    Frame::new()
        .fill(def.palette.sidebar)
        .inner_margin(Margin::symmetric(
            margin(m.spacing_md),
            margin(m.spacing_xs + 2),
        ))
}

/// Shows `add_contents` left to right in the unified toolbar: [`toolbar_frame`],
/// [`TOOLBAR_HEIGHT`] tall, plus a 1 px `border` line at the bottom.
pub fn toolbar<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let def = theme::current(ui.ctx());
    let inner = toolbar_frame(def).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let height = TOOLBAR_HEIGHT - 2.0 * f32::from(toolbar_frame(def).inner_margin.top) - 1.0;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), height),
            egui::Layout::left_to_right(egui::Align::Center),
            add_contents,
        )
        .inner
    });
    let rect = inner.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, def.palette.border),
    );
    inner
}

/// The sidebar frame: `sidebar` fill, padding 8. Pair it with [`paint_pane_divider`].
pub fn sidebar_frame(def: &ThemeDefinition) -> Frame {
    Frame::new()
        .fill(def.palette.sidebar)
        .inner_margin(Margin::same(margin(def.metrics.spacing_sm)))
}

/// The status bar frame without its top line: fill `surfaceVariant`, padding 8 / 16 / 8 / 16.
/// The 1 px line is part of the top padding.
pub fn status_bar_frame(def: &ThemeDefinition) -> Frame {
    let m = &def.metrics;
    Frame::new()
        .fill(def.palette.surface_variant)
        .inner_margin(Margin {
            left: margin(m.spacing_lg),
            right: margin(m.spacing_lg),
            top: margin(m.spacing_sm + 1),
            bottom: margin(m.spacing_sm),
        })
}

/// Shows `add_contents` in a status bar: [`status_bar_frame`] plus a 1 px `border` line at the
/// top.
pub fn status_bar<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let def = theme::current(ui.ctx());
    let inner = status_bar_frame(def).show(ui, add_contents);
    let rect = inner.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, def.palette.border),
    );
    inner
}

/// Paints the pane divider: a 1 px `border` line along the right edge of `rect`.
pub fn paint_pane_divider(painter: &Painter, rect: Rect, def: &ThemeDefinition) {
    painter.vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, def.palette.border),
    );
}

/// Shows `add_contents` with a 1 px right margin and paints a [`paint_pane_divider`] line there.
pub fn pane_divider<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let def = theme::current(ui.ctx());
    let inner = Frame::new()
        .inner_margin(Margin {
            right: 1,
            ..Margin::ZERO
        })
        .show(ui, add_contents);
    paint_pane_divider(ui.painter(), inner.response.rect, def);
    inner
}

/// The frame of a table's scroll pane: no border, no margins.
pub fn table_scroll_frame(_def: &ThemeDefinition) -> Frame {
    Frame::new()
        .stroke(Stroke::NONE)
        .inner_margin(Margin::ZERO)
        .outer_margin(Margin::ZERO)
}
