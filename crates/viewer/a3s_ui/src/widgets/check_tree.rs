//! A virtualised tri-state check tree, generic over a model.
//!
//! Ports the row rendering of `ui/main/tree/CheckTreeCellRenderer.java`,
//! `ui/repository/tree/CheckTreeCellRendererRepository.java`, `IndeterminateIcon.java`,
//! `MyRenderer.java` / `MyRendererRepository.java` (icon defaults), and the checkbox hotspot and
//! Space toggling of `ui/main/AddonsPanel.java:503-529` and `ui/repository/DownloadPanel.java:580-603`.
//!
//! The widget never mutates the model: it reports [`CheckTreeEvent`]s and the caller applies
//! them (selection propagation is caller logic, `ui/main/AddonsPanel.java:877-918`).

use std::hash::Hash;

use egui::AsId;

use egui::{
    Align2, Color32, Context, CornerRadius, EventFilter, Id, Key, Modifiers, Painter,
    PointerButton, Pos2, Rect, ScrollArea, Sense, Shape, Stroke, StrokeKind, Ui, WidgetInfo,
    WidgetType, pos2, vec2,
};

use crate::icon::{self, IconState, UiIcon};
use crate::theme::{self, ThemeDefinition};

/// Java tree icon size (`ui/main/tree/MyRenderer.java:15`); 18 pt after scaling.
pub const TREE_ICON_SIZE: u32 = 12;

/// Side of the painted check box (`FlatLaf` `CheckBox.icon` is 15×15).
const CHECKBOX_SIZE: f32 = 15.0;

/// Width of the checkbox cell, i.e. the click hotspot (`new JCheckBox().getPreferredSize().width`).
pub const CHECKBOX_HOTSPOT_WIDTH: f32 = 22.0;

/// Width of the expand/collapse handle column.
const EXPANDER_WIDTH: f32 = 16.0;

/// Gap between the icon and the label.
const ICON_GAP: f32 = 4.0;

/// What a row's checkbox shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckState {
    /// Not selected.
    Unchecked,
    /// Selected.
    Checked,
    /// A directory with some, but not all, direct children selected.
    Indeterminate,
}

impl CheckState {
    /// The tri-state rule of `ui/main/tree/CheckTreeCellRenderer.java:48-70`.
    ///
    /// A directory with a parent (never the root) whose **direct** children are partly
    /// unselected is [`CheckState::Indeterminate`], overriding `selected`. Leaves and the root
    /// show `selected`. (Java's `nbSelectedNodes` actually counts unselected children.)
    pub fn for_node(
        selected: bool,
        is_leaf: bool,
        has_parent: bool,
        children_selected: impl IntoIterator<Item = bool>,
    ) -> Self {
        if !is_leaf && has_parent {
            let (children, unselected) = children_selected
                .into_iter()
                .fold((0_usize, 0_usize), |(n, u), sel| {
                    (n + 1, u + usize::from(!sel))
                });
            if children != 0 && unselected != 0 && unselected < children {
                return Self::Indeterminate;
            }
        }
        Self::from_selected(selected)
    }

    /// [`CheckState::Checked`] or [`CheckState::Unchecked`].
    pub fn from_selected(selected: bool) -> Self {
        if selected {
            Self::Checked
        } else {
            Self::Unchecked
        }
    }
}

/// The default tree icon: Package for leaves, `FolderOpen` / Folder for directories.
///
/// Mirrors `ui/main/tree/MyRenderer.java:32-44`.
pub fn default_tree_icon(is_leaf: bool, expanded: bool) -> UiIcon {
    if is_leaf {
        UiIcon::Package
    } else if expanded {
        UiIcon::FolderOpen
    } else {
        UiIcon::Folder
    }
}

/// The data behind a [`CheckTree`].
pub trait CheckTreeModel {
    /// Node key; must be unique within the tree.
    type Key: Clone + Eq + Hash + std::fmt::Debug;

    /// Top-level nodes.
    fn roots(&self) -> Vec<Self::Key>;

    /// Direct children of `key`, in display order.
    fn children(&self, key: &Self::Key) -> Vec<Self::Key>;

    /// Whether `key` is a leaf. An empty directory is not a leaf.
    fn is_leaf(&self, key: &Self::Key) -> bool;

