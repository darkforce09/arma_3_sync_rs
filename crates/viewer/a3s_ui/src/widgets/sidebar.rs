//! Source-list sidebar: section headings and selectable rows with an icon and an optional
//! badge. Show them inside [`crate::frames::sidebar_frame`].

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, CornerRadius, FontId, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::theme::{self, ProgressTone};

/// Height of a sidebar row.
pub const SIDEBAR_ROW_HEIGHT: f32 = 26.0;

/// Size of the row icon glyph in points.
const ICON_SIZE: f32 = 15.0;

/// A small muted section heading ("Library", "Repositories").
pub fn sidebar_heading(ui: &mut Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(text)
            .small()
            .strong()
            .color(palette.text_secondary),
    );
}

/// A sidebar row: icon, label and an optional badge on the right.
#[derive(Clone, Debug)]
pub struct SidebarItem<'a> {
    icon: &'a str,
    label: &'a str,
    badge: Option<&'a str>,
    badge_tone: Option<ProgressTone>,
}

impl<'a> SidebarItem<'a> {
    /// A row showing the glyph `icon` (from [`crate::icon::icons`]) and `label`.
    pub fn new(icon: &'a str, label: &'a str) -> Self {
        Self {
            icon,
            label,
            badge: None,
            badge_tone: None,
        }
    }

    /// Muted text at the right edge, e.g. a count.
    pub fn badge(mut self, text: &'a str) -> Self {
        self.badge = Some(text);
        self
    }

    /// Draws the badge as a pill in `tone` instead of muted text.
    pub fn badge_tone(mut self, tone: ProgressTone) -> Self {
        self.badge_tone = Some(tone);
        self
    }

    /// Paints the row, highlighted when `selected`; a click on it is in the response.
    pub fn show(self, ui: &mut Ui, selected: bool) -> Response {
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), SIDEBAR_ROW_HEIGHT),
            Sense::click(),
        );
        let enabled = ui.is_enabled();
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, self.label)
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }

        let palette = theme::palette(ui.ctx());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, CornerRadius::same(6), palette.state_selection_bg);
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(6), palette.state_hover_overlay);
        }
        let (icon_color, text_color) = if selected {
            (palette.state_selection_fg, palette.state_selection_fg)
        } else {
            (palette.accent_primary, palette.text_primary)
        };
        let left = rect.left() + 8.0;
        painter.text(
            pos2(left, rect.center().y),
            Align2::LEFT_CENTER,
            self.icon,
            FontId::proportional(ICON_SIZE),
            icon_color,
        );
        // The badge first: the label is cut with "…" before it.
        let mut label_right = rect.right() - 8.0;
        if let Some(badge) = self.badge {
            let font = egui::TextStyle::Small.resolve(ui.style());
            let right = pos2(rect.right() - 8.0, rect.center().y);
            let badge_rect = if let Some(tone) = self.badge_tone {
                let color = palette.status_color(tone);
                let galley = painter.layout_no_wrap(badge.to_owned(), font, color);
                let pill = Rect::from_center_size(
                    pos2(right.x - galley.size().x / 2.0 - 6.0, right.y),
                    galley.size() + vec2(12.0, 4.0),
                );
                painter.rect_filled(pill, CornerRadius::same(8), color.gamma_multiply(0.18));
                painter.galley(
                    Align2::CENTER_CENTER
                        .anchor_size(pill.center(), galley.size())
                        .min,
                    galley,
                    color,
                );
                pill
            } else {
                let color = if selected {
                    palette.state_selection_fg
                } else {
                    palette.text_secondary
                };
                painter.text(right, Align2::RIGHT_CENTER, badge, font, color)
            };
            label_right = badge_rect.left() - 6.0;
        }
        let label_left = left + ICON_SIZE + 8.0;
        let mut job = LayoutJob::simple_singleline(
            self.label.to_owned(),
            egui::TextStyle::Body.resolve(ui.style()),
            text_color,
        );
        job.wrap = TextWrapping {
            max_width: (label_right - label_left).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = painter.layout_job(job);
        let elided = galley.elided;
        painter.galley(
            Align2::LEFT_CENTER
                .anchor_size(pos2(label_left, rect.center().y), galley.size())
                .min,
            galley,
            text_color,
        );
        if elided {
            response.on_hover_text(self.label)
        } else {
            response
        }
    }
}
