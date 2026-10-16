//! The Activity screen: the running downloads with their progress, Pause and Stop, the paused
//! ones, and the recent results.

use std::time::SystemTime;

use a3s_foundation::error::UserFacing as _;
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_ui::widgets::{Progress, ProgressBar, button};
use a3s_viewer_context::sync::{self, DownloadRecord, DownloadResult};
use a3s_viewer_context::{Command, TaskInfo, TaskKind, TaskSnapshot};
use egui::{Align, Label, Layout, RichText, Ui};

use super::{ScreenCx, ago_text, count_text, section_title, size_text, time_left_text};
use crate::shell::Action;

/// Files listed under a running download.
const LISTED_FILES: usize = 3;

/// A running transfer and what the screen shows of it.
struct Running {
    task: TaskInfo,
    repository: String,
    snapshot: TaskSnapshot,
    files: Vec<String>,

    /// Waiting to go on after a lost connection: the attempt that comes next.
    reconnecting: Option<u32>,
}

/// Whether the screen lists running tasks of `kind`; the sidebar badge counts those.
pub(crate) fn listed(kind: TaskKind) -> bool {
    matches!(
        kind,
        TaskKind::Download | TaskKind::AutoUpdate | TaskKind::Compare
    )
}

pub(super) fn show(ui: &mut Ui, cx: &mut ScreenCx<'_>) {
    let running: Vec<Running> = {
        let tasks: Vec<TaskInfo> = cx
            .viewer
            .running_tasks()
            .into_iter()
            .filter(|task| listed(task.key.kind))
            .collect();
        let state = cx.viewer.read();
        tasks
            .into_iter()
            .filter_map(|task| {
                let repository = task.key.repository.clone()?;
                let snapshot = cx.viewer.task_snapshot(task.id).unwrap_or_default();
                let files = state
                    .sync
                    .downloads
                    .active_files
                    .get(&repository)
                    .cloned()
                    .unwrap_or_default();
                let reconnecting = state.sync.downloads.reconnecting.get(&repository).copied();
                Some(Running {
                    task,
                    repository,
                    snapshot,
                    files,
                    reconnecting,
                })
            })
            .collect()
    };
    let (paused, recent) = {
        let state = cx.viewer.read();
        (
            state.sync.downloads.paused_all(),
            state.sync.downloads.recent.clone(),
        )
    };
    // The toolbar search narrows every list to its repositories.
    let search = cx.search;
    let running: Vec<Running> = running
        .into_iter()
        .filter(|transfer| super::matches_search(search, &transfer.repository))
        .collect();
    let paused: Vec<(String, u64)> = paused
        .into_iter()
        .filter(|(repository, _)| super::matches_search(search, repository))
        .collect();
    let recent: Vec<DownloadRecord> = recent
        .into_iter()
        .filter(|record| super::matches_search(search, &record.repository))
        .collect();
    if running.is_empty() && paused.is_empty() && recent.is_empty() {
        super::empty_state(
            ui,
            icons::PULSE,
            "Activity",
            "Downloads in progress and their results appear here.",
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if !running.is_empty() {
                section_title(ui, "In progress");
                for transfer in &running {
                    running_card(ui, transfer, cx);
                    ui.add_space(6.0);
                }
            }
            if !paused.is_empty() {
                section_title(ui, "Paused");
                frames::group(ui, |ui| {
                    for (index, (repository, remaining)) in paused.iter().enumerate() {
                        if index > 0 {
                            frames::group_separator(ui);
                        }
                        paused_row(ui, repository, *remaining, cx);
                    }
                });
            }
            if !recent.is_empty() {
                section_title(ui, "Recent");
                let now = SystemTime::now();
                frames::group(ui, |ui| {
                    for (index, record) in recent.iter().enumerate() {
                        if index > 0 {
                            frames::group_separator(ui);
                        }
                        recent_row(ui, record, now, cx);
                    }
                });
            }
        });
}

