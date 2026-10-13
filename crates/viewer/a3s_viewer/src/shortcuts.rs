//! Keyboard shortcuts of the main window. `COMMAND` is ⌘ on macOS and Ctrl elsewhere.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::shell::Action;
use crate::windows::SecondaryWindow;

/// Check every repository now.
pub(crate) const CHECK_NOW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);

/// Open the Settings window.
pub(crate) const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);

/// Focus the search field.
pub(crate) const SEARCH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);

/// Consumes this frame's shortcuts and returns their actions.
pub(crate) fn read(ctx: &egui::Context) -> Vec<Action> {
    ctx.input_mut(|input| {
        [
            (CHECK_NOW, Action::CheckNow),
            (SETTINGS, Action::Open(SecondaryWindow::Settings)),
            (SEARCH, Action::FocusSearch),
        ]
        .into_iter()
        .filter(|(shortcut, _)| input.consume_shortcut(shortcut))
        .map(|(_, action)| action)
        .collect()
    })
}
