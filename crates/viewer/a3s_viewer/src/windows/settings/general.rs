//! General: appearance, what happens when the game starts, start-up and tray, repository
//! checks, notifications and the application version.

use std::sync::mpsc;

use a3s_platform::autostart::{AutostartEntry, AutostartStatus, autostart_backend};
use a3s_types::constants::{
    CheckRepositoriesFrequency, LookAndFeel, MinimizationType, StartWithOs,
};
use a3s_types::local::Preferences;
use a3s_ui::frames;
use a3s_ui::widgets::{button, form_row, popup_button, segmented};
use a3s_updater::apply::relaunch::RelaunchPlan;
use a3s_updater::apply::{CheckOutcome, source_for_preference, text};
use a3s_updater::{DiagnosticLog, Release, UpdateContext, UpdateEvent, UpdateSession};
use a3s_viewer_context::{TaskKey, TaskKind, ViewerContext};
use egui::Ui;

use super::{PaneCx, note};
use crate::desktop::Desktop;
use crate::fields::{labelled_row, toggle_row};

/// The application version, shared by the whole workspace.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The appearance choice, stored in the preferences' look-and-feel field: the Java client's
/// dark look ("Noire") reads as Dark, its other named looks as Light, and the default look as
/// Auto (follow the system).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Appearance {
    Light,
    Dark,
    #[default]
    Auto,
}

impl Appearance {
    /// Every choice, in display order.
    pub(crate) const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::Auto];

    /// The appearance a stored look stands for.
    pub(crate) fn from_look(look: LookAndFeel) -> Self {
        match look {
            LookAndFeel::LafDefault => Self::Auto,
            LookAndFeel::LafNoire => Self::Dark,
            LookAndFeel::LafAluminium
            | LookAndFeel::LafGraphite
            | LookAndFeel::LafHifi
            | LookAndFeel::LafMetal => Self::Light,
        }
    }

    /// The look stored for this appearance.
    pub(crate) fn look(self) -> LookAndFeel {
        match self {
            Self::Light => LookAndFeel::LafMetal,
            Self::Dark => LookAndFeel::LafNoire,
            Self::Auto => LookAndFeel::LafDefault,
        }
    }

    /// The egui theme preference.
    pub(crate) fn theme(self) -> egui::ThemePreference {
        match self {
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
            Self::Auto => egui::ThemePreference::System,
        }
    }

    /// The appearance's name.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Auto => "Auto",
        }
    }
}

const GAME_START_OPTIONS: [(MinimizationType, &str); 4] = [
    (MinimizationType::Nothing, "Keep the window open"),
    (MinimizationType::TaskBar, "Minimize the window"),
    (MinimizationType::Tray, "Hide in the tray"),
    (MinimizationType::Close, "Quit Arma3Sync"),
];

const FREQUENCIES: [(CheckRepositoriesFrequency, &str); 4] = [
    (CheckRepositoriesFrequency::Freq1, "Every 15 minutes"),
    (CheckRepositoriesFrequency::Freq2, "Every 30 minutes"),
    (CheckRepositoriesFrequency::Freq3, "Every hour"),
    (CheckRepositoriesFrequency::Disabled, "Only at start-up"),
];

/// Where the application update check and its installation stand.
#[derive(Default)]
enum UpdateCheck {
    #[default]
    NotChecked,
    Running(mpsc::Receiver<(String, Option<Box<Release>>)>),

    /// The check's line, and the newer release it found.
    Done(String, Option<Box<Release>>),

    /// The update downloads and installs; its progress line.
    Installing(mpsc::Receiver<InstallMessage>, String),
}

/// What the installation reports.
enum InstallMessage {
    Progress(String),
    Done(Result<RelaunchPlan, String>),
}

#[derive(Default)]
pub(super) struct GeneralPane {
    /// Whether an autostart entry exists; read once when the pane is first shown.
    autostart: Option<bool>,
    update: UpdateCheck,

    /// How to start the installed update; the window quits and starts it.
    pub(super) relaunch: Option<RelaunchPlan>,
}

impl GeneralPane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>) {
        let mut preferences = cx.viewer.read().client.preferences().clone();
        let before = preferences.clone();
        let autostart = *self
            .autostart
            .get_or_insert_with(|| autostart_enabled(cx.desktop));
        self.poll_update();