fn running_card(ui: &mut Ui, transfer: &Running, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let snapshot = &transfer.snapshot;
    frames::group(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icons::DOWNLOAD_SIMPLE)
                    .size(20.0)
                    .color(palette.accent_primary),
            );
            ui.label(RichText::new(&transfer.repository).strong());
            if let Some(phase) = snapshot.phase {
                ui.label(RichText::new(phase.label()).color(palette.text_secondary));
            } else {
                ui.label(
                    RichText::new(transfer.task.key.kind.label()).color(palette.text_secondary),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match transfer.task.key.kind {
                    TaskKind::Download => {
                        if button(ui, &format!("{} Stop", icons::STOP)).clicked() {
                            send(
                                cx,
                                sync::Command::Stop {
                                    repository: transfer.repository.clone(),
                                },
                            );
                        }
                        if button(ui, &format!("{} Pause", icons::PAUSE)).clicked() {
                            send(
                                cx,
                                sync::Command::Pause {
                                    repository: transfer.repository.clone(),
                                },
                            );
                        }
                    }
                    _ => {
                        if button(ui, &format!("{} Stop", icons::STOP)).clicked() {
                            cx.viewer.cancel_task(transfer.task.id);
                        }
                    }
                }
            });
        });
        let progress = match &snapshot.transfer {
            Some(transfer) if transfer.total > 0 => Progress::of(
                i64::try_from(transfer.transferred).unwrap_or(i64::MAX),
                i64::try_from(transfer.total).unwrap_or(i64::MAX),
            ),
            _ => match snapshot.progress {
                Some((_, percent)) => Progress::of(i64::from(percent), 100),
                None => Progress::Indeterminate { text: None },
            },
        };
        ui.add(ProgressBar::new(progress).tone(ProgressTone::Info));
        if let Some(transfer) = &snapshot.transfer {
            let mut line = format!(
                "{} of {}",
                size_text(transfer.transferred),
                size_text(transfer.total)
            );
            if let Some(speed) = snapshot.speed {
                line.push_str(&format!(
                    " · {}",
                    a3s_foundation::format::speed(i64::try_from(speed).unwrap_or(i64::MAX))
                ));
            }
            if let Some(remaining) = transfer.remaining {
                line.push_str(&format!(" · {}", time_left_text(remaining)));
            }
            ui.label(RichText::new(line).small().color(palette.text_secondary));
        }
        let files: Vec<&str> = if transfer.files.is_empty() {
            snapshot
                .transfer
                .as_ref()
                .and_then(|transfer| transfer.current_file.as_deref())
                .into_iter()
                .collect()
        } else {
            transfer.files.iter().map(String::as_str).collect()
        };
        for file in files.iter().take(LISTED_FILES) {
            ui.add(
                Label::new(RichText::new(*file).small().color(palette.text_secondary)).truncate(),
            );
        }
        if files.len() > LISTED_FILES {
            ui.label(
                RichText::new(format!("and {} more", files.len() - LISTED_FILES))
                    .small()
                    .color(palette.text_secondary),
            );
        }
        if let Some(attempt) = transfer.reconnecting {
            ui.label(
                RichText::new(format!(
                    "{} Connection lost; trying again in a few seconds (attempt {attempt} of {})",
                    icons::WARNING,
                    sync::RECONNECT_ATTEMPTS
                ))
                .small()
                .color(palette.status_warning),
            );
        }
        // Mirrors ui/repository/workers/AddonsDownloader.java:373-387
        if let Some(seconds) = snapshot
            .waiting_seconds
            .filter(|s| *s >= WAITING_SHOWN_AFTER)
        {
            ui.label(
                RichText::new(format!(
                    "{} Waiting for the server ({seconds} s)",
                    icons::HOURGLASS
                ))
                .small()
                .color(palette.status_warning),
            );
        }
        details(ui, transfer);
    });
}

