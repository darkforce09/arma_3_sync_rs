//! The Publish segment: the published revision, the local changes and the repository size;
//! the step list of the running or last publish; what players will get.

use std::time::{Duration, SystemTime};

use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_ui::widgets::{StepState, button, status_pill, step_row};
use a3s_viewer_context::TaskSnapshot;
use a3s_viewer_context::admin::{
    self, BuildStage, ModChanges, PublishStep, Remote, ServerCheck, StepDetail, StepStatus,
    UploadStage,
};
use egui::{Align, Label, Layout, RichText, Ui};

use super::{View, send};
use crate::screens::{ScreenCx, ago_text, count_text, section_title, size_text};

pub(super) fn show(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    notices(ui, view);
    tiles(ui, view);
    server_check(ui, view);
    steps(ui, view, cx);
    players(ui, view, cx.search);
    history(ui, view, cx);
}

/// The result of "Check the server files".
// Mirrors ui/repository/dialogs/error/ErrorsListDialog.java:17-109
fn server_check(ui: &mut Ui, view: &View) {
    let Some(check) = &view.publishing.server_check else {
        return;
    };
    let palette = theme::palette(ui.ctx());
    section_title(ui, "Server files");
    frames::group(ui, |ui| match check {
        ServerCheck::Running(percent) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(match percent {
                    Some(percent) => format!("Checking the server's files… {percent}%"),
                    None => "Checking the server's files…".to_owned(),
                });
            });
        }
        ServerCheck::Done { checked, missing } if missing.is_empty() => {
            ui.label(
                RichText::new(format!(
                    "{} All {} are on the server.",
                    icons::CHECK_CIRCLE,
                    count_text(*checked, "file", "files")
                ))
                .color(palette.status_success),
            );
        }
        ServerCheck::Done { missing, .. } => {
            ui.label(
                RichText::new(format!(
                    "{} {} missing on the server; the next publish uploads them.",
                    icons::WARNING,
                    count_text(missing.len(), "file is", "files are")
                ))
                .color(palette.status_warning),
            );
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .id_salt("a3s_viewer::publishing::missing")
                .show(ui, |ui| {
                    for message in missing {
                        ui.add(Label::new(RichText::new(message).small()).wrap());
                    }
                });
        }
        ServerCheck::NothingPublished => {
            ui.label("Nothing is published on the server yet.");
        }
        ServerCheck::Failed(error) => {
            ui.label(RichText::new(&error.message).color(palette.status_danger));
        }
    });
}

/// The last builds of the source folder, newest first, collapsed, with Copy.
// Mirrors ui/repository/dialogs/ChangelogPanel.java:225-264
fn history(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    let Some(Ok(local)) = &view.publishing.local else {
        return;
    };
    if local.history.is_empty() {
        return;
    }
    let palette = theme::palette(ui.ctx());
    let lines: Vec<(String, Vec<String>)> = local
        .history
        .iter()
        .map(|changelog| {
            let mut title = format!("Revision {}", changelog.revision);
            if let Some(date) = changelog
                .build_date
                .and_then(chrono::DateTime::from_timestamp_millis)
            {
                title.push_str(&format!(" · {}", date.format("%b %-d, %Y %H:%M UTC")));
            }
            let details = [
                ("New", &changelog.new_addons),
                ("Updated", &changelog.updated_addons),
                ("Removed", &changelog.deleted_addons),
            ]
            .into_iter()
            .filter(|(_, addons)| !addons.is_empty())
            .map(|(label, addons)| format!("{label}: {}", addons.join(", ")))
            .collect();
            (title, details)
        })
        .collect();
    ui.add_space(16.0);
    egui::CollapsingHeader::new(RichText::new("History").strong())
        .id_salt(("a3s_viewer::publishing::history", view.name()))
        .default_open(false)
        .show(ui, |ui| {
            for (title, details) in &lines {
                ui.label(RichText::new(title).strong());
                if details.is_empty() {
                    ui.label(RichText::new("No mod changed.").color(palette.text_secondary));
                }
                for detail in details {
                    ui.add(Label::new(RichText::new(detail).color(palette.text_secondary)).wrap());
                }
                ui.add_space(6.0);
            }
            if button(ui, "Copy").clicked() {
                let text = lines
                    .iter()
                    .map(|(title, details)| {
                        std::iter::once(title.clone())
                            .chain(details.iter().cloned())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                ui.ctx().copy_text(text);
                cx.toasts.info("Copied the history");
            }
        });
}

/// The server or the source folder could not be read.
fn notices(ui: &mut Ui, view: &View) {
    let palette = theme::palette(ui.ctx());
    let mut problems = Vec::new();
    if let Remote::Failed(error) = &view.publishing.remote {
        problems.push(format!(
            "Cannot read the published repository: {}",
            error.message
        ));
    }
    if let Some(Err(error)) = &view.publishing.local {
        problems.push(error.message.clone());
    }
    for problem in problems {
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARNING).color(palette.status_warning));
                ui.add(Label::new(problem).wrap());
            });
        });
        ui.add_space(12.0);
    }
}

