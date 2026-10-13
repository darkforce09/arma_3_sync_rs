//! The sheet shown when quitting needs a decision.

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};

use super::{SheetOutcome, Shown};

/// Why quitting asks first.
pub(crate) enum ExitPrompt {
    /// These tasks run and quitting stops them.
    TasksRunning(Vec<String>),

    /// The settings could not be written on exit; the message says why.
    SaveFailed(String),
}

pub(super) fn show(ctx: &egui::Context, prompt: &ExitPrompt) -> Shown {
    match prompt {
        ExitPrompt::TasksRunning(tasks) => {
            let action = Dialog::new("a3s_viewer::sheet::exit", "Quit Arma3Sync?")
                .ok_label("Quit")
                .default_button(Some(DialogButton::Cancel))
                .show(ctx, |ui| {
                    ui.label("Quitting stops what is still running:");
                    for task in tasks {
                        ui.label(format!("• {task}"));
                    }
                });
            match action {
                DialogAction::None => Shown::Open,
                DialogAction::Ok => Shown::Closed(Some(SheetOutcome::QuitConfirmed)),
                DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
            }
        }
        ExitPrompt::SaveFailed(message) => {
            let action = Dialog::new("a3s_viewer::sheet::save_failed", "Settings not saved")
                .ok_label("Quit anyway")
                .cancel_label("Stay")
                .default_button(Some(DialogButton::Cancel))
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(message.as_str());
                    ui.label("Quit anyway and lose the changes made since start-up?");
                });
            match action {
                DialogAction::None => Shown::Open,
                DialogAction::Ok => Shown::Closed(Some(SheetOutcome::QuitWithoutSaving)),
                DialogAction::Cancel | DialogAction::Close => {
                    Shown::Closed(Some(SheetOutcome::Stay))
                }
            }
        }
    }
}
