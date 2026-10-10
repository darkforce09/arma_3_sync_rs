//! UI constants shared by every viewer panel.
//!
//! Mirrors the non-op-code part of `ui/UIConstants.java`. The `OP_*` controller codes become an
//! event enum in the viewer context crate and are deliberately not ported here.

use egui::Color32;

/// Application name shown in window titles. Mirrors `ui/UIConstants.java:10`.
pub const APPLICATION_NAME: &str = "Arma3Sync";

/// Default main-window height in points. Mirrors `ui/UIConstants.java:12`.
pub const DEFAULT_HEIGHT: u32 = 665;

/// Text of the confirming dialog button. Mirrors `ui/AbstractDialog.java:41`.
pub const OK: &str = "OK";

/// Text of the cancelling dialog button. Mirrors `ui/AbstractDialog.java:42` and
/// `ui/UiBootstrap.java:35`.
pub const CANCEL: &str = "Cancel";

/// Text of the "yes" option button. Mirrors `ui/UiBootstrap.java:33`.
pub const YES: &str = "Yes";

/// Text of the "no" option button. Mirrors `ui/UiBootstrap.java:34`.
pub const NO: &str = "No";

/// Swing `Color.RED`, theme-independent on purpose.
///
/// The addon and sync trees use it for missing, duplicated or updated entries
/// (`ui/main/tree/CheckTreeCellRenderer.java:44`,
/// `ui/repository/tree/CheckTreeCellRendererRepository.java:48`).
pub const JAVA_RED: Color32 = Color32::from_rgb(0xFF, 0x00, 0x00);

/// Swing `Color.BLUE`, theme-independent on purpose.
///
/// The sync tree uses it for deleted entries
/// (`ui/repository/tree/CheckTreeCellRendererRepository.java:44`).
pub const JAVA_BLUE: Color32 = Color32::from_rgb(0x00, 0x00, 0xFF);