fn tiles(ui: &mut Ui, view: &View) {
    let publishing = &view.publishing;
    let published = match &publishing.remote {
        Remote::NotLoaded | Remote::Loading => ("…".to_owned(), "Reading the server".to_owned()),
        Remote::Failed(_) => ("Unknown".to_owned(), "Cannot reach the server".to_owned()),
        Remote::Loaded(remote) => match &remote.server_info {
            Some(info) => (
                format!("Revision {}", info.revision),
                info.build_date
                    .and_then(|millis| u64::try_from(millis).ok())
                    .map(|millis| {
                        ago_text(
                            SystemTime::UNIX_EPOCH + Duration::from_millis(millis),
                            SystemTime::now(),
                        )
                    })
                    .unwrap_or_default(),
            ),
            None => ("Not yet".to_owned(), "Nothing on the server".to_owned()),
        },
    };
    let local = publishing
        .local
        .as_ref()
        .and_then(|local| local.as_ref().ok());
    let changes = match (&publishing.local, local) {
        (None, _) => ("…".to_owned(), "Scanning the source folder".to_owned()),
        (_, None) => (
            "Unknown".to_owned(),
            "Cannot read the source folder".to_owned(),
        ),
        (_, Some(local)) if local.changed_files == 0 => {
            ("None".to_owned(), "Matches the server".to_owned())
        }
        (_, Some(local)) => (
            count_text(local.changed_files, "file", "files"),
            size_text(local.changed_bytes),
        ),
    };
    let size = local.map_or_else(
        || ("…".to_owned(), String::new()),
        |local| {
            (
                size_text(local.bytes),
                count_text(local.mods.len(), "mod", "mods"),
            )
        },
    );
    ui.columns(3, |columns| {
        tile(&mut columns[0], "Published", &published);
        tile(&mut columns[1], "Local changes", &changes);
        tile(&mut columns[2], "Repository", &size);
    });
}

fn tile(ui: &mut Ui, title: &str, (value, detail): &(String, String)) {
    let palette = theme::palette(ui.ctx());
    frames::group(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.label(RichText::new(title).small().color(palette.text_secondary));
        ui.label(RichText::new(value).heading());
        ui.add(Label::new(RichText::new(detail).small().color(palette.text_secondary)).truncate());
    });
}

/// The step list with its live details, and Retry or Resume under a failed or stopped step.
fn steps(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let run = view.publishing.run.clone().unwrap_or_default();
    let snapshot = view
        .publish_task
        .and_then(|id| cx.viewer.task_snapshot(id))
        .unwrap_or_default();
    section_title(ui, "Steps");
    frames::group(ui, |ui| {
        for (index, step) in PublishStep::ALL.into_iter().enumerate() {
            if index > 0 {
                frames::group_separator(ui);
            }
            let state = run.step(step);
            let marker = match state.status {
                StepStatus::Done => StepState::Done,
                StepStatus::Running | StepStatus::Failed(_) | StepStatus::Stopped => {
                    StepState::Active
                }
                StepStatus::Pending => StepState::Pending,
            };
            step_row(
                ui,
                index + 1,
                marker,
                title(step),
                &detail_text(step, &state.status, &state.detail, &snapshot),
            );
            let problem = match &state.status {
                StepStatus::Failed(error) => Some((error.message.as_str(), "Retry")),
                StepStatus::Stopped => Some(("Stopped.", "Resume")),
                _ => None,
            };
            if let Some((text, action)) = problem
                && view.publish_task.is_none()
            {
                ui.horizontal(|ui| {
                    ui.add_space(32.0);
                    ui.label(RichText::new(icons::WARNING).color(palette.status_danger));
                    ui.add(Label::new(RichText::new(text).color(palette.status_danger)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, action).clicked() {
                            send(
                                cx,
                                admin::Command::Publish {
                                    repository: view.name().to_owned(),
                                    from: step,
                                },
                            );
                        }
                    });
                });
            }
        }
        if view.publish_task.is_some() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, "Stop").clicked() {
                    send(
                        cx,
                        admin::Command::CancelPublish {
                            repository: view.name().to_owned(),
                        },
                    );
                }
            });
        }
    });
}

fn title(step: PublishStep) -> &'static str {
    match step {
        PublishStep::Scan => "Scan source folder",
        PublishStep::Build => "Build",
        PublishStep::Verify => "Verify",
        PublishStep::Upload => "Upload",
    }
}