        frames::group(ui, |ui| {
            form_row(ui, "Appearance", |ui| {
                let mut index = Appearance::ALL
                    .iter()
                    .position(|a| *a == Appearance::from_look(preferences.look_and_feel))
                    .unwrap_or(2);
                let labels = Appearance::ALL.map(Appearance::label);
                if segmented(ui, &mut index, &labels).changed() {
                    let appearance = Appearance::ALL[index];
                    preferences.look_and_feel = appearance.look();
                    ui.ctx().set_theme(appearance.theme());
                }
            });
            frames::group_separator(ui);
            labelled_row(ui, "When the game starts", |ui| {
                popup_button(
                    ui,
                    "a3s_viewer::settings::game_start",
                    &mut preferences.launch_panel_game_launch,
                    &GAME_START_OPTIONS,
                )
            });
        });
        ui.add_space(12.0);

        let mut keep_in_tray = preferences.launch_panel_minimized == MinimizationType::Tray;
        frames::group(ui, |ui| {
            let mut on = autostart;
            if toggle_row(ui, "Open at login", &mut on).changed() {
                self.set_autostart(cx, on, &mut preferences);
            }
            frames::group_separator(ui);
            if toggle_row(ui, "Keep running in the tray", &mut keep_in_tray).changed() {
                preferences.launch_panel_minimized = if keep_in_tray {
                    MinimizationType::Tray
                } else {
                    MinimizationType::TaskBar
                };
            }
        });
        ui.add_space(12.0);

        frames::group(ui, |ui| {
            labelled_row(ui, "Check repositories", |ui| {
                popup_button(
                    ui,
                    "a3s_viewer::settings::frequency",
                    &mut preferences.check_repositories_frequency,
                    &FREQUENCIES,
                )
            });
        });
        ui.add_space(12.0);

        frames::group(ui, |ui| {
            form_row(ui, &format!("Arma3Sync {VERSION}"), |ui| {
                let running = matches!(
                    self.update,
                    UpdateCheck::Running(_) | UpdateCheck::Installing(..)
                );
                if ui
                    .add_enabled_ui(!running, |ui| button(ui, "Check now"))
                    .inner
                    .clicked()
                {
                    self.check_update(ui.ctx(), cx.viewer);
                }
                if running {
                    ui.spinner();
                }
            });
            match &self.update {
                UpdateCheck::Done(line, release) => {
                    note(ui, line.clone());
                    if let Some(release) = release.clone()
                        && a3s_ui::widgets::primary_button(ui, "Install and restart").clicked()
                    {
                        self.install(ui.ctx(), cx.viewer, *release);
                    }
                }
                UpdateCheck::Installing(_, line) => note(ui, line.clone()),
                UpdateCheck::NotChecked | UpdateCheck::Running(_) => {}
            }
        });

