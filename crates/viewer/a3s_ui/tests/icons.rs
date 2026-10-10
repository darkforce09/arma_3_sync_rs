//! Icon table, tinting, scaling and the app icon (spec B17–B23).

use a3s_ui::icon::{self, IconState, IconTone, UiIcon};
use a3s_ui::theme::ThemeLibrary;
use egui::Color32;

/// `ui/icon/UiIcon.java:4-45`.
const TABLE: [(UiIcon, &str, bool, IconTone); 42] = [
    (UiIcon::App, "app", false, IconTone::Accent),
    (UiIcon::Community, "community", false, IconTone::Accent),
    (UiIcon::Add, "add", true, IconTone::Primary),
    (
        UiIcon::ArrowDoubleUp,
        "arrow-double-up",
        true,
        IconTone::Primary,
    ),
    (UiIcon::ArrowUp, "arrow-up", true, IconTone::Primary),
    (UiIcon::ArrowDown, "arrow-down", true, IconTone::Primary),
    (UiIcon::Check, "check", true, IconTone::Success),
    (UiIcon::Close, "close", true, IconTone::Danger),
    (UiIcon::CloseStrong, "close-strong", true, IconTone::Danger),
    (UiIcon::Delete, "delete", true, IconTone::Danger),
    (UiIcon::Duplicate, "duplicate", true, IconTone::Primary),
    (UiIcon::Donate, "donate", true, IconTone::Accent),
    (UiIcon::Download, "download", true, IconTone::Primary),
    (UiIcon::Edit, "edit", true, IconTone::Primary),
    (UiIcon::Events, "events", true, IconTone::Primary),
    (UiIcon::File, "file", true, IconTone::Primary),
    (UiIcon::Folder, "folder", false, IconTone::Warning),
    (UiIcon::FolderOpen, "folder-open", false, IconTone::Warning),
    (UiIcon::Globe, "globe", true, IconTone::Info),
    (UiIcon::Help, "help", true, IconTone::Info),
    (UiIcon::Info, "info", true, IconTone::Info),
    (UiIcon::Key, "key", true, IconTone::Primary),
    (UiIcon::Link, "link", true, IconTone::Info),
    (UiIcon::Moon, "moon", false, IconTone::Primary),
    (UiIcon::Package, "package", true, IconTone::Muted),
    (UiIcon::Pause, "pause", true, IconTone::Primary),
    (UiIcon::Play, "play", true, IconTone::Success),
    (UiIcon::Power, "power", true, IconTone::Accent),
    (UiIcon::Proxy, "proxy", true, IconTone::Primary),
    (UiIcon::Radio, "radio", true, IconTone::Primary),
    (UiIcon::Refresh, "refresh", true, IconTone::Info),
    (UiIcon::Repository, "repository", true, IconTone::Primary),
    (UiIcon::Report, "report", true, IconTone::Info),
    (UiIcon::Save, "save", true, IconTone::Primary),
    (UiIcon::Settings, "settings", true, IconTone::Primary),
    (UiIcon::Shield, "shield", true, IconTone::Info),
    (UiIcon::Shortcut, "shortcut", true, IconTone::Primary),
    (UiIcon::Stop, "stop", true, IconTone::Danger),
    (UiIcon::Sun, "sun", false, IconTone::Warning),
    (UiIcon::Update, "update", true, IconTone::Info),
    (UiIcon::Upload, "upload", true, IconTone::Accent),
    (UiIcon::Warning, "warning", true, IconTone::Warning),
];

#[test]
fn a5_icon_table_matches_java() {
    assert_eq!(UiIcon::ALL.len(), 42);
    for (i, (icon, stem, aware, tone)) in TABLE.into_iter().enumerate() {
        assert_eq!(UiIcon::ALL[i], icon, "order at {i}");
        assert_eq!(icon.file_name(), stem);
        assert_eq!(icon.is_theme_aware(), aware, "{stem}");
        assert_eq!(icon.tone(), tone, "{stem}");
        assert!(icon.svg_bytes().starts_with(b"<"), "{stem}.svg is embedded");
    }
    let not_aware: Vec<_> = UiIcon::ALL
        .into_iter()
        .filter(|icon| !icon.is_theme_aware())
        .collect();
    assert_eq!(
        not_aware,
        [
            UiIcon::App,
            UiIcon::Community,
            UiIcon::Folder,
            UiIcon::FolderOpen,
            UiIcon::Moon,
            UiIcon::Sun
        ]
    );
}