/// The right-hand text of a step row.
fn detail_text(
    step: PublishStep,
    status: &StepStatus,
    detail: &StepDetail,
    snapshot: &TaskSnapshot,
) -> String {
    match status {
        StepStatus::Failed(_) => return "Failed".to_owned(),
        StepStatus::Stopped => return "Stopped".to_owned(),
        StepStatus::Pending => return String::new(),
        StepStatus::Running | StepStatus::Done => {}
    }
    let percent = |text: &str, percent: &Option<u8>| match percent {
        Some(percent) => format!("{text} {percent}%"),
        None => format!("{text}…"),
    };
    match detail {
        StepDetail::None => match step {
            PublishStep::Scan => "Reading the source folder…".to_owned(),
            _ => String::new(),
        },
        StepDetail::Scanned {
            files,
            changed_files,
            changed_bytes,
        } => format!(
            "{}, {} changed ({})",
            count_text(*files, "file", "files"),
            changed_files,
            size_text(*changed_bytes)
        ),
        StepDetail::Building { stage, percent: p } => {
            let text = match stage {
                None => "Scanning",
                Some(BuildStage::Checksums) => "Computing checksums",
                Some(BuildStage::Zsync) => "Writing delta files",
                Some(BuildStage::Compression) => "Compressing",
                Some(BuildStage::Cleanup) => "Cleaning up",
            };
            percent(text, p)
        }
        StepDetail::Built { revision, rehashed } => format!(
            "Revision {revision}, {} checked again",
            count_text(*rehashed, "file", "files")
        ),
        StepDetail::Verifying { percent: p } => percent("Checking the server", p),
        StepDetail::Verified { checked, missing } if *missing == 0 => {
            format!(
                "All {} on the server",
                count_text(*checked, "file", "files")
            )
        }
        StepDetail::Verified { missing, .. } => format!(
            "{} missing on the server; the upload restores them",
            count_text(*missing, "file", "files")
        ),
        StepDetail::NothingPublished => "Nothing published yet".to_owned(),
        StepDetail::Uploading { stage, percent: p } => match stage {
            Some(UploadStage::Uploading) => match &snapshot.transfer {
                Some(transfer) => {
                    let mut text = format!(
                        "{} of {}",
                        size_text(transfer.transferred),
                        size_text(transfer.total)
                    );
                    if let Some(speed) = snapshot.speed {
                        text.push_str(&format!(" · {}/s", size_text(speed)));
                    }
                    text
                }
                None => "Uploading…".to_owned(),
            },
            Some(UploadStage::Checking) => percent("Checking the server", p),
            Some(UploadStage::Deleting) => percent("Removing old files", p),
            None => "Connecting…".to_owned(),
        },
        StepDetail::ServedDirectly => "Not needed: the server serves the source folder".to_owned(),
        StepDetail::Uploaded { files, bytes, .. } => format!(
            "{} uploaded ({})",
            count_text(*files, "file", "files"),
            size_text(*bytes)
        ),
    }
}

/// The mods players get: from the build of this run, or else from the source folder.
fn players(ui: &mut Ui, view: &View, search: &str) {
    let palette = theme::palette(ui.ctx());
    section_title(ui, "What players will get");
    let built = view
        .publishing
        .run
        .as_ref()
        .and_then(|run| run.changes.clone());
    let scanned = view
        .publishing
        .local
        .as_ref()
        .and_then(|local| local.as_ref().ok())
        .map(|local| local.changes.clone());
    frames::group(ui, |ui| {
        let Some(changes) = built.or(scanned) else {
            ui.label(
                RichText::new("Shown once the source folder was read.")
                    .color(palette.text_secondary),
            );
            return;
        };
        if changes.is_empty() {
            ui.label(RichText::new("No mod changes.").color(palette.text_secondary));
            return;
        }
        change_rows(ui, &changes, search);
    });
}

fn change_rows(ui: &mut Ui, changes: &ModChanges, search: &str) {
    let rows = changes
        .added
        .iter()
        .map(|name| (name, "Added", ProgressTone::Success))
        .chain(
            changes
                .updated
                .iter()
                .map(|name| (name, "Updated", ProgressTone::Info)),
        )
        .chain(
            changes
                .removed
                .iter()
                .map(|name| (name, "Removed", ProgressTone::Warning)),
        );
    for (index, (name, text, tone)) in rows
        .filter(|(name, ..)| crate::screens::matches_search(search, name))
        .enumerate()
    {
        if index > 0 {
            frames::group_separator(ui);
        }
        ui.horizontal(|ui| {
            ui.add(Label::new(name.as_str()).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                status_pill(ui, text, tone);
            });
        });
    }
}
