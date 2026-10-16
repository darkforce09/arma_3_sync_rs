//! The Repository screen: one repository's header with its Update button, its events, the
//! changes a download makes, and the changelog of the revisions not downloaded yet.

use std::convert::Infallible;

use a3s_foundation::error::UserFacing as _;
use a3s_types::local::Repository;
use a3s_types::{Changelog, Event};
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_ui::widgets::{button, primary_button, status_pill};
use a3s_viewer_context::sync::{self, ChangeKind, DownloadResult, PlanState};
use a3s_viewer_context::{
    Change, Command, RepositoryRuntime, TaskKey, TaskKind, ViewerContext, repositories,
};
use egui::{Align, Label, Layout, RichText, Ui};

use super::{ScreenCx, count_text, section_title, size_text};
use crate::sheets::add_repository::protocol_input;
use crate::shell::{Action, Screen};

/// What the screen shows, copied out of the state so no guard is held while it runs actions.
struct View {
    repository: Repository,
    url: String,
    runtime: RepositoryRuntime,
    plan: Option<PlanState>,
    paused: Option<u64>,
    failed_files: usize,
    events: Vec<Event>,
    changelogs: Vec<Changelog>,
}

impl View {
    fn take(viewer: &ViewerContext, name: &str) -> Option<Self> {
        let state = viewer.read();
        let repository = state.client.repository(name).ok()?.clone();
        let runtime = state.repositories.runtime(name);
        let downloads = &state.sync.downloads;
        let failed_files = match downloads.last_result(name).map(|record| &record.result) {
            Some(DownloadResult::Issues(errors)) => errors.len(),
            _ => 0,
        };
        let events = runtime
            .metadata
            .events
            .as_ref()
            .map(|events| events.list.clone())
            .unwrap_or_default();
        let mut changelogs: Vec<Changelog> = runtime
            .metadata
            .changelogs
            .as_ref()
            .map(|changelogs| {
                changelogs
                    .list
                    .iter()
                    .filter(|changelog| changelog.revision > repository.revision)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        changelogs.sort_by_key(|changelog| std::cmp::Reverse(changelog.revision));
        Some(Self {
            url: state
                .client
                .repository_display_url(name)
                .unwrap_or_default(),
            plan: downloads.plans.get(name).cloned(),
            paused: downloads.paused(name),
            failed_files,
            events,
            changelogs,
            runtime,
            repository,
        })
    }

    /// The comparison is missing or older than the server revision.
    fn plan_is_stale(&self) -> bool {
        self.plan
            .as_ref()
            .is_none_or(|plan| plan.server_revision != self.runtime.metadata.server_revision())
    }
}

/// What the Update button does and says.
enum Primary {
    Update(Option<u64>),
    Resume(u64),
    Updating(Option<u8>),
    Comparing,
    UpToDate,
}

pub(super) fn show(ui: &mut Ui, name: &str, cx: &mut ScreenCx<'_>) {
    let Some(view) = View::take(cx.viewer, name) else {
        super::empty_state(
            ui,
            icons::DATABASE,
            name,
            "This repository no longer exists.",
        );
        return;
    };
    let downloading = cx
        .viewer
        .is_running(&TaskKey::repository(TaskKind::Download, name));
    let comparing = cx
        .viewer
        .is_running(&TaskKey::repository(TaskKind::Compare, name))
        .is_some();
    compare_when_stale(cx, name, &view, downloading.is_some() || comparing);

    let primary = if let Some(id) = downloading {
        let percent = cx
            .viewer
            .task_snapshot(id)
            .and_then(|snapshot| snapshot.transfer)
            .map(|transfer| transfer.percent());
        Primary::Updating(percent)
    } else if let Some(remaining) = view.paused {
        Primary::Resume(remaining)
    } else {
        match view.plan.as_ref().map(|plan| &plan.result) {
            Some(Ok(plan)) if !view.plan_is_stale() => {
                if plan.is_up_to_date() {
                    Primary::UpToDate
                } else {
                    Primary::Update(Some(plan.download_size))
                }
            }
            _ if comparing => Primary::Comparing,
            _ => Primary::Update(None),
        }
    };

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            header(ui, &view, &primary, cx);
            notices(ui, &view, cx);
            events(ui, &view, cx);
            changes(ui, &view, comparing);
            changelog(ui, &view);
        });
}

/// Compares the repository with its folder once its metadata is known and the last comparison
/// is older than the server revision.
fn compare_when_stale(cx: &ScreenCx<'_>, name: &str, view: &View, busy: bool) {
    let checked = view.runtime.metadata.server_info.is_some() && !view.runtime.checking;
    if busy || !checked || view.paused.is_some() || !view.plan_is_stale() {
        return;
    }
    if let Err(err) = sync::compare(cx.viewer.cx(), name) {
        tracing::warn!("Cannot compare repository {name}: {err}");
    }
}

