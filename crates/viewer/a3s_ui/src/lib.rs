//! Theme, icons and reusable egui widgets for the Arma3Sync viewer.
//!
//! Ports the look of the Java Swing UI (`fr.soe.a3s.ui`): the `FlatLaf` theme tokens
//! ([`theme`]), the bundled Geist fonts ([`fonts`]), the Phosphor icon font ([`icon`]), the `UiStyle`
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

pub use self::theme::{ThemeDefinition, ThemeLibrary, ThemeMode};

/// Installs the Geist fonts and the light and dark styles, then selects the default (dark)
/// theme (`ui/ThemeManager.java:20,25-31`).
///
/// Call it once before the first frame (e.g. in the eframe creation callback). egui loads new
/// fonts only at the start of the next pass, so the styles installed here use a proportional
/// heading at first; a small plugin switches to the full [`theme::style_for`] styles (Geist
/// `SemiBold` headings) at the start of the first pass in which the fonts are available.
/// Style tweaks made before that pass are overwritten then; make them afterwards.
pub fn install(ctx: &egui::Context) {
    fonts::install_fonts(ctx);
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        ctx.set_style_of(
            mode.into(),
            theme::style_before_fonts(ThemeLibrary::for_mode(mode)),
        );
    }
    ctx.add_plugin(GeistStylePlugin { pending: true });
    ctx.with_plugin(|plugin: &mut GeistStylePlugin| plugin.pending = true);
    theme::set_mode(ctx, ThemeMode::default());
}

/// Applies the full styles once the Geist fonts are loaded (see [`install`]).
struct GeistStylePlugin {
    pending: bool,
}

impl egui::plugin::Plugin for GeistStylePlugin {
    fn debug_name(&self) -> &'static str {
        "a3s_ui::GeistStylePlugin"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        if !self.pending {
            return;
        }
        let ctx = ui.ctx().clone();
        let family = fonts::FontFace::SemiBold.family();
        if !ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
            return;
        }
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            ctx.set_style_of(mode.into(), theme::style_for(ThemeLibrary::for_mode(mode)));
        }
        ui.set_style(ctx.style_of(ctx.theme()));
        self.pending = false;
    }
}
