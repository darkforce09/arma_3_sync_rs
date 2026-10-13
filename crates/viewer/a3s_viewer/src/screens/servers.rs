//! The Servers screen: favourite servers. "Join" picks the server the next launch connects to
//! (the configuration's server name, with its preset as the default addon group); add with +,
//! edit with a double-click, remove with Undo.

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::ServiceError;
use a3s_types::FavoriteServer;
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_ui::widgets::{button, icon_text_button, status_pill};
use a3s_viewer_context::{Change, ViewerContext};
use egui::{Align, Layout, RichText, Ui};

use super::ScreenCx;
use crate::sheets::Sheet;
use crate::sheets::server::{ServerSheet, set_join};
use crate::toasts::Toaster;

/// The Servers screen's state.
#[derive(Default)]
pub(crate) struct ServersScreen {
    selected: Option<usize>,
}

/// What the screen shows, copied out of the state.
struct View {
    servers: Vec<FavoriteServer>,
    joined: Option<String>,
}

impl ServersScreen {
    pub(crate) fn show(&mut self, ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        let viewer = cx.viewer;
        let view = {
            let state = viewer.read();
            let client = &state.client;
            View {
                servers: client.favorite_servers().to_vec(),
                joined: client.configuration().server_name.clone(),
            }
        };
        if self
            .selected
            .is_some_and(|index| index >= view.servers.len())
        {
            self.selected = None;
        }

        ui.horizontal(|ui| {
            if icon_text_button(ui, icons::PLUS, "Add server").clicked() {
                cx.sheets
                    .push(Sheet::Server(Box::new(ServerSheet::new_server())));
            }
            if let Some(index) = self.selected
                && icon_text_button(ui, icons::MINUS, "Remove").clicked()
            {
                remove(viewer, cx.toasts, index, &view);
                self.selected = None;
            }
        });
        ui.add_space(8.0);

        if view.servers.is_empty() {
            super::empty_state(
                ui,
                icons::STAR,
                "No favourite servers",
                "Add the servers you play on to join them when the game starts.",
            );
        } else {
            frames::group(ui, |ui| {
                for (index, server) in view.servers.iter().enumerate() {
                    if index > 0 {
                        frames::group_separator(ui);
                    }
                    let joined = is_joined(server, view.joined.as_deref());
                    match self.row(ui, index, server, joined) {
                        RowAction::None => {}
                        RowAction::Select => self.selected = Some(index),
                        RowAction::Edit => {
                            self.selected = Some(index);
                            cx.sheets
                                .push(Sheet::Server(Box::new(ServerSheet::edit(index, server))));
                        }
                        RowAction::Join => join(viewer, cx.toasts, Some(server)),
                        RowAction::Leave => join(viewer, cx.toasts, None),
                    }
                }
            });
        }
    }

    /// One server: the name selects (a double-click edits), the buttons join.
    fn row(&self, ui: &mut Ui, index: usize, server: &FavoriteServer, joined: bool) -> RowAction {
        let palette = theme::palette(ui.ctx());
        let mut action = RowAction::None;
        ui.horizontal(|ui| {
            ui.set_min_height(36.0);
            let star = if joined {
                palette.status_color(ProgressTone::Warning)
            } else {
                palette.text_secondary
            };
            ui.label(RichText::new(icons::STAR).color(star));
            ui.vertical(|ui| {
                let name = server.name.as_deref().unwrap_or("Unnamed server");
                let response =
                    ui.selectable_label(self.selected == Some(index), RichText::new(name).strong());
                if response.double_clicked() {
                    action = RowAction::Edit;
                } else if response.clicked() {
                    action = RowAction::Select;
                }
                let mut details = format!(
                    "{}:{}",
                    server.ip_address.as_deref().unwrap_or("?"),
                    server.port
                );
                if let Some(modset) = &server.modset_name {
                    details.push_str("  ·  ");
                    details.push_str(modset);
                }
                ui.horizontal(|ui| {
                    // Lines up with the name inside its selectable frame.
                    ui.add_space(ui.spacing().button_padding.x);
                    ui.label(RichText::new(details).color(palette.text_secondary));
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if joined {
                    if button(ui, "Don't join").clicked() {
                        action = RowAction::Leave;
                    }
                    status_pill(ui, "Join on play", ProgressTone::Success);
                } else if button(ui, "Join").clicked() {
                    action = RowAction::Join;
                }
                if server.password.as_deref().is_some_and(|p| !p.is_empty()) {
                    ui.label(RichText::new(icons::LOCK_SIMPLE).color(palette.text_secondary))
                        .on_hover_text("Password saved");
                }
            });
        });
        action
    }
}

enum RowAction {
    None,
    Select,
    Edit,
    Join,
    Leave,
}

fn is_joined(server: &FavoriteServer, joined: Option<&str>) -> bool {
    joined.is_some() && server.name.as_deref() == joined
}

/// Makes `server` the one the next launch joins (with its preset), or none.
fn join(viewer: &ViewerContext, toasts: &mut Toaster, server: Option<&FavoriteServer>) {
    let name = server.and_then(|s| s.name.clone());
    let modset = server.and_then(|s| s.modset_name.clone());
    let joined = viewer.mutate(Some(Change::Online), |state| {
        set_join(&mut state.client, name, modset)
    });
    if let Err(err) = joined {
        toasts.error(err.user_message());
    }
}

/// Removes the server at `index` and offers Undo, which adds it back (joined again if it was).
fn remove(viewer: &ViewerContext, toasts: &mut Toaster, index: usize, view: &View) {
    let Some(server) = view.servers.get(index).cloned() else {
        return;
    };
    let was_joined = is_joined(&server, view.joined.as_deref());
    let previous_modset = viewer.read().client.configuration().default_modset.clone();
    let removed = viewer.mutate(Some(Change::Online), |state| {
        state.client.delete_favorite_server(index)?;
        if was_joined {
            set_join(&mut state.client, None, None)?;
        }
        Ok::<(), ServiceError>(())
    });
    if let Err(err) = removed {
        toasts.error(err.user_message());
        return;
    }
    let label = server.name.clone().unwrap_or_default();
    toasts.undoable(format!("Removed {label}"), move |viewer| {
        let restored = viewer.mutate(Some(Change::Online), |state| {
            let name = server.name.clone();
            state.client.add_favorite_server(server)?;
            if was_joined {
                set_join(&mut state.client, name, previous_modset)?;
            }
            Ok::<(), ServiceError>(())
        });
        if let Err(err) = restored {
            tracing::warn!(
                "Cannot restore the favourite server: {}",
                err.user_message()
            );
        }
    });
}
