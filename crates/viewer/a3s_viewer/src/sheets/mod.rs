//! Sheets: short modal tasks over the dimmed main window, shown one at a time in queue order.

mod about;
pub(crate) mod add_repository;
pub(crate) mod download_settings;
mod exit;
mod game_running;
mod launcher_preset;
mod load_warnings;
mod path_list;
mod play_check;
mod preset_export;
mod report;
pub(crate) mod server;
mod update_issues;
mod upload_connection;
mod welcome;

use std::collections::VecDeque;

use a3s_tools::preset::ImportResult;
use a3s_viewer_context::ViewerContext;

use self::add_repository::{Mode, RepositoryForm};
use self::download_settings::DownloadSettings;
pub(crate) use self::exit::ExitPrompt;
pub(crate) use self::game_running::GameRunning;
use self::path_list::PathList;
pub use self::path_list::PathListKind;
pub(crate) use self::play_check::PlayCheck;
pub(crate) use self::preset_export::PresetExport;
use self::report::ReportSheet;
use self::server::ServerSheet;
pub(crate) use self::update_issues::UpdateIssues;
use self::upload_connection::UploadConnection;
use self::welcome::Welcome;
pub use self::welcome::WelcomeSetup;
use crate::shell::Screen;
use crate::toasts::Toaster;
use crate::windows::SettingsPane;

/// The kind of a sheet, as tests and callers see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetKind {
    /// First run: the Arma 3 folder, the mods folder and the Java data import.
    Welcome,

    /// Problems found while loading the client data.
    LoadWarnings,

    /// The version, the licence and the credits.
    About,

    /// Quitting while tasks run, or after the settings could not be saved.
    ExitConfirmation,

    /// Adding a repository.
    AddRepository,

    /// Editing a repository's connection.
    EditRepository,

    /// Creating a repository published from a folder of this computer.
    PublishFolder,

    /// Where and how a repository downloads.
    DownloadSettings,

    /// Files that failed in the last download of a repository.
    UpdateIssues,

    /// The report of a finished download.
    DownloadReport,

    /// An update asks first because Arma 3 is running.
    GameRunning,

    /// Play found enabled mods that are missing or enabled twice.
    PlayCheck,

    /// Comparing an Arma 3 Launcher preset with the mods on this computer.
    LauncherPreset,

    /// Exporting an addon group as an Arma 3 Launcher preset.
    PresetExport,

    /// Adding a favourite server.
    AddServer,

    /// Editing a favourite server.
    EditServer,

    /// Editing where a repository built on this computer is uploaded.
    UploadConnection,

    /// Editing a path list of a repository built on this computer.
    PathList(PathListKind),
}

/// A sheet and its state.
pub(crate) enum Sheet {
    Welcome(Box<Welcome>),
    LoadWarnings(Vec<String>),
    About,
    Exit(ExitPrompt),
    Repository(Box<RepositoryForm>),
    DownloadSettings(DownloadSettings),
    UpdateIssues(UpdateIssues),
    Report(ReportSheet),
    GameRunning(GameRunning),
    PlayCheck(PlayCheck),
    LauncherPreset(Box<ImportResult>),
    PresetExport(Box<PresetExport>),
    Server(Box<ServerSheet>),
    UploadConnection(Box<UploadConnection>),
    PathList(PathList),
}

impl Sheet {
    pub(crate) fn welcome(setup: WelcomeSetup) -> Self {
        Self::Welcome(Box::new(Welcome::new(setup)))
    }

    pub(crate) fn add_repository(viewer: &ViewerContext) -> Self {
        Self::Repository(Box::new(RepositoryForm::add(viewer)))
    }

    /// The sheet creating a repository to publish from a folder of this computer.
    pub(crate) fn publish_folder(viewer: &ViewerContext) -> Self {
        Self::Repository(Box::new(RepositoryForm::publish(viewer)))
    }

    /// The Upload connection sheet of `repository`.
    pub(crate) fn upload_connection(viewer: &ViewerContext, repository: &str) -> Self {
        Self::UploadConnection(Box::new(UploadConnection::new(viewer, repository)))
    }

    /// The report of a finished download of `repository`.
    pub(crate) fn report(repository: &str, text: &str) -> Self {
        Self::Report(ReportSheet::new(repository, text))
    }

    /// The Download settings sheet of `repository`.
    pub(crate) fn download_settings(repository: &str) -> Self {
        Self::DownloadSettings(DownloadSettings::new(repository))
    }

    /// The sheet editing the `kind` list of `repository`.
    pub(crate) fn path_list(repository: &str, kind: PathListKind) -> Self {
        Self::PathList(PathList::new(repository, kind))
    }

    /// The Edit connection sheet of `name`; `None` when it does not exist.
    pub(crate) fn edit_repository(viewer: &ViewerContext, name: &str) -> Option<Self> {
        RepositoryForm::edit(viewer, name).map(|form| Self::Repository(Box::new(form)))
    }

