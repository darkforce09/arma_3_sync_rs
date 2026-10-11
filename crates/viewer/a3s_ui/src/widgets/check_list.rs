//! A vertical list of check boxes. Ports `ui/CheckBoxList.java`.

use egui::AsId;

use egui::{Align2, Id, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, pos2, vec2};

use super::check_tree::{CHECKBOX_HOTSPOT_WIDTH, CheckState, paint_checkbox};
use crate::theme;

/// One entry of a [`CheckList`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckItem {
    /// Text of the entry.
    pub label: String,
    /// Whether the entry is checked.
    pub checked: bool,
}

impl CheckItem {
    /// A new item.
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
        }
    }
}

/// Labels of the checked items, in list order. Mirrors `ui/CheckBoxList.java:40-51`.
pub fn checked_labels(items: &[CheckItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.checked)
        .map(|item| item.label.clone())
        .collect()
}

/// Indexes of the checked items, in list order. Mirrors `ui/CheckBoxList.java:53-63`.
pub fn checked_indexes(items: &[CheckItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.checked)
        .map(|(index, _)| index)
        .collect()
}

/// A check list with single selection. Keep it across frames: it holds the selected row.
#[derive(Clone, Debug)]
pub struct CheckList {
    id: Id,
    selected: Option<usize>,
}

impl CheckList {
    /// A new list with ids derived from `id_salt`.
    pub fn new(id_salt: impl AsId) -> Self {
        Self {
            id: Id::new(id_salt),
            selected: None,
        }
    }

    /// The index of the last pressed row.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Shows the list. A primary click anywhere on a row toggles it and selects it
    /// (`ui/CheckBoxList.java:24-37`; Java reacts to any mouse button). The returned
    /// response is marked changed when an item was toggled.
    pub fn show(&mut self, ui: &mut Ui, items: &mut [CheckItem]) -> Response {
        let def = theme::current(ui.ctx());
        let row_height = ui.spacing().interact_size.y;
        let width = ui.available_width();
        let mut list_response: Option<Response> = None;

        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, item) in items.iter_mut().enumerate() {
                let (rect, _) = ui.allocate_exact_size(vec2(width, row_height), Sense::hover());
                let mut row = ui.interact(rect, self.id.with(index), Sense::click());
                if row.clicked() {
                    item.checked = !item.checked;
                    self.selected = Some(index);
                    row.mark_changed();
                }
                let checked = item.checked;
                let label = item.label.clone();
                row.widget_info(|| {
                    WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), checked, &label)
                });

                if ui.is_rect_visible(rect) {
                    let selected = self.selected == Some(index);
                    let painter = ui.painter();
                    let text_color = if selected {
                        painter.rect_filled(rect, 0.0, def.palette.state_selection_bg);
                        def.palette.state_selection_fg
                    } else {
                        def.palette.text_primary
                    };
                    let box_rect = Rect::from_center_size(
                        pos2(rect.left() + CHECKBOX_HOTSPOT_WIDTH / 2.0, rect.center().y),
                        vec2(15.0, 15.0),
                    );
                    paint_checkbox(
                        painter,
                        box_rect,
                        CheckState::from_selected(checked),
                        row.hovered(),
                        def,
                    );
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let galley = painter.layout_no_wrap(label, font, text_color);
                    let pos = Align2::LEFT_CENTER.anchor_size(
                        pos2(rect.left() + CHECKBOX_HOTSPOT_WIDTH + 2.0, rect.center().y),
                        galley.size(),
                    );
                    painter.galley(pos.min, galley, text_color);
                }
                list_response = Some(match list_response.take() {
                    Some(previous) => previous | row,
                    None => row,
                });
            }
        });
        list_response.unwrap_or_else(|| ui.allocate_response(vec2(width, 0.0), Sense::hover()))
    }
}
