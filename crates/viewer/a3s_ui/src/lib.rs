//! Theme, icons and reusable egui widgets for the Arma3Sync viewer.
//!
//! Ports the look of the Java Swing UI (`fr.soe.a3s.ui`): the `FlatLaf` theme tokens
//! ([`theme`]), the bundled Geist fonts ([`fonts`]), the SVG icon set ([`icon`]), the `UiStyle`
//! frames ([`frames`]), the shared widgets ([`widgets`]) and the dialog bases ([`dialog`]).
//! [`gallery::Gallery`] shows all of them; `examples/demo.rs` runs it.
//!
//! Call [`install`] once at startup.

pub mod consts;
pub mod dialog;
pub mod fonts;
pub mod frames;
pub mod gallery;
pub mod icon;
pub mod theme;
pub mod widgets;

pub use self::icon::{IconState, IconTone, UiIcon};
pub use self::theme::{ThemeDefinition, ThemeLibrary, ThemeMode};

/// Installs the Geist fonts and the light and dark styles, then selects the default (dark)
/// theme (`ui/ThemeManager.java:20,25-31`).
pub fn install(ctx: &egui::Context) {
    fonts::install_fonts(ctx);
    ctx.set_style_of(egui::Theme::Light, theme::style_for(ThemeLibrary::light()));
    ctx.set_style_of(egui::Theme::Dark, theme::style_for(ThemeLibrary::dark()));
    theme::set_mode(ctx, ThemeMode::default());
}
