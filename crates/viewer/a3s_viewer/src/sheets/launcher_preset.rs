//! The sheet comparing an Arma 3 Launcher preset with the mods on this computer.

use a3s_foundation::error::UserFacing as _;
use a3s_tools::preset::{ImportResult, add_imported_group};
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_viewer_context::{Change, ViewerContext};
use egui::{RichText, Ui};

use super::Shown;
use crate::toasts::Toaster;

pub(super) fn show(
    ctx: &egui::Context,
    result: &ImportResult,
    viewer: &ViewerContext,
    toasts: &mut Toaster,
) -> Shown {
    let matched = result.matched.len() + result.matched_dlc.len();
    let action = Dialog::new(
        "a3s_viewer::sheet::launcher_preset",
        format!("Preset {}", result.preset_name),
    )
    .ok_label("Enable matched")
    .ok_enabled(matched > 0)
    .cancel_label("Close")
    .default_button(Some(DialogButton::Ok))
    .show(ctx, |ui| {
        ui.set_max_width(480.0);
        ui.label(format!(
            "{matched} of {} mods are on this computer.",
            result.requested_count + result.requested_dlc_count
        ));
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                section(
                    ui,
                    "Matched",
                    ProgressTone::Success,
                    result
                        .matched
                        .iter()
                        .map(|m| m.local_name.as_deref().unwrap_or(&m.name))
                        .chain(result.matched_dlc.iter().map(|dlc| dlc.name.as_str())),
                );
                section(
                    ui,
                    "Missing",
                    ProgressTone::Danger,
                    result
                        .missing
                        .iter()
                        .map(|m| m.name.as_str())
                        .chain(result.missing_dlc.iter().map(|dlc| dlc.name.as_str())),
                );
            });
    });
    match action {
        DialogAction::None => Shown::Open,
        DialogAction::Ok => {
            enable_matched(viewer, toasts, result);
            Shown::Closed(None)
        }
        DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
    }
}

fn section<'a>(ui: &mut Ui, title: &str, tone: ProgressTone, names: impl Iterator<Item = &'a str>) {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        return;
    }
    let color = theme::palette(ui.ctx()).status_color(tone);
    ui.add_space(6.0);
    ui.label(RichText::new(format!("{title} ({})", names.len())).strong());
    for name in names {
        ui.horizontal(|ui| {
            let icon = if tone == ProgressTone::Success {
                icons::CHECK_CIRCLE
            } else {
                icons::X_CIRCLE
            };
            ui.label(RichText::new(icon).color(color));
            ui.label(name);
        });
    }
}

/// Adds a group named after the preset with the matched mods enabled; Undo removes it.
// Mirrors ui/main/AddonsPanel.java:1049-1100 (without the missing entries)
fn enable_matched(viewer: &ViewerContext, toasts: &mut Toaster, result: &ImportResult) {
    let matched = ImportResult {
        missing: Vec::new(),
        missing_dlc: Vec::new(),
        ..result.clone()
    };
    let mut added = None;
    let edited = viewer.mutate(Some(Change::AddonSelection), |state| {
        state.client.update_active_profile(|profile| {
            added = Some((
                profile.name.clone(),
                add_imported_group(&mut profile.tree, &matched),
            ));
            true
        })
    });
    match (edited, added) {
        (Ok(_), Some((profile, group))) => {
            toasts.undoable(format!("Added the group {group}"), move |viewer| {
                let removed = viewer.mutate(Some(Change::AddonSelection), |state| {
                    state.client.update_active_profile(|active| {
                        if active.name != profile {
                            return false;
                        }
                        let index = active
                            .tree
                            .children
                            .iter()
                            .position(|node| !node.is_leaf() && node.name() == group);
                        index.is_some_and(|index| active.tree.remove_child(index).is_some())
                    })
                });
                if let Err(err) = removed {
                    tracing::warn!("Undo of the preset group failed: {}", err.user_message());
                }
            });
        }
        (Err(err), _) => toasts.error(err.user_message()),
        (Ok(_), None) => {}
    }
}
