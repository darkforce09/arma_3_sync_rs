//! The source-list sidebar: Library, Repositories with their badges, Publishing, and the last
//! check time at the bottom.

use std::time::{Duration, SystemTime};

use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_ui::widgets::{SidebarItem, sidebar_heading};
use egui::{Align, Layout, RichText, Ui};

use crate::shell::{Action, Screen};
use crate::snapshot::{Badge, Snapshot};

/// What the sidebar needs besides the snapshot.
pub(crate) struct SidebarView<'a> {
    pub(crate) snapshot: &'a Snapshot,
    pub(crate) screen: &'a Screen,
    /// Running tasks the Activity screen lists.
    pub(crate) running_tasks: usize,
    pub(crate) last_check: Option<SystemTime>,

    /// A check asked for by the user runs; it can be stopped.
    pub(crate) user_check: bool,
}

/// Shows the sidebar rows; returns the action of a clicked row.
pub(crate) fn show(ui: &mut Ui, view: &SidebarView<'_>) -> Option<Action> {
    let mut action = None;
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let palette = theme::palette(ui.ctx());
        if view.last_check.is_some() {
            // "Checked N min ago" moves on by itself.
            ui.ctx().request_repaint_after(Duration::from_secs(30));
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(check_status(
                    view.snapshot.checking,
                    view.last_check,
                    SystemTime::now(),
                ))
                .small()
                .color(palette.text_secondary),
            );
            if view.user_check && ui.small_button("Stop").clicked() {
                action = Some(Action::CancelCheck);
            }
        });
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    sidebar_heading(ui, "Library");
                    let tasks = view.running_tasks.to_string();
                    let library = [
                        (Screen::Mods, SidebarItem::new(icons::PUZZLE_PIECE, "Mods")),
                        (
                            Screen::Servers,
                            SidebarItem::new(icons::HARD_DRIVES, "Servers"),
                        ),
                        (Screen::Activity, {
                            let item = SidebarItem::new(icons::PULSE, "Activity");
                            if view.running_tasks > 0 {
                                item.badge(&tasks)
                            } else {
                                item
                            }
                        }),
                    ];
                    for (screen, item) in library {
                        if item.show(ui, *view.screen == screen).clicked() {
                            action = Some(Action::Navigate(screen));
                        }
                    }

                    sidebar_heading(ui, "Repositories");
                    for row in &view.snapshot.repositories {
                        let screen = Screen::Repository(row.name.clone());
                        let count;
                        let item = SidebarItem::new(icons::DATABASE, &row.name);
                        let item = match row.badge {
                            Badge::None => item,
                            Badge::Checking => item.badge("…"),
                            Badge::Failed => item.badge("!").badge_tone(ProgressTone::Danger),
                            Badge::Updates(changed) => {
                                count = if changed > 0 {
                                    changed.to_string()
                                } else {
                                    "New".to_owned()
                                };
                                item.badge(&count).badge_tone(ProgressTone::Warning)
                            }
                        };
                        let mut response = item.show(ui, *view.screen == screen);
                        if let Some(error) = &row.error {
                            response =
                                response.on_hover_text(format!("The last check failed: {error}"));
                        }
                        if response.clicked() {
                            action = Some(Action::Navigate(screen));
                        }
                    }
                    if SidebarItem::new(icons::PLUS_CIRCLE, "Add repository")
                        .show(ui, false)
                        .clicked()
                    {
                        action = Some(Action::AddRepository);
                    }

                    sidebar_heading(ui, "Publishing");
                    for name in &view.snapshot.publishing {
                        let screen = Screen::Publishing(name.clone());
                        if SidebarItem::new(icons::CLOUD_ARROW_UP, name)
                            .show(ui, *view.screen == screen)
                            .clicked()
                        {
                            action = Some(Action::Navigate(screen));
                        }
                    }
                    if SidebarItem::new(icons::FOLDER_PLUS, "Publish a folder")
                        .show(ui, false)
                        .on_hover_text("Create a repository from a folder of this computer")
                        .clicked()
                    {
                        action = Some(Action::PublishFolder);
                    }
                });
        });
    });
    action
}

/// The status line at the bottom of the sidebar.
fn check_status(checking: bool, last_check: Option<SystemTime>, now: SystemTime) -> String {
    if checking {
        return "Checking repositories…".to_owned();
    }
    let Some(last_check) = last_check else {
        return "Not checked yet".to_owned();
    };
    let elapsed = now.duration_since(last_check).unwrap_or(Duration::ZERO);
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "Checked just now".to_owned(),
        1..=59 => format!("Checked {minutes} min ago"),
        _ => format!("Checked {} h ago", minutes / 60),
    }
}
