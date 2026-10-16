//! The sheet shown when an update starts while Arma 3 runs: files the game has open may fail to
//! update, so it asks first.
// Mirrors ui/repository/DownloadPanel.java:1005-1014

use a3s_foundation::error::UserFacing as _;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_viewer_context::sync::{self, Selection};
use a3s_viewer_context::{Command, ViewerContext};

use super::Shown;
use crate::toasts::Toaster;

/// The download waiting for the decision.
pub(crate) struct GameRunning {
    repository: String,
    event: Option<String>,
    selection: Selection,
}

impl GameRunning {
    pub(crate) fn new(repository: String, event: Option<String>, selection: Selection) -> Self {
        Self {
            repository,
            event,
            selection,
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        let action = Dialog::new("a3s_viewer::sheet::game_running", "Arma 3 is running")
            .ok_label("Update anyway")
            .default_button(Some(DialogButton::Cancel))
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(format!(
                    "Updating {} while the game runs can fail for the files it has open. Quit \
                     the game first, or update anyway.",
                    self.repository
                ));
            });
        match action {
            DialogAction::None => Shown::Open,
            DialogAction::Ok => {
                let command = Command::Sync(sync::Command::DownloadAnyway {
                    repository: self.repository.clone(),
                    event: self.event.take(),
                    selection: std::mem::take(&mut self.selection),
                });
                if let Err(err) = viewer.send(command) {
                    toasts.error(err.user_message());
                }
                Shown::Closed(None)
            }
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
        }
    }
}
