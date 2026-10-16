//! The main window: its state, the per-frame flow, and the window behaviour (hide to the tray,
//! quit).

use std::sync::mpsc;
use std::time::SystemTime;

use a3s_foundation::error::UserFacing as _;
use a3s_types::constants::MinimizationType;
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_viewer_context::{
    AppEvent, Change, Command, TaskKey, TaskKind, ViewerContext, profiles, repositories,
};
use egui::{Frame, Ui, ViewportCommand};

use crate::desktop::Desktop;
use crate::events::EventSink;
use crate::launch::{self, PlayRequest};
use crate::requests::WindowRequest;
use crate::screens::{ScreenCx, ScreenStates};
use crate::sheets::{
    ExitPrompt, PathListKind, Sheet, SheetKind, SheetOutcome, Sheets, UpdateIssues, WelcomeSetup,
};
use crate::sidebar::{self, SidebarView};
use crate::snapshot::Snapshot;
use crate::toasts::Toaster;
use crate::windows::{SecondaryWindow, SecondaryWindows, SettingsPane};
use crate::{screens, shortcuts, toolbar};

/// A screen of the content area, chosen in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// The active profile's addons.
    Mods,

    /// Favourite servers.
    Servers,

    /// Running and recent transfers.
    Activity,

    /// One repository, by name.
    Repository(String),

    /// Publishing one repository built on this computer, by name.
    Publishing(String),
}

impl Screen {
    /// The toolbar title.
    pub fn title(&self) -> String {
        match self {
            Self::Mods => "Mods".to_owned(),
            Self::Servers => "Servers".to_owned(),
            Self::Activity => "Activity".to_owned(),
            Self::Repository(name) => name.clone(),
            Self::Publishing(name) => format!("Publish {name}"),
        }
    }
}

/// Something the user asked for, from the toolbar, the sidebar, a shortcut or the tray.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Navigate(Screen),
    AddRepository,
    PublishFolder,
    EditRepository(String),
    DownloadSettings(String),
    EditUploadConnection(String),
    EditPathList(String, PathListKind),
    ShowIssues(String),
    Play,
    SelectProfile(String),
    Open(SecondaryWindow),
    OpenSettings(SettingsPane),
    CheckNow,
    CancelCheck,
    StopAutoRestart,
    Rescan,
    FocusSearch,
    Show,
    Hide,
    Minimize,
    Quit,
}

impl From<WindowRequest> for Action {
    fn from(request: WindowRequest) -> Self {
        match request {
            WindowRequest::Show => Self::Show,
            WindowRequest::Hide => Self::Hide,
            WindowRequest::CheckNow => Self::CheckNow,
            WindowRequest::Quit => Self::Quit,
        }
    }
}

/// How the window starts.
pub struct ShellSetup {
    /// Problems found while loading the client data; shown in a sheet.
    pub load_warnings: Vec<String>,

    /// Shows the Welcome sheet first.
    pub welcome: Option<WelcomeSetup>,

    /// Requests from the tray and from later instances.
    pub requests: mpsc::Receiver<WindowRequest>,

    /// The window starts hidden in the tray.
    pub hidden: bool,
}

/// The main window's UI over a [`ViewerContext`].
///
/// eframe calls [`Self::logic`] before every frame and also while the window is hidden, then
/// [`Self::show`] for visible frames.
pub struct Shell {
    viewer: ViewerContext,
    screen: Screen,
    search: String,
    sheets: Sheets,
    toasts: Toaster,
    windows: SecondaryWindows,
    screens: ScreenStates,
    requests: mpsc::Receiver<WindowRequest>,
    tray_available: bool,
    hidden: bool,
    last_check: Option<SystemTime>,

    /// The exit was decided: the next close request closes the window.
    closing: bool,
}