    /// Row text.
    fn label(&self, key: &Self::Key) -> String;

    /// Checkbox state, usually via [`CheckState::for_node`].
    fn check_state(&self, key: &Self::Key) -> CheckState;

    /// Label colour override, e.g. [`crate::consts::JAVA_RED`] for missing addons.
    fn label_color(&self, _key: &Self::Key) -> Option<Color32> {
        None
    }

    /// Row icon; defaults to [`default_tree_icon`]. The sync tree overrides this with the
    /// warning/package rule of `ui/repository/tree/MyRendererRepository.java:34-59`.
    fn icon(&self, key: &Self::Key, expanded: bool) -> Option<UiIcon> {
        Some(default_tree_icon(self.is_leaf(key), expanded))
    }
}

/// What happened in a [`CheckTree`] this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckTreeEvent<K> {
    /// The checkbox hotspot was clicked, or Space was pressed on the selected row.
    Toggled(K),
    /// The row was clicked outside the checkbox hotspot; it is now the selected row.
    RowClicked(K),
    /// The row was double-clicked.
    RowDoubleClicked(K),
    /// The row was secondary-clicked at the given pointer position; it is now the selected row.
    /// The caller opens its menu there.
    ContextMenu(K, Pos2),
    /// A key without built-in meaning was pressed on the selected row while the tree had focus.
    /// Currently [`Key::Delete`] (`ui/main/AddonsPanel.java:524`).
    Key(K, Key),
    /// The row was expanded (`true`) or collapsed (`false`).
    Expanded(K, bool),
}

struct VisibleRow<K> {
    key: K,
    depth: usize,
    is_leaf: bool,
    expanded: bool,
}

/// A tri-state check tree. Keep it across frames: it holds the selected row.
#[derive(Clone, Debug)]
pub struct CheckTree {
    id: Id,
    selected: Option<Id>,
    default_expanded: bool,
    max_height: Option<f32>,
}

impl CheckTree {
    /// A tree whose expansion state is stored under `id_salt`.
    pub fn new(id_salt: impl AsId) -> Self {
        Self {
            id: Id::new(id_salt),
            selected: None,
            default_expanded: false,
            max_height: None,
        }
    }

    /// Whether directories without a stored expansion state start expanded (default `false`).
    pub fn default_expanded(mut self, expanded: bool) -> Self {
        self.default_expanded = expanded;
        self
    }

    /// Limits the height of the scroll area; the default is the available height.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// The egui id of the tree.
    pub fn id(&self) -> Id {
        self.id
    }

    fn focus_id(&self) -> Id {
        self.id.with("focus")
    }

    /// Whether the tree has keyboard focus (it gains it when a row is clicked).
    pub fn has_focus(&self, ctx: &Context) -> bool {
        ctx.memory(|mem| mem.has_focus(self.focus_id()))
    }

    fn row_id(tree: Id, key: &impl AsId) -> Id {
        tree.with(("row", key))
    }

    fn expanded_id(tree: Id, key: &impl AsId) -> Id {
        tree.with(("expanded", key))
    }

    /// Selects the row of `key`.
    pub fn select<K: AsId>(&mut self, key: &K) {
        self.selected = Some(Self::row_id(self.id, key));
    }

    /// Clears the row selection.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Whether `key` is the selected row.
    pub fn is_selected<K: AsId>(&self, key: &K) -> bool {
        self.selected == Some(Self::row_id(self.id, key))
    }

    /// Stores the expansion state of `key` in the tree created with `id_salt`.
    pub fn set_expanded<K: AsId>(ctx: &Context, id_salt: impl AsId, key: &K, open: bool) {
        let id = Self::expanded_id(Id::new(id_salt), key);
        ctx.data_mut(|data| data.insert_temp(id, open));
    }

    /// Expands every key of `keys` (the "Expand All" checkbox).
    pub fn expand_all<'k, K: AsId + 'k>(
        ctx: &Context,
        id_salt: impl AsId,
        keys: impl IntoIterator<Item = &'k K>,
    ) {
        let tree = Id::new(id_salt);
        ctx.data_mut(|data| {
            for key in keys {
                data.insert_temp(Self::expanded_id(tree, key), true);
            }
        });
    }

