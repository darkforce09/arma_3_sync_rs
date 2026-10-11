//! Embedded Geist fonts.

use a3s_ui::fonts::{self, FontFace};
use egui::FontFamily;

/// `ui/FontInstaller.java:43-45`: at least 10 KiB and an SFNT signature.
const MIN_FONT_SIZE_BYTES: usize = 10 * 1024;

#[test]
fn font_assets_and_definitions() {
    for face in FontFace::ALL {
        let bytes = face.ttf_bytes();
        assert!(
            bytes.len() >= MIN_FONT_SIZE_BYTES,
            "{} is too small",
            face.name()
        );
        assert!(
            bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) || bytes.starts_with(b"OTTO"),
            "{} has an SFNT signature",
            face.name()
        );
    }

    let defs = fonts::font_definitions();
    let names = [
        "Geist-Regular",
        "Geist-Italic",
        "Geist-Medium",
        "Geist-SemiBold",
        "Geist-Bold",
        "Geist-BoldItalic",
    ];
    for name in names {
        assert!(defs.font_data.contains_key(name), "{name} registered");
        let family = defs
            .families
            .get(&FontFamily::Name(name.into()))
            .expect("named family");
        assert_eq!(family[0], name);
        assert!(family.len() > 1, "{name} keeps egui fallbacks");
    }
    assert_eq!(defs.families[&FontFamily::Proportional][0], "Geist-Regular");
    assert_ne!(defs.families[&FontFamily::Monospace][0], "Geist-Regular");

    assert!(fonts::GEIST_OFL_LICENSE.contains("SIL OPEN FONT LICENSE Version 1.1"));
    assert_eq!(FontFace::for_swing_style(false, false), FontFace::Regular);
    assert_eq!(FontFace::for_swing_style(false, true), FontFace::Italic);
    assert_eq!(FontFace::for_swing_style(true, false), FontFace::SemiBold);
    assert_eq!(FontFace::for_swing_style(true, true), FontFace::BoldItalic);
    assert_eq!(
        fonts::font_id(FontFace::SemiBold, 13.0),
        egui::FontId::new(13.0, FontFamily::Name("Geist-SemiBold".into()))
    );

    let styles = fonts::text_styles();
    assert_eq!(styles[&egui::TextStyle::Body].size, fonts::BODY_SIZE);
    assert_eq!(styles[&egui::TextStyle::Button].size, 13.0);
    assert_eq!(styles[&egui::TextStyle::Small].size, 11.0);
    assert_eq!(
        styles[&egui::TextStyle::Heading],
        fonts::font_id(FontFace::SemiBold, 16.0)
    );
    assert_eq!(styles[&egui::TextStyle::Monospace].size, 12.0);
}
