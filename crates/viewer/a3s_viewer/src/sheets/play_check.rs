//! The sheet Play shows before starting when the enabled mods are not ready: some are enabled
//! twice (the game would load them twice, so it does not start), or some are not on this
//! computer (download them first, start without them, or cancel).
// Mirrors ui/main/LaunchPanel.java:276-297

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::widgets::button;
use egui::Label;

use super::{SheetOutcome, Shown};

/// What is wrong with the enabled mods.
pub(crate) enum PlayCheck {
    /// These mods are enabled in two groups.
    EnabledTwice(Vec<String>),

    /// These mods are not on this computer; `downloadable` when a repository of the profile's
    /// groups has them.
    Missing {
        names: Vec<String>,
        downloadable: bool,
    },
}

impl PlayCheck {
    pub(super) fn show(&self, ctx: &egui::Context) -> Shown {
        match self {
            Self::EnabledTwice(names) => {
                let action = Dialog::new("a3s_viewer::sheet::play_check", "Mods enabled twice")
                    .show_ok(false)
                    .cancel_label("OK")
                    .default_button(Some(DialogButton::Cancel))
                    .show(ctx, |ui| {
                        ui.set_max_width(420.0);
                        ui.add(
                            Label::new(
                                "These mods are enabled in two groups, so the game would load \
                                 them twice. Disable them in one group on the Mods screen.",
                            )
                            .wrap(),
                        );
                        list(ui, names);
                    });
                closed_unless_open(action, None)
            }
            Self::Missing {
                names,
                downloadable,
            } => {
                let mut play_without = false;
                let ok = if *downloadable {
                    "Update and play"
                } else {
                    "Play without them"
                };
                let action = Dialog::new("a3s_viewer::sheet::play_check", "Mods are missing")
                    .ok_label(ok)
                    .default_button(Some(DialogButton::Ok))
                    .show(ctx, |ui| {
                        ui.set_max_width(420.0);
                        ui.label("These enabled mods are not on this computer:");
                        list(ui, names);
                        if *downloadable {
                            ui.add_space(6.0);
                            play_without = button(ui, "Play without them").clicked();
                        }
                    });
                if play_without {
                    return Shown::Closed(Some(SheetOutcome::PlayChecked {
                        update_first: false,
                    }));
                }
                let outcome = SheetOutcome::PlayChecked {
                    update_first: *downloadable,
                };
                closed_unless_open(action, Some(outcome))
            }
        }
    }
}

fn list(ui: &mut egui::Ui, names: &[String]) {
    ui.add_space(4.0);
    for name in names {
        ui.label(format!("• {name}"));
    }
}

/// The sheet closes on any button; OK asks for `ok`.
fn closed_unless_open(action: DialogAction, ok: Option<SheetOutcome>) -> Shown {
    match action {
        DialogAction::None => Shown::Open,
        DialogAction::Ok => Shown::Closed(ok),
        DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
    }
}
