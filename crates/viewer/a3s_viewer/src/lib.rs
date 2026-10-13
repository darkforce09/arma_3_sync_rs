//! The Arma3Sync desktop window.
//!
//! [`run`] opens the window over a [`a3s_viewer_context::ViewerContext`]: single-instance lock,
//! tray icon, and the [`Shell`]: unified toolbar with the Play button, source-list sidebar,
//! screens, sheets, toasts, the Settings window and the tool windows.
//!
//! [`Shell`] is the whole per-frame UI without eframe, so tests drive it headless with
//! `egui_kittest`: call [`Shell::logic`] then [`Shell::show`] once per frame, as eframe does.

mod desktop;
mod events;
mod fields;
mod launch;
mod legacy_import;
mod notify;
mod picker;
mod requests;
mod run;
mod screens;
mod sheets;
mod shell;
mod shortcuts;
mod sidebar;
mod snapshot;
mod toasts;
mod toolbar;
mod tray;
mod windows;

pub use self::desktop::Desktop;
pub use self::requests::{RequestSender, WindowRequest, channel};
pub use self::run::{Exit, RunOptions, ViewerError, display_available, run};
pub use self::sheets::{SheetKind, WelcomeSetup};
pub use self::shell::{Screen, Shell, ShellSetup};
pub use self::toasts::{Toaster, UndoAction};
pub use self::windows::{SecondaryWindow, SettingsPane, SettingsWindow};