#[test]
fn a6_tint_for_follows_tone_and_state() {
    let light = &ThemeLibrary::light().palette;
    let dark = &ThemeLibrary::dark().palette;
    assert_eq!(
        icon::tint_for(light, UiIcon::Add, IconState::Normal),
        Some(Color32::from_rgb(0x1E, 0x23, 0x2C))
    );
    for p in [light, dark] {
        assert_eq!(icon::tint_for(p, UiIcon::Folder, IconState::Normal), None);
        assert_eq!(icon::tint_for(p, UiIcon::Folder, IconState::Selected), None);
        assert_eq!(
            icon::tint_for(p, UiIcon::Folder, IconState::Disabled),
            Some(p.text_disabled)
        );
        assert_eq!(
            icon::tint_for(p, UiIcon::Package, IconState::Normal),
            Some(p.text_secondary)
        );
    }
    assert_eq!(
        icon::tint_for(dark, UiIcon::Check, IconState::Selected),
        Some(Color32::from_rgb(0xE0, 0xEA, 0xFF))
    );

    for p in [light, dark] {
        for icon in UiIcon::ALL {
            let expected_normal = icon.is_theme_aware().then(|| match icon.tone() {
                IconTone::Accent => p.accent_primary,
                IconTone::Danger => p.status_danger,
                IconTone::Info => p.status_info,
                IconTone::Muted => p.text_secondary,
                IconTone::Success => p.status_success,
                IconTone::Warning => p.status_warning,
                IconTone::Primary => p.text_primary,
            });
            assert_eq!(icon::tint_for(p, icon, IconState::Normal), expected_normal);
            // The app icon is always the untinted PNG (Java `Icons.image(APP)`).
            let app = icon == UiIcon::App;
            assert_eq!(
                icon::tint_for(p, icon, IconState::Disabled),
                (!app).then_some(p.text_disabled)
            );
            assert_eq!(
                icon::tint_for(p, icon, IconState::Selected),
                icon.is_theme_aware().then_some(p.state_selection_fg)
            );
        }
        assert_eq!(icon::tint_for(p, UiIcon::App, IconState::Normal), None);
        assert_eq!(icon::tint_for(p, UiIcon::App, IconState::Disabled), None);
    }
}

#[test]
fn a7_scaled_size() {
    for (size, expected) in [
        (12, 18),
        (16, 24),
        (18, 27),
        (5, 8),
        (1, 8),
        (64, 96),
        (65, 65),
        (128, 128),
        (256, 256),
    ] {
        assert_eq!(icon::scaled_size(size), expected, "size {size}");
    }
}

#[test]
fn a9_app_icon_data() {
    let small = icon::app_icon_data(UiIcon::WINDOW_ICON_SIZE);
    assert_eq!((small.width, small.height), (24, 24));
    assert_eq!(small.rgba.len(), 24 * 24 * 4);
    let centre = (12 * 24 + 12) * 4;
    assert!(small.rgba[centre + 3] > 0, "centre pixel is visible");

    let big = icon::app_icon_data(UiIcon::PICTURE_SIZE);
    assert_eq!((big.width, big.height), (256, 256));
    assert_eq!(big.rgba.len(), 256 * 256 * 4);
}

#[test]
fn a8_icon_image_is_sized_in_points() {
    let ctx = egui::Context::default();
    a3s_ui::install(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let image = icon::icon_image(ui.ctx(), UiIcon::Folder, 18, IconState::Normal);
        assert_eq!(image.size(), Some(egui::vec2(27.0, 27.0)));
        let crest = icon::icon_image(ui.ctx(), UiIcon::Community, 18, IconState::Normal);
        let size = crest.size().expect("sized texture");
        assert!((size.y - 27.0).abs() < 0.01 && size.x < size.y);
        // Cached: the same texture comes back.
        let a = icon::icon_texture(ui.ctx(), UiIcon::Add, 16, IconState::Normal);
        let b = icon::icon_texture(ui.ctx(), UiIcon::Add, 16, IconState::Normal);
        assert_eq!(a.id(), b.id());
        icon::invalidate(ui.ctx());
        let c = icon::icon_texture(ui.ctx(), UiIcon::Add, 16, IconState::Normal);
        assert_ne!(a.id(), c.id());
    });
    output.textures_delta.clear();
}