fn header(ui: &mut Ui, view: &View, primary: &Primary, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let name = view.repository.name.as_str();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::DATABASE)
                .size(36.0)
                .color(palette.accent_primary),
        );
        ui.vertical(|ui| {
            ui.label(RichText::new(name).heading());
            ui.add(Label::new(RichText::new(&view.url).color(palette.text_secondary)).truncate());
            let revision = view
                .runtime
                .metadata
                .server_revision()
                .unwrap_or(view.repository.revision);
            let mut details = format!("Revision {revision}");
            if let Some(mods) = repositories::addon_count(&view.runtime.metadata) {
                details.push_str(&format!(" · {}", count_text(mods, "mod", "mods")));
            }
            ui.label(RichText::new(details).small().color(palette.text_secondary));
            let folder = view
                .repository
                .default_download_location
                .as_deref()
                .filter(|folder| !folder.is_empty())
                .map_or_else(
                    || "No download folder".to_owned(),
                    |folder| format!("Downloads to {folder}"),
                );
            ui.add(
                Label::new(RichText::new(folder).small().color(palette.text_secondary)).truncate(),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button(RichText::new(icons::DOTS_THREE_CIRCLE).size(18.0), |ui| {
                if ui.button("Edit connection…").clicked() {
                    cx.actions.push(Action::EditRepository(name.to_owned()));
                }
                if ui.button("Download settings…").clicked() {
                    cx.actions.push(Action::DownloadSettings(name.to_owned()));
                }
                if ui.button("Publish from this computer…").clicked() {
                    cx.actions
                        .push(Action::Navigate(Screen::Publishing(name.to_owned())));
                }
                if ui.button("Check now").clicked() {
                    send(
                        cx,
                        Command::Repositories(repositories::Command::Check {
                            names: vec![name.to_owned()],
                        }),
                    );
                }
                ui.separator();
                if ui.button("Remove repository").clicked() {
                    remove(cx, view);
                }
            });
            primary_action(ui, name, primary, cx);
        });
    });
}

fn primary_action(ui: &mut Ui, name: &str, primary: &Primary, cx: &mut ScreenCx<'_>) {
    let download = || {
        Command::Sync(sync::Command::Download {
            repository: name.to_owned(),
            event: None,
        })
    };
    match primary {
        Primary::Update(size) => {
            let label = match size {
                Some(size) => format!("Update {}", size_text(*size)),
                None => "Update".to_owned(),
            };
            if primary_button(ui, &label).clicked() {
                send(cx, download());
            }
        }
        Primary::Resume(remaining) => {
            if primary_button(ui, &format!("Resume {}", size_text(*remaining))).clicked() {
                send(cx, download());
            }
        }
        Primary::Updating(percent) => {
            let label = match percent {
                Some(percent) => format!("Updating… {percent}%"),
                None => "Updating…".to_owned(),
            };
            if button(ui, &label).clicked() {
                cx.actions.push(Action::Navigate(Screen::Activity));
            }
        }
        Primary::Comparing => {
            ui.add_enabled(false, egui::Button::new("Comparing…"));
        }
        Primary::UpToDate => {
            ui.add_enabled(
                false,
                egui::Button::new(format!("{} Up to date", icons::CHECK_CIRCLE)),
            );
        }
    }
}

/// A failed check, a failed comparison, and failed files of the last download.
fn notices(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let name = view.repository.name.as_str();
    if let Some(error) = &view.runtime.last_check_error
        && !view.runtime.checking
    {
        ui.add_space(12.0);
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARNING).color(palette.status_danger));
                ui.add(
                    Label::new(format!("Cannot check the repository: {}", error.message)).wrap(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Edit connection…").clicked() {
                        cx.actions.push(Action::EditRepository(name.to_owned()));
                    }
                    if button(ui, "Check again").clicked() {
                        send(
                            cx,
                            Command::Repositories(repositories::Command::Check {
                                names: vec![name.to_owned()],
                            }),
                        );
                    }
                });
            });
        });
    }
    if let Some(PlanState {
        result: Err(error), ..
    }) = &view.plan
    {
        ui.add_space(12.0);
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARNING).color(palette.status_warning));
                ui.add(Label::new(&error.message).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Edit connection…").clicked() {
                        cx.actions.push(Action::EditRepository(name.to_owned()));
                    }
                });
            });
        });
    }
    if view.failed_files > 0 {
        ui.add_space(12.0);
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARNING).color(palette.status_warning));
                ui.label(format!(
                    "{} not updated last time.",
                    count_text(view.failed_files, "file was", "files were")
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Show issues").clicked() {
                        cx.actions.push(Action::ShowIssues(name.to_owned()));
                    }
                });
            });
        });
    }
}

/// One card per event of the repository.
fn events(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    if view.events.is_empty() {
        return;
    }
    let palette = theme::palette(ui.ctx());
    section_title(ui, "Events");
    for event in &view.events {
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::CALENDAR_BLANK).color(palette.accent_primary));
                ui.vertical(|ui| {
                    ui.label(RichText::new(&event.name).strong());
                    if let Some(description) =
                        event.description.as_deref().filter(|d| !d.is_empty())
                    {
                        ui.add(
                            Label::new(RichText::new(description).color(palette.text_secondary))
                                .wrap(),
                        );
                    }
                    ui.label(
                        RichText::new(count_text(event.addon_names.len(), "mod", "mods"))
                            .small()
                            .color(palette.text_secondary),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Use preset").clicked() {
                        super::mods::use_event_preset(
                            cx.viewer,
                            cx.toasts,
                            &view.repository.name,
                            event,
                        );
                    }
                });
            });
        });
        ui.add_space(6.0);
    }
}