    /// Whether `key` is expanded in this tree.
    pub fn is_expanded<K: AsId>(&self, ctx: &Context, key: &K) -> bool {
        let id = Self::expanded_id(self.id, key);
        ctx.data(|data| data.get_temp::<bool>(id))
            .unwrap_or(self.default_expanded)
    }

    fn flatten<M: CheckTreeModel>(&self, ctx: &Context, model: &M) -> Vec<VisibleRow<M::Key>> {
        let mut rows = Vec::new();
        let mut stack: Vec<(M::Key, usize)> =
            model.roots().into_iter().rev().map(|k| (k, 0)).collect();
        while let Some((key, depth)) = stack.pop() {
            let is_leaf = model.is_leaf(&key);
            let expanded = !is_leaf && self.is_expanded(ctx, &key);
            if expanded {
                stack.extend(
                    model
                        .children(&key)
                        .into_iter()
                        .rev()
                        .map(|k| (k, depth + 1)),
                );
            }
            rows.push(VisibleRow {
                key,
                depth,
                is_leaf,
                expanded,
            });
        }
        rows
    }

    /// Shows the tree and returns this frame's events.
    pub fn show<M: CheckTreeModel>(
        &mut self,
        ui: &mut Ui,
        model: &M,
    ) -> Vec<CheckTreeEvent<M::Key>> {
        let mut events = Vec::new();
        let rows = self.flatten(ui.ctx(), model);
        let row_height = ui.spacing().interact_size.y;
        let focus_id = self.focus_id();
        let mut wants_focus = false;
        let mut scroll_to_selected = false;

        if self.has_focus(ui.ctx()) {
            // Keep Up/Down for the selection instead of egui's directional focus moves.
            ui.memory_mut(|mem| {
                mem.set_focus_lock_filter(
                    focus_id,
                    EventFilter {
                        vertical_arrows: true,
                        ..EventFilter::default()
                    },
                );
            });
            scroll_to_selected = self.keyboard(ui, &rows, &mut events);
        }

        let mut scroll = ScrollArea::vertical()
            .id_salt(self.id.with("scroll"))
            .auto_shrink([false, true]);
        if let Some(max_height) = self.max_height {
            scroll = scroll.max_height(max_height);
        }

        let tree_rect = ui
            .scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                scroll
                    .show_rows(ui, row_height, rows.len(), |ui, range| {
                        for row in &rows[range] {
                            let rect = self.row_ui(
                                ui,
                                model,
                                row,
                                row_height,
                                &mut wants_focus,
                                &mut events,
                            );
                            if scroll_to_selected && self.is_selected(&row.key) {
                                ui.scroll_to_rect(rect, None);
                            }
                        }
                    })
                    .inner_rect
            })
            .inner;

        // The tree keeps keyboard focus through a focusable, non-clickable widget; rows are not
        // focusable themselves, so Tab moves over the tree as one stop.
        let _focus = ui.interact(tree_rect, focus_id, Sense::FOCUSABLE);
        // Requested after the focus widget is registered: registering it during the click frame
        // would surrender the focus again ("clicked elsewhere").
        if wants_focus {
            ui.memory_mut(|mem| mem.request_focus(focus_id));
        }
        events
    }

    /// Space toggles the selected row, Delete reports it, Up/Down move the selection
    /// (`ui/main/AddonsPanel.java:522-529`, `ui/repository/DownloadPanel.java:603`). Returns
    /// whether the selection moved.
    fn keyboard<K: Clone + Eq + Hash + std::fmt::Debug>(
        &mut self,
        ui: &Ui,
        rows: &[VisibleRow<K>],
        events: &mut Vec<CheckTreeEvent<K>>,
    ) -> bool {
        let index = rows
            .iter()
            .position(|row| Some(Self::row_id(self.id, &row.key)) == self.selected);
        let (space, delete, up, down) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Space),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
            )
        });
        if let Some(i) = index {
            if space {
                events.push(CheckTreeEvent::Toggled(rows[i].key.clone()));
            }
            if delete {
                events.push(CheckTreeEvent::Key(rows[i].key.clone(), Key::Delete));
            }
        }
        let target = match (index, up, down) {
            (Some(i), true, false) => i.checked_sub(1),
            (Some(i), false, true) => (i + 1 < rows.len()).then_some(i + 1),
            // Like JTree: the first arrow press without a selection selects the first row.
            (None, true, false) | (None, false, true) => (!rows.is_empty()).then_some(0),
            _ => None,
        };
        // Moving the selection emits no event, like a JTree selection without listeners; read
        // it with `is_selected`.
        if let Some(t) = target {
            self.selected = Some(Self::row_id(self.id, &rows[t].key));
            return true;
        }
        false
    }

    fn row_ui<M: CheckTreeModel>(
        &mut self,
        ui: &mut Ui,
        model: &M,
        row: &VisibleRow<M::Key>,
        row_height: f32,
        wants_focus: &mut bool,
        events: &mut Vec<CheckTreeEvent<M::Key>>,
    ) -> Rect {
        let def = theme::current(ui.ctx());
        let key = &row.key;
        let row_id = Self::row_id(self.id, key);
        let label = model.label(key);
        let state = model.check_state(key);

        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::hover());
        let indent = ui.spacing().indent * row.depth as f32;
        let expander_rect = Rect::from_min_size(
            pos2(rect.left() + indent, rect.top()),
            vec2(EXPANDER_WIDTH, row_height),
        );
        // The checkbox hotspot starts at the row's indented left edge (JTree path bounds).
        let hotspot = Rect::from_min_size(
            pos2(expander_rect.right(), rect.top()),
            vec2(CHECKBOX_HOTSPOT_WIDTH, row_height),
        );

        // Registered first so the expander and checkbox, registered later, win the hit test.
        let row_response = ui.interact(rect, row_id, Sense::CLICK);
        row_response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::SelectableLabel,
                ui.is_enabled(),
                self.selected == Some(row_id),
                &label,
            )
        });

        let expander = if row.is_leaf {
            None
        } else {
            let response = ui.interact(expander_rect, row_id.with("expander"), Sense::CLICK);
            response.widget_info(|| {
                WidgetInfo::selected(
                    WidgetType::Button,
                    ui.is_enabled(),
                    row.expanded,
                    format!("Toggle {label}"),
                )
            });
            Some(response)
        };

        let check_response = ui.interact(hotspot, row_id.with("check"), Sense::CLICK);
        check_response.widget_info(|| match state {
            CheckState::Indeterminate => {
                WidgetInfo::labeled(WidgetType::Checkbox, ui.is_enabled(), &label)
            }
            CheckState::Checked | CheckState::Unchecked => WidgetInfo::selected(
                WidgetType::Checkbox,
                ui.is_enabled(),
                state == CheckState::Checked,
                &label,
            ),
        });

        if let Some(expander) = &expander
            && expander.clicked()
        {
            let open = !row.expanded;
            Self::set_expanded_id(ui.ctx(), Self::expanded_id(self.id, key), open);
            events.push(CheckTreeEvent::Expanded(key.clone(), open));
        }
        if check_response.clicked() {
            self.selected = Some(row_id);
            *wants_focus = true;
            events.push(CheckTreeEvent::Toggled(key.clone()));
        }
        if row_response.clicked() {
            self.selected = Some(row_id);
            *wants_focus = true;
            events.push(CheckTreeEvent::RowClicked(key.clone()));
        }
        if row_response.double_clicked() {
            events.push(CheckTreeEvent::RowDoubleClicked(key.clone()));
        }
        if row_response.clicked_by(PointerButton::Secondary)
            || check_response.clicked_by(PointerButton::Secondary)
        {
            self.selected = Some(row_id);
            *wants_focus = true;
            let pos = row_response
                .interact_pointer_pos()
                .or_else(|| check_response.interact_pointer_pos())
                .unwrap_or_else(|| rect.center());
            events.push(CheckTreeEvent::ContextMenu(key.clone(), pos));
        }

        if !ui.is_rect_visible(rect) {
            return rect;
        }
        let painter = ui.painter();
        let selected = self.selected == Some(row_id);
        let content_left = hotspot.right();
        let text_color = model.label_color(key).unwrap_or(if selected {
            def.palette.state_selection_fg
        } else {
            def.palette.text_primary
        });

        let font = egui::TextStyle::Body.resolve(ui.style());
        let galley = painter.layout_no_wrap(label, font, text_color);
        let icon_size = icon::scaled_size(TREE_ICON_SIZE) as f32;
        let icon = model.icon(key, row.expanded);
        let icon_width = if icon.is_some() {
            icon_size + ICON_GAP
        } else {
            0.0
        };

        if selected {
            // Swing paints the selection behind the renderer component (icon + label).
            let highlight = Rect::from_min_max(
                pos2(content_left, rect.top()),
                pos2(
                    content_left + icon_width + galley.size().x + ICON_GAP,
                    rect.bottom(),
                ),
            );
            painter.rect_filled(highlight, 0.0, ui.visuals().selection.bg_fill);
        }

        if expander.is_some() {
            paint_expander(
                painter,
                expander_rect,
                row.expanded,
                def.palette.text_secondary,
            );
        }
        let box_rect = Rect::from_center_size(
            pos2(
                hotspot.left() + CHECKBOX_HOTSPOT_WIDTH / 2.0,
                hotspot.center().y,
            ),
            vec2(CHECKBOX_SIZE, CHECKBOX_SIZE),
        );
        paint_checkbox(painter, box_rect, state, check_response.hovered(), def);

        let mut x = content_left;
        if let Some(icon) = icon {
            let icon_rect = Rect::from_min_size(
                pos2(x, rect.center().y - icon_size / 2.0),
                vec2(icon_size, icon_size),
            );
            icon::icon_image(ui.ctx(), icon, TREE_ICON_SIZE, IconState::Normal)
                .paint_at(ui, icon_rect);
            x += icon_width;
        }
        let text_pos = Align2::LEFT_CENTER.anchor_size(pos2(x, rect.center().y), galley.size());
        painter.galley(text_pos.min, galley, text_color);
        rect
    }

    fn set_expanded_id(ctx: &Context, id: Id, open: bool) {
        ctx.data_mut(|data| data.insert_temp(id, open));
    }
}

