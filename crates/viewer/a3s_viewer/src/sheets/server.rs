//! The favourite server sheet: name, address, port, password and the addon group (preset) to
//! load, for a new server or an existing one.

use a3s_repo_files::services::FavoriteServerEdit;
use a3s_types::FavoriteServer;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::popup_button;
use egui::{RichText, Ui};

use crate::fields::{labelled_row, text_row};

/// What the sheet edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ServerTarget {
    /// A new server.
    New,

    /// The favourite server at this index, which had this name.
    Existing { index: usize, name: Option<String> },
}

/// The sheet's fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerSheet {
    pub(crate) target: ServerTarget,
    name: String,
    address: String,
    port: String,
    password: String,
    modset: Option<String>,
}

/// What the user did with the sheet this frame.
pub(crate) enum ServerSheetAction {
    /// Still open.
    Open,

    /// Closed without saving.
    Cancelled,

    /// Saved with these values.
    Saved(FavoriteServerEdit),
}

impl ServerSheet {
    /// A sheet for a new server on the default game port.
    pub(crate) fn new_server() -> Self {
        Self {
            target: ServerTarget::New,
            name: String::new(),
            address: String::new(),
            port: "2302".to_owned(),
            password: String::new(),
            modset: None,
        }
    }

    /// A sheet editing the server at `index`.
    pub(crate) fn edit(index: usize, server: &FavoriteServer) -> Self {
        Self {
            target: ServerTarget::Existing {
                index,
                name: server.name.clone(),
            },
            name: server.name.clone().unwrap_or_default(),
            address: server.ip_address.clone().unwrap_or_default(),
            port: server.port.to_string(),
            password: server.password.clone().unwrap_or_default(),
            modset: server.modset_name.clone(),
        }
    }

    /// Shows the sheet; `modsets` are the addon groups the preset can name.
    pub(crate) fn show(&mut self, ctx: &egui::Context, modsets: &[String]) -> ServerSheetAction {
        let (title, ok) = match self.target {
            ServerTarget::New => ("Add server", "Add"),
            ServerTarget::Existing { .. } => ("Edit server", "Save"),
        };
        let valid = self.validate();
        let action = Dialog::new("a3s_viewer::sheet::server", title)
            .ok_label(ok)
            .ok_enabled(valid.is_ok())
            .default_button(Some(DialogButton::Ok))
            .show(ctx, |ui| self.body(ui, modsets, valid.err()));
        match action {
            DialogAction::None => ServerSheetAction::Open,
            DialogAction::Ok => match self.validate() {
                Ok(port) => ServerSheetAction::Saved(self.edit_values(port)),
                Err(_) => ServerSheetAction::Open,
            },
            DialogAction::Cancel | DialogAction::Close => ServerSheetAction::Cancelled,
        }
    }

    fn body(&mut self, ui: &mut Ui, modsets: &[String], problem: Option<&'static str>) {
        ui.set_width(400.0);
        frames::group(ui, |ui| {
            text_row(ui, "Name", &mut self.name, false);
            frames::group_separator(ui);
            text_row(ui, "Address", &mut self.address, false);
            frames::group_separator(ui);
            text_row(ui, "Port", &mut self.port, false);
            frames::group_separator(ui);
            text_row(ui, "Password", &mut self.password, true);
            frames::group_separator(ui);
            labelled_row(ui, "Preset", |ui| {
                let current = self.modset.clone();
                let mut choices: Vec<(Option<String>, &str)> = vec![(None, "None")];
                choices.extend(modsets.iter().map(|m| (Some(m.clone()), m.as_str())));
                if let Some(current) = &current
                    && !modsets.contains(current)
                {
                    choices.push((Some(current.clone()), current.as_str()));
                }
                popup_button(
                    ui,
                    "a3s_viewer::sheet::server::preset",
                    &mut self.modset,
                    &choices,
                )
            });
        });
        if let Some(problem) = problem {
            ui.add_space(4.0);
            ui.label(RichText::new(problem).color(theme::palette(ui.ctx()).text_secondary));
        }
    }

    /// The port when the fields can be saved, else why not.
    fn validate(&self) -> Result<i32, &'static str> {
        if self.name.trim().is_empty() {
            return Err("Give the server a name.");
        }
        if self.address.trim().is_empty() {
            return Err("Enter the server's address.");
        }
        self.port
            .trim()
            .parse::<u16>()
            .map(i32::from)
            .map_err(|_invalid| "The port is a number from 0 to 65535.")
    }

    fn edit_values(&self, port: i32) -> FavoriteServerEdit {
        let optional = |text: &str| {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        };
        FavoriteServerEdit {
            name: optional(&self.name),
            ip_address: optional(&self.address),
            port,
            password: (!self.password.is_empty()).then(|| self.password.clone()),
            modset_name: self.modset.clone(),
        }
    }
}