/// "Changes since your last sync": what a download changes, by mod.
fn changes(ui: &mut Ui, view: &View, comparing: bool) {
    let palette = theme::palette(ui.ctx());
    section_title(ui, "Changes since your last sync");
    frames::group(ui, |ui| {
        let Some(Ok(plan)) = view.plan.as_ref().map(|plan| &plan.result) else {
            let text = if comparing {
                "Comparing with your download folder…"
            } else {
                "Not compared with your download folder yet."
            };
            ui.label(RichText::new(text).color(palette.text_secondary));
            return;
        };
        if plan.is_up_to_date() {
            ui.label(RichText::new("Everything is up to date.").color(palette.text_secondary));
            return;
        }
        for (index, change) in plan.changes.iter().enumerate() {
            if index > 0 {
                frames::group_separator(ui);
            }
            ui.horizontal(|ui| {
                let (text, tone) = match change.kind {
                    ChangeKind::New => ("New", ProgressTone::Success),
                    ChangeKind::Updated => ("Updated", ProgressTone::Info),
                    ChangeKind::Removed => ("Removed", ProgressTone::Warning),
                };
                ui.add(Label::new(&change.name).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let size = match change.kind {
                        ChangeKind::Removed => "to the Trash".to_owned(),
                        ChangeKind::New | ChangeKind::Updated => size_text(change.size),
                    };
                    ui.label(RichText::new(size).color(palette.text_secondary));
                    status_pill(ui, text, tone);
                });
            });
        }
    });
}

/// The revisions newer than the local one, newest first, collapsed.
fn changelog(ui: &mut Ui, view: &View) {
    if view.changelogs.is_empty() {
        return;
    }
    let palette = theme::palette(ui.ctx());
    ui.add_space(16.0);
    egui::CollapsingHeader::new(RichText::new("Changelog").strong())
        .id_salt(("a3s_viewer::changelog", &view.repository.name))
        .default_open(false)
        .show(ui, |ui| {
            for changelog in &view.changelogs {
                let mut title = format!("Revision {}", changelog.revision);
                if let Some(date) = changelog.build_date.and_then(date_text) {
                    title.push_str(&format!(" · {date}"));
                }
                ui.label(RichText::new(title).strong());
                for (label, addons) in [
                    ("New", &changelog.new_addons),
                    ("Updated", &changelog.updated_addons),
                    ("Removed", &changelog.deleted_addons),
                ] {
                    if !addons.is_empty() {
                        ui.add(
                            Label::new(
                                RichText::new(format!("{label}: {}", addons.join(", ")))
                                    .color(palette.text_secondary),
                            )
                            .wrap(),
                        );
                    }
                }
                ui.add_space(6.0);
            }
        });
}

/// A Java date (milliseconds since the epoch) as `Oct 10, 2026`, in UTC.
fn date_text(millis: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|date| date.format("%b %-d, %Y").to_string())
}

/// Removes the repository, with an Undo toast that stores it again.
fn remove(cx: &mut ScreenCx<'_>, view: &View) {
    let name = view.repository.name.clone();
    match cx.viewer.remove_repository(&name) {
        Ok(_) => {
            cx.viewer
                .mutate(None, |state| {
                    sync::forget(state, &name);
                    Ok::<(), Infallible>(())
                })
                .ok();
            let repository = view.repository.clone();
            cx.toasts
                .undoable(format!("Removed {name}"), move |viewer: &ViewerContext| {
                    restore(viewer, repository);
                });
            cx.actions.push(Action::Navigate(Screen::Mods));
        }
        Err(err) => cx.toasts.error(err.user_message()),
    }
}

/// Stores a removed repository again, with all its settings, and checks it.
fn restore(viewer: &ViewerContext, repository: Repository) {
    let name = repository.name.clone();
    let restored = viewer.mutate(Some(Change::Repository), |state| {
        state
            .client
            .create_repository(&name, &protocol_input(&repository.protocol))?;
        state.client.update_repository(&name, |stored| {
            *stored = repository;
            Ok(true)
        })
    });
    match restored {
        Ok(_) => {
            let check = Command::Repositories(repositories::Command::Check { names: vec![name] });
            if let Err(err) = viewer.send(check) {
                tracing::warn!("Cannot check the restored repository: {err}");
            }
        }
        Err(err) => tracing::warn!("Cannot restore repository {name}: {}", err.user_message()),
    }
}

fn send(cx: &mut ScreenCx<'_>, command: Command) {
    if let Err(err) = cx.viewer.send(command) {
        cx.toasts.error(err.user_message());
    }
}