impl Shell {
    /// The window over `viewer`; the caller starts the background work.
    pub fn new(viewer: ViewerContext, setup: ShellSetup) -> Self {
        let mut sheets = Sheets::default();
        if let Some(welcome) = setup.welcome {
            sheets.push(Sheet::welcome(welcome));
        }
        if !setup.load_warnings.is_empty() {
            sheets.push(Sheet::LoadWarnings(setup.load_warnings));
        }
        Self {
            viewer,
            screen: Screen::Mods,
            search: String::new(),
            sheets,
            toasts: Toaster::default(),
            windows: SecondaryWindows::new(Desktop::system(None)),
            screens: ScreenStates::default(),
            requests: setup.requests,
            tray_available: false,
            hidden: setup.hidden,
            last_check: None,
            closing: false,
        }
    }

    /// Records whether a tray icon exists. Without one the window never hides, so a window
    /// that started hidden is shown.
    pub fn set_tray_available(&mut self, ctx: &egui::Context, available: bool) {
        self.tray_available = available;
        if !available && self.hidden {
            self.show_window(ctx);
        }
    }

    /// The desktop the Settings window integrates with (autostart, shortcuts, the Arma 3 user
    /// folder, the logs); the real one by default.
    pub fn set_desktop(&mut self, desktop: Desktop) {
        self.windows.set_desktop(desktop);
    }

    /// The context the window shows.
    pub fn viewer(&self) -> &ViewerContext {
        &self.viewer
    }

    /// The screen in the content area.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// The sheet on screen, if any.
    pub fn sheet(&self) -> Option<SheetKind> {
        self.sheets.current()
    }

    /// Opens the secondary window `window`, as its menu item does.
    pub fn open(&mut self, ctx: &egui::Context, window: SecondaryWindow) {
        self.windows.open(ctx, window);
    }

    /// Whether the secondary window `window` is open.
    pub fn is_open(&self, window: SecondaryWindow) -> bool {
        self.windows.is_open(window)
    }

    /// The pane the Settings window shows, when it is open.
    pub fn settings_pane(&self) -> Option<SettingsPane> {
        self.windows.settings_pane()
    }

    /// The window's toasts.
    pub fn toasts(&mut self) -> &mut Toaster {
        &mut self.toasts
    }

    /// Work that also runs while the window is hidden: requests from the tray and other
    /// instances, and the context's events. Shows nothing.
    pub fn logic(&mut self, ctx: &egui::Context) {
        let requests: Vec<WindowRequest> = self.requests.try_iter().collect();
        for request in requests {
            self.apply(ctx, Action::from(request));
        }
        if let Err(err) = profiles::ensure_scan(&self.viewer) {
            tracing::warn!("Cannot scan the addons: {err}");
        }
        let mut window_actions = Vec::new();
        let mut sink = EventSink {
            toasts: &mut self.toasts,
            sheets: &mut self.sheets,
            last_check: &mut self.last_check,
            hidden: self.hidden,
            game_starting: self
                .viewer
                .is_running(&TaskKey::new(TaskKind::Launch))
                .is_some(),
        };
        for event in self.viewer.poll_events() {
            if let AppEvent::Launch(event) = &event {
                window_actions.extend(launch::on_event(
                    event,
                    &self.viewer,
                    sink.toasts,
                    self.tray_available,
                ));
            }
            sink.handle(event);
        }
        for action in window_actions {
            self.apply(ctx, action);
        }
    }

    /// Shows the window: toolbar, sidebar, screen, sheets, secondary windows and toasts.
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.window_behaviour(&ctx);
        let mut actions = if self.sheets.current().is_none() {
            shortcuts::read(&ctx)
        } else {
            Vec::new()
        };

