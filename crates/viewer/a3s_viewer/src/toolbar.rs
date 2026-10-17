//! The unified toolbar: screen title, search field, the menu, the profile pop-up and the Play
//! button.

use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::widgets::primary_button;
use egui::{Align, Button, Id, Layout, RichText, TextEdit, Ui};

use crate::shell::Action;
use crate::shortcuts;
use crate::snapshot::Snapshot;
use crate::windows::{Appearance, SecondaryWindow, SettingsPane};

/// The id of the search field, for the focus shortcut.
pub(crate) fn search_id() -> Id {
    Id::new("a3s_viewer::search")
}

/// The Play button's label: it says whether updates come first.
pub(crate) fn play_label(update_first: bool) -> String {
    let text = if update_first {
        "Update and play"
    } else {
        "Play"
    };
    format!("{} {text}", icons::PLAY)
}

/// Shows the toolbar; returns the actions the user took.
pub(crate) fn show(
    ui: &mut Ui,
    title: &str,
    snapshot: &Snapshot,
    search: &mut String,
) -> Vec<Action> {
    let mut actions = Vec::new();
    frames::toolbar(ui, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(title).strong().size(15.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, &play_label(snapshot.profile_has_updates)).clicked() {
                actions.push(Action::Play);
            }
            if snapshot.restarting
                && ui
                    .button(format!("{} Stop restarting", icons::STOP))
                    .on_hover_text("Arma 3 is not started again when it closes")
                    .clicked()
            {
                actions.push(Action::StopAutoRestart);
            }
            let mut profile = snapshot.active_profile.clone();
            let mut manage = false;
            egui::ComboBox::from_id_salt("a3s_viewer::profile")
                .selected_text(profile.as_str())
                .show_ui(ui, |ui| {
                    for name in &snapshot.profiles {
                        if ui.selectable_label(*name == profile, name).clicked() {
                            profile = name.clone();
                        }
                    }
                    ui.separator();
                    manage = ui.button("Manage profiles…").clicked();
                })
                .response
                .on_hover_text("Profile");
            if profile != snapshot.active_profile {
                actions.push(Action::SelectProfile(profile));
            }
            if manage {
                actions.push(Action::OpenSettings(SettingsPane::Profiles));
            }
            menu(ui, snapshot, &mut actions);
            ui.add(
                TextEdit::singleline(search)
                    .id(search_id())
                    .hint_text(format!("{} Search", icons::MAGNIFYING_GLASS))
                    .desired_width(200.0),
            );
        });
    });
    actions
}

fn menu(ui: &mut Ui, snapshot: &Snapshot, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    ui.menu_button(RichText::new(icons::DOTS_THREE_CIRCLE).size(18.0), |ui| {
        if ui
            .add(Button::new("Check now").shortcut_text(ctx.format_shortcut(&shortcuts::CHECK_NOW)))
            .clicked()
        {
            actions.push(Action::CheckNow);
        }
        if ui.button("Rescan mods folders").clicked() {
            actions.push(Action::Rescan);
        }
        if ui
            .add(Button::new("Settings…").shortcut_text(ctx.format_shortcut(&shortcuts::SETTINGS)))
            .clicked()
        {
            actions.push(Action::Open(SecondaryWindow::Settings));
        }
        ui.separator();
        for tool in SecondaryWindow::TOOLS {
            if ui.button(format!("{}…", tool.title())).clicked() {
                actions.push(Action::Open(tool));
            }
        }
        ui.separator();
        ui.menu_button("Appearance", |ui| {
            for appearance in Appearance::ALL {
                if ui
                    .radio(snapshot.appearance == appearance, appearance.label())
                    .clicked()
                {
                    actions.push(Action::SetAppearance(appearance));
                }
            }
        });
        if ui.button("About Arma3Sync").clicked() {
            actions.push(Action::About);
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            actions.push(Action::Quit);
        }
    })
    .response
    .on_hover_text("More");
}