/// Seconds without throughput before the card says it waits for the server.
const WAITING_SHOWN_AFTER: u32 = 15;

/// The transfer's figures behind a disclosure: connections, disk writes, files, current file.
fn details(ui: &mut Ui, transfer: &Running) {
    let snapshot = &transfer.snapshot;
    let rows: Vec<(&str, String)> = [
        (
            "Connections",
            snapshot.active_connections.map(|n| n.to_string()),
        ),
        (
            "Writing to disk",
            snapshot.disk_usage.map(|bytes| {
                a3s_foundation::format::speed(i64::try_from(bytes).unwrap_or(i64::MAX))
            }),
        ),
        (
            "Files",
            snapshot.total.map(|(_, files)| {
                count_text(
                    usize::try_from(files).unwrap_or(usize::MAX),
                    "file",
                    "files",
                )
            }),
        ),
        (
            "Current file",
            snapshot
                .transfer
                .as_ref()
                .filter(|transfer| transfer.current_file.is_some())
                .map(|transfer| format!("{}%", transfer.current_file_percent)),
        ),
    ]
    .into_iter()
    .filter_map(|(label, value)| value.map(|value| (label, value)))
    .collect();
    if rows.is_empty() {
        return;
    }
    let palette = theme::palette(ui.ctx());
    egui::CollapsingHeader::new(RichText::new("Details").small())
        .id_salt(("a3s_viewer::activity::details", &transfer.repository))
        .default_open(false)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).small().color(palette.text_secondary));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(value).small());
                    });
                });
            }
        });
}

fn paused_row(ui: &mut Ui, repository: &str, remaining: u64, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(icons::PAUSE_CIRCLE).color(palette.text_secondary));
        ui.label(repository);
        ui.label(
            RichText::new(format!("{} left", size_text(remaining))).color(palette.text_secondary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, "Stop").clicked() {
                send(
                    cx,
                    sync::Command::Stop {
                        repository: repository.to_owned(),
                    },
                );
            }
            if button(ui, "Resume").clicked() {
                send(
                    cx,
                    sync::Command::Resume {
                        repository: repository.to_owned(),
                    },
                );
            }
        });
    });
}

fn recent_row(ui: &mut Ui, record: &DownloadRecord, now: SystemTime, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    // "N min ago" moves on by itself.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(30));
    ui.horizontal(|ui| {
        let (icon, tone, text) = match &record.result {
            DownloadResult::Success { .. } => (
                icons::CHECK_CIRCLE,
                ProgressTone::Success,
                "Finished".to_owned(),
            ),
            DownloadResult::Issues(errors) => (
                icons::WARNING,
                ProgressTone::Warning,
                format!(
                    "Finished with {}",
                    count_text(errors.len(), "issue", "issues")
                ),
            ),
            DownloadResult::Failed(error) => (
                icons::X_CIRCLE,
                ProgressTone::Danger,
                format!("Failed: {}", error.message),
            ),
        };
        ui.label(RichText::new(icon).color(palette.status_color(tone)));
        let title = match &record.event {
            Some(event) => format!("{} ({event})", record.repository),
            None => record.repository.clone(),
        };
        ui.label(title);
        ui.add(Label::new(RichText::new(text).color(palette.text_secondary)).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(ago_text(record.finished, now))
                    .small()
                    .color(palette.text_secondary),
            );
            if let Some(report) = &record.report
                && button(ui, "Report").clicked()
            {
                cx.sheets
                    .push(crate::sheets::Sheet::report(&record.repository, report));
            }
            if matches!(record.result, DownloadResult::Issues(_))
                && button(ui, "Show issues").clicked()
            {
                cx.actions
                    .push(Action::ShowIssues(record.repository.clone()));
            }
        });
    });
}

fn send(cx: &mut ScreenCx<'_>, command: sync::Command) {
    if let Err(err) = cx.viewer.send(Command::Sync(command)) {
        cx.toasts.error(err.user_message());
    }
}
