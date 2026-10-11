//! The bundled Geist font family.
//!
//! Ports `ui/FontInstaller.java`. Java searches external directories and the classpath and
//! validates the data at runtime; the fonts are embedded here, and a test checks the same
//! properties (size and SFNT signature) on the embedded bytes.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

/// Base text size in points (Q2: `FlatLaf`'s per-OS `defaultFont` is replaced by one size).
pub const BODY_SIZE: f32 = 13.0;

/// Size of [`TextStyle::Small`] (local choice).
pub const SMALL_SIZE: f32 = 11.0;

/// Size of [`TextStyle::Heading`] (local choice).
pub const HEADING_SIZE: f32 = 16.0;

/// Size of [`TextStyle::Monospace`] (local choice).
pub const MONOSPACE_SIZE: f32 = 12.0;

/// The SIL Open Font License 1.1 text of the Geist fonts. It must travel with the binary.
pub const GEIST_OFL_LICENSE: &str = include_str!("../assets/fonts/Geist-OFL.txt");

/// One of the six bundled Geist faces (`ui/FontInstaller.java:54-60`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFace {
    /// Geist Regular (Swing plain).
    Regular,
    /// Geist Italic (Swing italic).
    Italic,
    /// Geist Medium. Registered but never selected by Java.
    Medium,
    /// Geist `SemiBold` (Swing bold).
    SemiBold,
    /// Geist Bold. Registered but never selected by Java.
    Bold,
    /// Geist Bold Italic (Swing bold + italic).
    BoldItalic,
}

impl FontFace {
    /// All faces in Java registration order.
    pub const ALL: [Self; 6] = [
        Self::Regular,
        Self::Italic,
        Self::Medium,
        Self::SemiBold,
        Self::Bold,
        Self::BoldItalic,
    ];

    /// Font name, also the name of its [`FontFamily::Name`] family, e.g. `"Geist-Regular"`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Regular => "Geist-Regular",
            Self::Italic => "Geist-Italic",
            Self::Medium => "Geist-Medium",
            Self::SemiBold => "Geist-SemiBold",
            Self::Bold => "Geist-Bold",
            Self::BoldItalic => "Geist-BoldItalic",
        }
    }

    /// The embedded TTF data.
    pub fn ttf_bytes(self) -> &'static [u8] {
        match self {
            Self::Regular => include_bytes!("../assets/fonts/Geist-Regular.ttf"),
            Self::Italic => include_bytes!("../assets/fonts/Geist-Italic.ttf"),
            Self::Medium => include_bytes!("../assets/fonts/Geist-Medium.ttf"),
            Self::SemiBold => include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
            Self::Bold => include_bytes!("../assets/fonts/Geist-Bold.ttf"),
            Self::BoldItalic => include_bytes!("../assets/fonts/Geist-BoldItalic.ttf"),
        }
    }

    /// The egui family that renders this face first.
    pub fn family(self) -> FontFamily {
        FontFamily::Name(Arc::from(self.name()))
    }

    /// The face Java picks for a Swing font style: bold maps to **`SemiBold`**.
    ///
    /// Mirrors `ui/FontInstaller.java:159-162,557-587`.
    pub fn for_swing_style(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (true, true) => Self::BoldItalic,
            (true, false) => Self::SemiBold,
            (false, true) => Self::Italic,
            (false, false) => Self::Regular,
        }
    }
}

/// A [`FontId`] for `face` at `size` points.
pub fn font_id(face: FontFace, size: f32) -> FontId {
    FontId::new(size, face.family())
}

/// Like [`font_id`], but falls back to [`FontFamily::Proportional`] while the Geist fonts are
/// not installed in `ctx` (e.g. a frame that ran before [`crate::install`]).
pub fn font_id_in(ctx: &Context, face: FontFace, size: f32) -> FontId {
    let family = face.family();
    if ctx.fonts(|f| f.definitions().families.contains_key(&family)) {
        FontId::new(size, family)
    } else {
        FontId::proportional(size)
    }
}

/// The font definitions: egui's defaults plus the six Geist faces, with Geist Regular first in
/// [`FontFamily::Proportional`] and one named family per face.
pub fn font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let proportional_fallbacks = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    for face in FontFace::ALL {
        defs.font_data.insert(
            face.name().to_owned(),
            Arc::new(FontData::from_static(face.ttf_bytes())),
        );
        let mut family = vec![face.name().to_owned()];
        family.extend(proportional_fallbacks.iter().cloned());
        defs.families.insert(face.family(), family);
    }

    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, FontFace::Regular.name().to_owned());
    defs
}

/// Installs [`font_definitions`] into the context.
pub fn install_fonts(ctx: &Context) {
    ctx.set_fonts(font_definitions());
}

/// Text styles: body and button 13 Regular, small 11, heading 16 `SemiBold`, monospace 12.
///
/// Regular text uses [`FontFamily::Proportional`], whose first font is Geist Regular.
pub fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (TextStyle::Small, FontId::proportional(SMALL_SIZE)),
        (TextStyle::Body, FontId::proportional(BODY_SIZE)),
        (TextStyle::Button, FontId::proportional(BODY_SIZE)),
        (
            TextStyle::Heading,
            font_id(FontFace::SemiBold, HEADING_SIZE),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONOSPACE_SIZE, FontFamily::Monospace),
        ),
    ])
}
