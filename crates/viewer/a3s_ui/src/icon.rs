//! Icons: the Phosphor icon font and the application icon.
//!
//! An icon is a glyph. Put a constant from [`icons`] into any text, e.g.
//! `format!("{} Play", icons::PLAY)`; it takes the size and colour of that text.
//! [`crate::install`] registers the font as a fallback of every text family.

/// Phosphor glyphs (regular weight), e.g. [`icons::PLAY`].
pub use egui_phosphor::regular as icons;

/// The application icon: the Arma3Sync logo as a 512×512 PNG.
pub const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon.png");

/// Name of the icon font in the font definitions.
pub(crate) const ICON_FONT_NAME: &str = "phosphor";

/// The icon font data.
pub(crate) fn icon_font_bytes() -> &'static [u8] {
    egui_phosphor::Variant::Regular.font_bytes()
}