        let snapshot = Snapshot::take(&self.viewer.read());
        let def = theme::current(&ctx);
        egui::Panel::top("a3s_viewer::toolbar")
            .exact_size(frames::TOOLBAR_HEIGHT)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                actions.extend(toolbar::show(
                    ui,
                    &self.screen.title(),
                    &snapshot,
                    &mut self.search,
                ));
            });
        let sidebar = egui::Panel::left("a3s_viewer::sidebar")
            .exact_size(frames::SIDEBAR_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(frames::sidebar_frame(def))
            .show(ui, |ui| {
                sidebar::show(
                    ui,
                    &SidebarView {
                        snapshot: &snapshot,
                        screen: &self.screen,
                        running_tasks: self
                            .viewer
                            .running_tasks()
                            .iter()
                            .filter(|task| screens::activity::listed(task.key.kind))
                            .count(),
                        last_check: self.last_check,
                        user_check: self
                            .viewer
                            .is_running(&TaskKey::new(TaskKind::CheckRepositories))
                            .is_some(),
                    },
                )
            });
        frames::paint_pane_divider(ui.painter(), sidebar.response.rect, def);
        actions.extend(sidebar.inner);
        let content = egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(def.palette.window_background)
                    .inner_margin(16),
            )
            .show(ui, |ui| {
                let mut screen = ScreenCx {
                    viewer: &self.viewer,
                    search: &self.search,
                    toasts: &mut self.toasts,
                    sheets: &mut self.sheets,
                    actions: Vec::new(),
                };
                screens::show(ui, &self.screen, &mut self.screens, &mut screen);
                screen.actions
            });
        actions.extend(content.inner);

        for action in actions {
            self.apply_with(&ctx, action, &snapshot);
        }
        match self.sheets.show(&ctx, &self.viewer, &mut self.toasts) {
            Some(SheetOutcome::QuitConfirmed) => self.save_and_close(&ctx),
            Some(SheetOutcome::QuitWithoutSaving) => self.close(&ctx),
            Some(SheetOutcome::Stay) => self.viewer.start_background(),
            Some(SheetOutcome::Navigate(screen)) => self.screen = screen,
            Some(SheetOutcome::Play) => launch::request(
                &self.viewer,
                &PlayRequest {
                    profile: snapshot.active_profile.clone(),
                    update_first: false,
                    checked: true,
                },
                &mut self.toasts,
                &mut self.sheets,
            ),
            Some(SheetOutcome::PlayChecked { update_first }) => launch::request(
                &self.viewer,
                &PlayRequest {
                    profile: snapshot.active_profile.clone(),
                    update_first: update_first || snapshot.profile_has_updates,
                    checked: true,
                },
                &mut self.toasts,
                &mut self.sheets,
            ),
            None => {}
        }
        self.windows.show(&ctx, &self.viewer);
        self.toasts.show(ui, &self.viewer);
        for action in self.toasts.take_actions() {
            self.apply(&ctx, action);
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        let snapshot = Snapshot::take(&self.viewer.read());
        self.apply_with(ctx, action, &snapshot);
    }

    fn apply_with(&mut self, ctx: &egui::Context, action: Action, snapshot: &Snapshot) {
        match action {
            Action::Navigate(screen) => self.screen = screen,
            Action::AddRepository => self.sheets.push(Sheet::add_repository(&self.viewer)),
            Action::PublishFolder => self.sheets.push(Sheet::publish_folder(&self.viewer)),
            Action::EditRepository(name) => {
                if let Some(sheet) = Sheet::edit_repository(&self.viewer, &name) {
                    self.sheets.push(sheet);
                }
            }
            Action::DownloadSettings(name) => self.sheets.push(Sheet::download_settings(&name)),
            Action::EditUploadConnection(name) => self
                .sheets
                .push(Sheet::upload_connection(&self.viewer, &name)),
            Action::EditPathList(name, kind) => self.sheets.push(Sheet::path_list(&name, kind)),
            Action::ShowIssues(name) => self
                .sheets
                .push(Sheet::UpdateIssues(UpdateIssues::new(name, false))),
            Action::Play => launch::request(
                &self.viewer,
                &PlayRequest {
                    profile: snapshot.active_profile.clone(),
                    update_first: snapshot.profile_has_updates,
                    checked: false,
                },
                &mut self.toasts,
                &mut self.sheets,
            ),
            Action::SelectProfile(name) => {
                let selected = self.viewer.mutate(Some(Change::Profile), |state| {
                    state.client.set_active_profile(&name)
                });
                if let Err(err) = selected {
                    self.toasts.error(err.user_message());
                }
            }
            Action::Open(window) => self.windows.open(ctx, window),
            Action::OpenSettings(pane) => self.windows.open_settings(ctx, pane),
            Action::CheckNow => self.check_now(snapshot),
            Action::StopAutoRestart => {
                let game = self
                    .viewer
                    .mutate(None, |state| {
                        Ok::<_, std::convert::Infallible>(state.launch.game.take())
                    })
                    .ok()
                    .flatten();
                if let Some(game) = game {
                    game.stop_auto_restart();
                    self.toasts
                        .info("Arma 3 will not start again when it closes");
                }
            }
            Action::CancelCheck => {
                let command = Command::Repositories(repositories::Command::CancelCheck);
                if let Err(err) = self.viewer.send(command) {
                    self.toasts.error(err.user_message());
                }
            }
            Action::Rescan => {
                if let Err(err) = profiles::rescan(&self.viewer) {
                    self.toasts.error(err.user_message());
                }
            }
            Action::FocusSearch => {
                ctx.memory_mut(|memory| memory.request_focus(toolbar::search_id()));
            }
            Action::Show => self.show_window(ctx),
            Action::Hide => self.hide_window(ctx),
            Action::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
            Action::Quit => self.quit(ctx),
        }
    }

    fn check_now(&mut self, snapshot: &Snapshot) {
        let names = snapshot
            .repositories
            .iter()
            .map(|row| row.name.clone())
            .collect();
        let command = Command::Repositories(repositories::Command::Check { names });
        if let Err(err) = self.viewer.send(command) {
            self.toasts.error(err.user_message());
        }
    }

    /// Closing the window and minimizing it follow the "minimized" preference: into the tray
    /// when it says so and a tray exists, otherwise a close quits.
    // Mirrors ui/main/MainPanel.java:441-449,867-875
    fn window_behaviour(&mut self, ctx: &egui::Context) {
        let to_tray = self.tray_available
            && self
                .viewer
                .read()
                .client
                .preferences()
                .launch_panel_minimized
                == MinimizationType::Tray;
        let (close_requested, minimized) = ctx.input(|input| {
            (
                input.viewport().close_requested(),
                input.viewport().minimized,
            )
        });
        if close_requested && !self.closing {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if to_tray {
                self.hide_window(ctx);
            } else {
                self.quit(ctx);
            }
        }
        if to_tray && minimized == Some(true) && !self.hidden {
            self.hide_window(ctx);
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        if !self.tray_available {
            return;
        }
        self.hidden = true;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    /// Quits, asking first when work that matters runs.
    fn quit(&mut self, ctx: &egui::Context) {
        let running: Vec<String> = self
            .viewer
            .running_tasks()
            .into_iter()
            .filter(|task| !stops_silently(task.key.kind))
            .map(|task| match &task.key.repository {
                Some(repository) => format!("{} ({repository})", task.key.kind.label()),
                None => task.key.kind.label().to_owned(),
            })
            .collect();
        if running.is_empty() {
            self.save_and_close(ctx);
        } else {
            self.show_window(ctx);
            self.sheets
                .push_front(Sheet::Exit(ExitPrompt::TasksRunning(running)));
        }
    }

    /// Stops the background work and saves; closes the window, or asks when the save failed.
    // Mirrors ui/main/MainPanel.java:877-900
    fn save_and_close(&mut self, ctx: &egui::Context) {
        self.viewer.prepare_exit();
        let size = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .map_or_else(|| input.content_rect().size(), |rect| rect.size())
        });
        match self
            .viewer
            .save_on_exit(size.y.round() as i32, size.x.round() as i32)
        {
            Ok(()) => self.close(ctx),
            Err(err) => {
                self.show_window(ctx);
                self.sheets
                    .push_front(Sheet::Exit(ExitPrompt::SaveFailed(err.user_message())));
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

/// Tasks that quitting stops without asking: checks and watchers that lose nothing.
fn stops_silently(kind: TaskKind) -> bool {
    matches!(
        kind,
        TaskKind::PeriodicCheck
            | TaskKind::CheckRepositories
            | TaskKind::Compare
            | TaskKind::PublishingStatus
            | TaskKind::SelfUpdateCheck
            | TaskKind::PartialTransferTest
            | TaskKind::AdminCheck
            | TaskKind::AddonScan
            | TaskKind::Launch
            | TaskKind::RptTail
    )
}