        if preferences != before {
            cx.report(cx.viewer.set_preferences(preferences));
        }
    }

    /// Creates or removes the autostart entry (which starts the window hidden with `-run`) and
    /// records the choice in the preferences.
    // Mirrors service/PreferencesService.java:29-54
    fn set_autostart(&mut self, cx: &mut PaneCx<'_>, on: bool, preferences: &mut Preferences) {
        let Some(env) = &cx.desktop.env else {
            cx.toasts
                .error("Open at login is not available: the environment is unknown.");
            return;
        };
        let backend = autostart_backend(env, cx.desktop.registry.as_ref());
        let result = if on {
            AutostartEntry::for_current_process(env, vec!["-run".to_owned()])
                .and_then(|entry| backend.enable(&entry))
        } else {
            backend.disable()
        };
        if cx.report(result).is_some() {
            self.autostart = Some(on);
            preferences.start_with_os = if on {
                StartWithOs::Enabled
            } else {
                StartWithOs::Disabled
            };
        }
    }

    /// Looks for a newer release on the background runtime; no download.
    fn check_update(&mut self, ctx: &egui::Context, viewer: &ViewerContext) {
        let context = match update_context(viewer) {
            Ok(context) => context,
            Err(err) => {
                self.update = UpdateCheck::Done(text::check_failed(&err), None);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = viewer.cx().spawn_task(
            TaskKey::new(TaskKind::SelfUpdateCheck),
            true,
            move |task| async move {
                let session = UpdateSession::new(context);
                let found = match session.check(&task.cancel).await {
                    Ok(CheckOutcome::Available { release, .. }) => (
                        format!("Version {} is available.", release.version),
                        Some(Box::new(release)),
                    ),
                    Ok(CheckOutcome::UpToDate { .. }) => {
                        ("Arma3Sync is up to date.".to_owned(), None)
                    }
                    Err(err) => (text::check_failed(&err), None),
                };
                sender.send(found).ok();
                ctx.request_repaint();
                Ok(())
            },
        );
        self.update = match spawned {
            Ok(_) => UpdateCheck::Running(receiver),
            Err(err) => UpdateCheck::Done(text::check_failed(&err), None),
        };
    }

    fn poll_update(&mut self) {
        match &mut self.update {
            UpdateCheck::Running(receiver) => match receiver.try_recv() {
                Ok((line, release)) => self.update = UpdateCheck::Done(line, release),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.update =
                        UpdateCheck::Done("The check stopped unexpectedly.".to_owned(), None);
                }
            },
            UpdateCheck::Installing(receiver, line) => loop {
                match receiver.try_recv() {
                    Ok(InstallMessage::Progress(progress)) => *line = progress,
                    Ok(InstallMessage::Done(Ok(plan))) => {
                        self.relaunch = Some(plan);
                        self.update = UpdateCheck::NotChecked;
                        break;
                    }
                    Ok(InstallMessage::Done(Err(error))) => {
                        self.update = UpdateCheck::Done(error, None);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.update =
                            UpdateCheck::Done("The update stopped unexpectedly.".to_owned(), None);
                        break;
                    }
                }
            },
            UpdateCheck::NotChecked | UpdateCheck::Done(..) => {}
        }
    }

    /// Downloads and installs `release` on the background runtime; once installed, the window
    /// quits and starts the new version.
    // Mirrors a3sUpdater/ui/Updater.java:60-140
    fn install(&mut self, ctx: &egui::Context, viewer: &ViewerContext, release: Release) {
        let context = match update_context(viewer) {
            Ok(context) => context,
            Err(err) => {
                self.update = UpdateCheck::Done(text::error_occurred(&err), None);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let first = text::updating(&release.version);
        let spawned = viewer.cx().spawn_task(
            TaskKey::new(TaskKind::SelfUpdate),
            true,
            move |task| async move {
                let session = UpdateSession::new(context);
                let progress = sender.clone();
                let repaint = ctx.clone();
                let on_event = move |event: UpdateEvent| {
                    let line = match event {
                        UpdateEvent::Downloading { bytes, total } => {
                            text::progress_percent(bytes, total)
                                .map_or_else(|| "Downloading…".to_owned(), text::downloading)
                        }
                        UpdateEvent::WaitingForElevation => {
                            "Waiting for administrator rights…".to_owned()
                        }
                        UpdateEvent::ElevatedInstallStarted => text::ELEVATED_CONTINUING.to_owned(),
                        UpdateEvent::Installing => "Installing…".to_owned(),
                        _ => return,
                    };
                    progress.send(InstallMessage::Progress(line)).ok();
                    repaint.request_repaint();
                };
                let result = session
                    .apply(&release, &on_event, &task.cancel)
                    .await
                    .map(|outcome| outcome.relaunch)
                    .map_err(|err| text::error_occurred(&err));
                sender.send(InstallMessage::Done(result)).ok();
                ctx.request_repaint();
                Ok(())
            },
        );
        self.update = match spawned {
            Ok(_) => UpdateCheck::Installing(receiver, first),
            Err(err) => UpdateCheck::Done(text::error_occurred(&err), None),
        };
    }
}

/// What the updater needs to know about this application.
fn update_context(viewer: &ViewerContext) -> std::io::Result<UpdateContext> {
    let (dev, prefer_github, config_dir) = {
        let state = viewer.read();
        (
            state.flags.dev_mode,
            state.client.preferences().prefer_github_updates,
            state.client.store().dirs().config.clone(),
        )
    };
    UpdateContext::for_current_process(
        VERSION,
        dev,
        source_for_preference(prefer_github),
        Some(config_dir),
        DiagnosticLog::from_env(),
    )
}

fn autostart_enabled(desktop: &Desktop) -> bool {
    let Some(env) = &desktop.env else {
        return false;
    };
    matches!(
        autostart_backend(env, desktop.registry.as_ref()).status(),
        Ok(AutostartStatus::Enabled { .. })
    )
}