fn paint_expander(painter: &Painter, rect: Rect, expanded: bool, color: Color32) {
    let c = rect.center();
    let s = 3.5;
    let points = if expanded {
        vec![
            pos2(c.x - s, c.y - s / 2.0),
            pos2(c.x + s, c.y - s / 2.0),
            pos2(c.x, c.y + s / 2.0 + 1.0),
        ]
    } else {
        vec![
            pos2(c.x - s / 2.0, c.y - s),
            pos2(c.x + s / 2.0 + 1.0, c.y),
            pos2(c.x - s / 2.0, c.y + s),
        ]
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Paints a `FlatLaf`-like check box. The indeterminate state paints the unchecked box plus the
/// bar of `ui/main/tree/IndeterminateIcon.java:17-26`.
///
/// D-13: the bar uses `accentPrimary` instead of Java's hard-coded rgba(50, 20, 255, 200), and
/// the unchecked border uses `textDisabled` (`stateFocus` while hovered).
pub(crate) fn paint_checkbox(
    painter: &Painter,
    rect: Rect,
    state: CheckState,
    hovered: bool,
    def: &ThemeDefinition,
) {
    let p = &def.palette;
    let radius = CornerRadius::same(2);
    match state {
        CheckState::Checked => {
            painter.rect_filled(rect, radius, p.accent_primary);
            let r = rect.shrink(3.5);
            painter.add(Shape::line(
                vec![
                    pos2(r.left(), r.center().y),
                    pos2(r.left() + r.width() * 0.4, r.bottom() - 0.5),
                    pos2(r.right(), r.top() + 0.5),
                ],
                Stroke::new(2.0, p.surface),
            ));
        }
        CheckState::Unchecked | CheckState::Indeterminate => {
            let border = if hovered {
                p.state_focus
            } else {
                p.text_disabled
            };
            painter.rect(
                rect,
                radius,
                p.surface,
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
            if state == CheckState::Indeterminate {
                // fillRect(a, (h - b) / 2, w - a - a, b) with a = 4, b = 2.
                let (a, b) = (4.0, 2.0);
                let y = rect.top() + ((rect.height() - b) / 2.0).floor();
                let bar =
                    Rect::from_min_size(pos2(rect.left() + a, y), vec2(rect.width() - a - a, b));
                painter.rect_filled(bar, 0.0, p.accent_primary);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_icons() {
        assert_eq!(default_tree_icon(true, false), UiIcon::Package);
        assert_eq!(default_tree_icon(false, true), UiIcon::FolderOpen);
        assert_eq!(default_tree_icon(false, false), UiIcon::Folder);
    }
}