    /// Whether `other` is the same sheet: the same kind, and for Update issues the same
    /// repository.
    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UpdateIssues(a), Self::UpdateIssues(b)) => a.repository() == b.repository(),
            _ => self.kind() == other.kind(),
        }
    }

    fn kind(&self) -> SheetKind {
        match self {
            Self::Welcome(_) => SheetKind::Welcome,
            Self::LoadWarnings(_) => SheetKind::LoadWarnings,
            Self::About => SheetKind::About,
            Self::Exit(_) => SheetKind::ExitConfirmation,
            Self::Repository(form) => match form.mode() {
                Mode::Add => SheetKind::AddRepository,
                Mode::Edit(_) => SheetKind::EditRepository,
                Mode::Publish => SheetKind::PublishFolder,
            },
            Self::DownloadSettings(_) => SheetKind::DownloadSettings,
            Self::UpdateIssues(_) => SheetKind::UpdateIssues,
            Self::Report(_) => SheetKind::DownloadReport,
            Self::GameRunning(_) => SheetKind::GameRunning,
            Self::PlayCheck(_) => SheetKind::PlayCheck,
            Self::LauncherPreset(_) => SheetKind::LauncherPreset,
            Self::PresetExport(_) => SheetKind::PresetExport,
            Self::Server(sheet) if sheet.adds() => SheetKind::AddServer,
            Self::Server(_) => SheetKind::EditServer,
            Self::UploadConnection(_) => SheetKind::UploadConnection,
            Self::PathList(list) => SheetKind::PathList(list.kind),
        }
    }
}

/// What a closed sheet asks the window to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SheetOutcome {
    /// Quit even though tasks run.
    QuitConfirmed,

    /// Quit although the settings were not saved.
    QuitWithoutSaving,

    /// Stay after a failed save: the background work starts again.
    Stay,

    /// Show this screen, e.g. the repository just added.
    Navigate(Screen),

    /// Start the game without waiting for the failed files.
    Play,

    /// Open the Settings window at this pane.
    OpenSettings(SettingsPane),

    /// Start the game after its check was answered; `update_first` downloads the missing mods.
    PlayChecked { update_first: bool },
}

/// Whether the sheet stays open after this frame.
enum Shown {
    Open,
    Closed(Option<SheetOutcome>),
}

/// The sheet queue.
#[derive(Default)]
pub(crate) struct Sheets {
    queue: VecDeque<Sheet>,
}

impl Sheets {
    /// Queues `sheet` after the others; the same sheet already queued is kept and the new one
    /// dropped.
    pub(crate) fn push(&mut self, sheet: Sheet) {
        if !self.queue.iter().any(|queued| queued.same_as(&sheet)) {
            self.queue.push_back(sheet);
        }
    }

    /// Shows `sheet` before the others, replacing the same sheet when it is queued.
    pub(crate) fn push_front(&mut self, sheet: Sheet) {
        self.queue.retain(|queued| !queued.same_as(&sheet));
        self.queue.push_front(sheet);
    }

    /// The sheet on screen.
    pub(crate) fn current(&self) -> Option<SheetKind> {
        self.queue.front().map(Sheet::kind)
    }

    /// Shows the first sheet; returns what it asks for when it closes.
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Option<SheetOutcome> {
        let sheet = self.queue.front_mut()?;
        let shown = match sheet {
            Sheet::Welcome(welcome) => welcome.show(ctx, viewer, toasts),
            Sheet::LoadWarnings(warnings) => load_warnings::show(ctx, warnings),
            Sheet::About => about::show(ctx),
            Sheet::Exit(prompt) => exit::show(ctx, prompt),
            Sheet::Repository(form) => form.show(ctx, viewer, toasts),
            Sheet::DownloadSettings(sheet) => sheet.show(ctx, viewer, toasts),
            Sheet::UpdateIssues(issues) => issues.show(ctx, viewer, toasts),
            Sheet::Report(report) => report.show(ctx, toasts),
            Sheet::GameRunning(sheet) => sheet.show(ctx, viewer, toasts),
            Sheet::PlayCheck(check) => check.show(ctx),
            Sheet::LauncherPreset(result) => launcher_preset::show(ctx, result, viewer, toasts),
            Sheet::PresetExport(export) => export.show(ctx, toasts),
            Sheet::Server(sheet) => sheet.show(ctx, viewer, toasts),
            Sheet::UploadConnection(sheet) => sheet.show(ctx, viewer, toasts),
            Sheet::PathList(list) => list.show(ctx, viewer, toasts),
        };
        match shown {
            Shown::Open => None,
            Shown::Closed(outcome) => {
                self.queue.pop_front();
                ctx.request_repaint();
                outcome
            }
        }
    }
}
