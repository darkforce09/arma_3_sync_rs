//! Container frames from `ui/UiStyle.java`: titled section, panel, toolbar, status bar, pane
//! divider and the borderless table scroll frame.
//!
//! Colours are read from the context's current theme on every call, which replaces
//! `UiStyle.refreshTree` and the client-property re-styling (`ui/UiStyle.java:91-134`).

use egui::{
    Align2, CornerRadius, Frame, InnerResponse, Margin, Painter, Rect, Stroke, StrokeKind, Ui, pos2,
};

use crate::fonts;
use crate::theme::{self, ThemeDefinition};

/// Swing `LineBorder(c, 1, true)` draws an outer arc equal to its thickness: effectively square.
const LINE_BORDER_RADIUS: u8 = 1;

/// Left inset of a section title, from the border's left edge.
const SECTION_TITLE_INSET: f32 = 8.0;

/// Gap left free around the section title on the interrupted border line.
const SECTION_TITLE_GAP: f32 = 4.0;

fn margin(value: i32) -> i8 {
    value.clamp(0, i32::from(i8::MAX)) as i8
}

/// The bordered panel frame: 1 px `border`, padding 8 / 12 / 12 / 12 (t/l/b/r).
///
/// Mirrors `ui/UiStyle.java:64-70`. The 1 px line is added to the padding as in Swing's
/// compound border.
pub fn panel_frame(def: &ThemeDefinition) -> Frame {
    let m = &def.metrics;
    Frame::new()
        .stroke(Stroke::new(1.0, def.palette.border))
        .corner_radius(CornerRadius::same(LINE_BORDER_RADIUS))
        .inner_margin(Margin {
            left: margin(m.spacing_md),
            right: margin(m.spacing_md),
            top: margin(m.spacing_sm),
            bottom: margin(m.spacing_md),
        })
}

/// Shows `add_contents` inside a [`panel_frame`] of the current theme.
pub fn panel<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    panel_frame(theme::current(ui.ctx())).show(ui, add_contents)
}

/// A titled section: 1 px `border` with the title on the top line (Swing `TitledBorder`
/// LEFT/TOP), padding 12 / 16 / 16 / 16 (t/l/b/r) below the title.
///
/// Mirrors `ui/UiStyle.java:53-62`.
pub fn section<R>(
    ui: &mut Ui,
    title: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let def = theme::current(ui.ctx());
    let m = &def.metrics;
    let font = egui::FontId::proportional(fonts::BODY_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), font, def.palette.text_primary);
    let title_height = galley.size().y;

    let frame = Frame::new().inner_margin(Margin {
        left: margin(m.spacing_lg + 1),
        right: margin(m.spacing_lg + 1),
        top: margin(title_height.ceil() as i32 + m.spacing_md),
        bottom: margin(m.spacing_lg + 1),
    });
    let inner = frame.show(ui, add_contents);
    let outer = inner.response.rect;
    let border = Rect::from_min_max(
        pos2(outer.left(), outer.top() + title_height / 2.0),
        outer.max,
    );
    let stroke = Stroke::new(1.0, def.palette.border);
    let painter = ui.painter();

    let text_left = border.left() + SECTION_TITLE_INSET;
    let text_rect = Align2::LEFT_TOP.anchor_size(pos2(text_left, outer.top()), galley.size());
    paint_line_border_with_gap(
        painter,
        border,
        stroke,
        text_rect.left() - SECTION_TITLE_GAP,
        text_rect.right() + SECTION_TITLE_GAP,
    );
    painter.galley(text_rect.min, galley, def.palette.text_primary);
    inner
}

/// Paints a square 1 px border whose top edge is interrupted between `gap_from` and `gap_to`.
fn paint_line_border_with_gap(
    painter: &Painter,
    rect: Rect,
    stroke: Stroke,
    gap_from: f32,
    gap_to: f32,
) {
    let r = rect.shrink(stroke.width / 2.0);
    painter.line_segment(
        [r.left_top(), pos2(gap_from.max(r.left()), r.top())],
        stroke,
    );
    painter.line_segment(
        [pos2(gap_to.min(r.right()), r.top()), r.right_top()],
        stroke,
    );
    painter.line_segment([r.right_top(), r.right_bottom()], stroke);
    painter.line_segment([r.right_bottom(), r.left_bottom()], stroke);
    painter.line_segment([r.left_bottom(), r.left_top()], stroke);
}

/// The toolbar frame without its bottom line: fill `surface`, padding 4 / 16 / 4 / 16.
///
/// Mirrors `ui/UiStyle.java:190-197`. The 1 px line is part of the bottom padding.
pub fn toolbar_frame(def: &ThemeDefinition) -> Frame {
    let m = &def.metrics;
    Frame::new().fill(def.palette.surface).inner_margin(Margin {
        left: margin(m.spacing_lg),
        right: margin(m.spacing_lg),
        top: margin(m.spacing_xs),
        bottom: margin(m.spacing_xs + 1),
    })
}

/// Shows `add_contents` in a toolbar: [`toolbar_frame`] plus a 1 px `border` line at the
/// bottom.
pub fn toolbar<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let def = theme::current(ui.ctx());
    let inner = toolbar_frame(def).show(ui, add_contents);
    let rect = inner.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, def.palette.border),
    );
    inner
}

/// The status bar frame without its top line: fill `surfaceVariant`, padding 8 / 16 / 8 / 16.
///
/// Mirrors `ui/UiStyle.java:199-206`. The 1 px line is part of the top padding.
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

/// Paints the pane divider: a 1 px `borderMuted` line along the right edge of `rect`.
///
/// Mirrors `ui/UiStyle.java:212-215`.
pub fn paint_pane_divider(painter: &Painter, rect: Rect, def: &ThemeDefinition) {
    painter.vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, def.palette.border_muted),
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

/// The frame of a table's scroll pane: no border, no margins (`Table.scrollPaneBorder` =
/// `createEmptyBorder()`, `ui/theme/ThemeTokens.java:128`).
pub fn table_scroll_frame(_def: &ThemeDefinition) -> Frame {
    Frame::new()
        .stroke(Stroke::NONE)
        .inner_margin(Margin::ZERO)
        .outer_margin(Margin::ZERO)
}

/// Paints a Swing `LineBorder(color, 1, true)` around `rect` (inside the rect).
pub(crate) fn paint_line_border(painter: &Painter, rect: Rect, color: egui::Color32) {
    painter.rect_stroke(
        rect,
        CornerRadius::same(LINE_BORDER_RADIUS),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
}
