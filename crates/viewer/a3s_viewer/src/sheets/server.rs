//! The favourite server sheet: name, address, port, password and the addon group (preset) to
//! load, for a new server or an existing one. Saving writes at once; an edited joined server
//! stays joined.

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::{ClientData, FavoriteServerEdit, ServiceError};
use a3s_types::FavoriteServer;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::popup_button;
use a3s_viewer_context::{Change, ViewerContext};
use egui::{RichText, Ui};

use super::Shown;
use crate::fields::{labelled_row, text_row};
use crate::toasts::Toaster;

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
    target: ServerTarget,
    name: String,
    address: String,
    port: String,
    password: String,
    modset: Option<String>,
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

    /// Whether the sheet adds a server rather than editing one.
    pub(crate) fn adds(&self) -> bool {
        self.target == ServerTarget::New
    }

    /// Shows the sheet over the active profile's addon groups; saves when it closes with OK.
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        let modsets: Vec<String> = viewer
            .read()
            .client
            .active_profile()
            .map(|profile| {
                profile
                    .tree
                    .directories()
                    .map(|group| group.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let (title, ok) = match self.target {
            ServerTarget::New => ("Add server", "Add"),
            ServerTarget::Existing { .. } => ("Edit server", "Save"),
        };
        let valid = self.validate();
        let action = Dialog::new("a3s_viewer::sheet::server", title)
            .ok_label(ok)
            .ok_enabled(valid.is_ok())
            .default_button(Some(DialogButton::Ok))
            .show(ctx, |ui| self.body(ui, &modsets, valid.err()));
        match action {
            DialogAction::None => Shown::Open,
            DialogAction::Ok => match self.validate() {
                Ok(port) => {
                    save(viewer, toasts, &self.target, self.edit_values(port));
                    Shown::Closed(None)
                }
                Err(_) => Shown::Open,
            },
            DialogAction::Cancel | DialogAction::Close => Shown::Closed(None),
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

/// Makes the server named `name` the one the next launch joins, with `modset` as the default
/// addon group; `None` joins none.
pub(crate) fn set_join(
    client: &mut ClientData,
    name: Option<String>,
    modset: Option<String>,
) -> Result<(), ServiceError> {
    client.update_configuration(|configuration| {
        configuration.server_name = name;
        configuration.default_modset = modset;
        true
    })?;
    Ok(())
}

/// Adds a server or writes an edited one; an edited joined server stays joined.
fn save(
    viewer: &ViewerContext,
    toasts: &mut Toaster,
    target: &ServerTarget,
    edit: FavoriteServerEdit,
) {
    let saved = viewer.mutate(Some(Change::Online), |state| match target {
        ServerTarget::New => state.client.add_favorite_server(FavoriteServer {
            name: edit.name,
            ip_address: edit.ip_address,
            port: edit.port,
            password: edit.password,
            modset_name: edit.modset_name,
            ..FavoriteServer::default()
        }),
        ServerTarget::Existing { index, name } => {
            let joined = state.client.configuration().server_name.as_deref();
            let was_joined = joined.is_some() && name.as_deref() == joined;
            let (new_name, modset) = (edit.name.clone(), edit.modset_name.clone());
            state.client.edit_favorite_server(*index, edit)?;
            if was_joined {
                set_join(&mut state.client, new_name, modset)?;
            }
            Ok(())
        }
    });
    if let Err(err) = saved {
        toasts.error(err.user_message());
    }
}
