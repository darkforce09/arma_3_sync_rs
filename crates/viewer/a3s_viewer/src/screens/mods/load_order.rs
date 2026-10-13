//! The Load order segment: the enabled addons in the order the game loads them, reordered by
//! dragging.

use a3s_foundation::format;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_viewer_context::profiles::AddonScan;
use egui::{Align, Layout, RichText, Sense, Ui, vec2};

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LoadOrderRow {
    /// The lowercase registry key.
    pub(crate) key: String,

    /// The folder name.
    pub(crate) name: String,

    /// The folder it is in, or that it is missing.
    pub(crate) detail: String,

    /// Bytes on disk.
    pub(crate) size: u64,
}

/// The rows of `order` (lowercase keys) with their names from the scan.
pub(crate) fn rows(order: &[String], scan: Option<&AddonScan>) -> Vec<LoadOrderRow> {
    order
        .iter()
        .map(|key| {
            let addon = scan.and_then(|scan| scan.registry.get(key));
            LoadOrderRow {
                key: key.clone(),
                name: addon.map_or_else(|| key.clone(), |addon| addon.name.clone()),
                detail: addon.map_or_else(
                    || "Not on this computer".to_owned(),
                    |addon| super::model::folder_name(&addon.path),
                ),
                size: scan.map_or(0, |scan| scan.size(key)),
            }
        })
        .collect()
}

/// Shows `rows` as a draggable list; returns the new order of all rows after a drop that moved
/// one. With a search text, only matching rows show and dragging is off.
pub(crate) fn show(ui: &mut Ui, rows: &[LoadOrderRow], search: &str) -> Option<Vec<String>> {
    let palette = theme::palette(ui.ctx());
    if rows.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No mods enabled").color(palette.text_secondary));
        });
        return None;
    }
    let search = search.trim().to_lowercase();
    let row_height = 34.0;
    let mut moved = None;
    egui::ScrollArea::vertical()
        .id_salt("a3s_viewer::mods::load_order")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if !search.is_empty() {
                for (index, row) in rows.iter().enumerate() {
                    if row.name.to_lowercase().contains(&search) {
                        row_ui(ui, index, row, row_height);
                    }
                }
                return;
            }
            let response = egui_dnd::dnd(ui, "a3s_viewer::mods::load_order::dnd").show(
                rows.iter(),
                |ui, row, handle, state| {
                    ui.horizontal(|ui| {
                        handle.ui(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(icons::DOTS_SIX_VERTICAL)
                                        .color(palette.text_secondary),
                                )
                                .sense(Sense::hover()),
                            )
                            .on_hover_text(format!("Drag {}", row.name));
                        });
                        row_ui(ui, state.index, row, row_height);
                    });
                },
            );
            if let Some(update) = response.final_update()
                && update.from != update.to
            {
                let mut keys: Vec<String> = rows.iter().map(|row| row.key.clone()).collect();
                egui_dnd::utils::shift_vec(update.from, update.to, &mut keys);
                moved = Some(keys);
            }
        });
    moved
}

fn row_ui(ui: &mut Ui, index: usize, row: &LoadOrderRow, height: f32) {
    let palette = theme::palette(ui.ctx());
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(
                RichText::new(format!("{:>2}", index + 1))
                    .monospace()
                    .color(palette.text_secondary),
            );
            ui.label(&row.name);
            ui.label(
                RichText::new(&row.detail)
                    .small()
                    .color(palette.text_secondary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if row.size > 0 {
                    ui.label(
                        RichText::new(format::size(row.size as i64))
                            .small()
                            .color(palette.text_secondary),
                    );
                }
            });
        },
    );
}
