//! Sheets: short modal tasks over the dimmed main window, shown one at a time in queue order.

mod add_repository;
mod exit;
mod load_warnings;
mod welcome;

use std::collections::VecDeque;

use a3s_viewer_context::ViewerContext;

pub(crate) use self::exit::ExitPrompt;
use self::welcome::Welcome;
pub use self::welcome::WelcomeSetup;
use crate::toasts::Toaster;

/// The kind of a sheet, as tests and callers see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetKind {
    /// First run: the Arma 3 folder, the mods folder and the Java data import.
    Welcome,

    /// Problems found while loading the client data.
    LoadWarnings,

    /// Quitting while tasks run, or after the settings could not be saved.
    ExitConfirmation,

    /// Adding a repository.
    AddRepository,
}

/// A sheet and its state.
pub(crate) enum Sheet {
    Welcome(Box<Welcome>),
    LoadWarnings(Vec<String>),
    Exit(ExitPrompt),
    AddRepository,
}

impl Sheet {
    pub(crate) fn welcome(setup: WelcomeSetup) -> Self {
        Self::Welcome(Box::new(Welcome::new(setup)))
    }

    fn kind(&self) -> SheetKind {
        match self {
            Self::Welcome(_) => SheetKind::Welcome,
            Self::LoadWarnings(_) => SheetKind::LoadWarnings,
            Self::Exit(_) => SheetKind::ExitConfirmation,
            Self::AddRepository => SheetKind::AddRepository,
        }
    }
}

/// What a closed sheet asks the window to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SheetOutcome {
    /// Quit even though tasks run.
    QuitConfirmed,

    /// Quit although the settings were not saved.
    QuitWithoutSaving,

    /// Stay after a failed save: the background work starts again.
    Stay,
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
    /// Queues `sheet` after the others; a sheet of the same kind already queued is kept and
    /// the new one dropped.
    pub(crate) fn push(&mut self, sheet: Sheet) {
        if !self
            .queue
            .iter()
            .any(|queued| queued.kind() == sheet.kind())
        {
            self.queue.push_back(sheet);
        }
    }

    /// Shows `sheet` before the others, replacing a queued sheet of the same kind.
    pub(crate) fn push_front(&mut self, sheet: Sheet) {
        self.queue.retain(|queued| queued.kind() != sheet.kind());
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
            Sheet::Exit(prompt) => exit::show(ctx, prompt),
            Sheet::AddRepository => add_repository::show(ctx),
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
