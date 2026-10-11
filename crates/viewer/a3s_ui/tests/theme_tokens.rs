//! Theme token values and their egui mapping.

use a3s_ui::frames;
use a3s_ui::theme::{self, ThemeLibrary, ThemeMode, ThemePalette, ThemeShadow};
use egui::{Color32, CornerRadius, Margin, Stroke, Theme};

fn hex(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// (token, light, dark) — `ui/theme/ThemeLibrary.java:29-33,42-46`.
fn expected_palette() -> Vec<(&'static str, Color32, Color32)> {
    vec![
        ("windowBackground", hex(0xF4F6FB), hex(0x10151E)),
        ("surface", hex(0xFFFFFF), hex(0x1B222C)),
        ("surfaceVariant", hex(0xF6F8FC), hex(0x242B38)),
        ("surfaceMuted", hex(0xE9EDF5), hex(0x2D3545)),
        ("border", hex(0xDFE4EF), hex(0x3A4250)),
        ("borderMuted", hex(0xE4E8F0), hex(0x2E3645)),
        ("textPrimary", hex(0x1E232C), hex(0xF3F4F6)),
        ("textSecondary", hex(0x4B5565), hex(0xD1D5DB)),
        ("textDisabled", hex(0x9CA3AF), hex(0x8B94A7)),
        ("iconDefault", hex(0x1F2937), hex(0xCBD5F5)),
        ("accentPrimary", hex(0x2563EB), hex(0x60A5FA)),
        ("accentSecondary", hex(0x7C3AED), hex(0xA78BFA)),
        ("statusSuccess", hex(0x16A34A), hex(0x22C55E)),
        ("statusWarning", hex(0xF59E0B), hex(0xF4A259)),
        ("statusDanger", hex(0xDC2626), hex(0xF87171)),
        ("statusInfo", hex(0x0284C7), hex(0x38BDF8)),
        (
            "stateHoverOverlay",
            Color32::from_rgba_unmultiplied(15, 23, 42, 24),
            Color32::from_rgba_unmultiplied(148, 163, 184, 36),
        ),
        ("stateFocus", hex(0x2563EB), hex(0x93C5FD)),
        ("stateSelectionBg", hex(0xDCE7FF), hex(0x1E3A8A)),
        ("stateSelectionFg", hex(0x1E3A8A), hex(0xE0EAFF)),
        ("stateDisabledBg", hex(0xE5E7EB), hex(0x1F2937)),
    ]
}

#[test]
fn palette_metrics_and_shadows() {
    let light = ThemeLibrary::light().palette.named_tokens();
    let dark = ThemeLibrary::dark().palette.named_tokens();
    let expected = expected_palette();
    assert_eq!(expected.len(), 21);
    for (i, (name, l, d)) in expected.into_iter().enumerate() {
        assert_eq!(light[i], (name, l), "light {name}");
        assert_eq!(dark[i], (name, d), "dark {name}");
    }
    assert_eq!(
        ThemeLibrary::light()
            .palette
            .state_hover_overlay
            .to_srgba_unmultiplied()[3],
        24
    );
    assert_eq!(
        ThemeLibrary::dark()
            .palette
            .state_hover_overlay
            .to_srgba_unmultiplied()[3],
        36
    );

    for def in [ThemeLibrary::light(), ThemeLibrary::dark()] {
        let m = &def.metrics;
        assert_eq!(
            [
                m.spacing_xs,
                m.spacing_sm,
                m.spacing_md,
                m.spacing_lg,
                m.spacing_xl,
                m.spacing_xxl,
                m.radius_sm,
                m.radius_md,
                m.radius_lg,
                m.focus_width
            ],
            [4, 8, 12, 16, 24, 32, 8, 12, 18, 1]
        );
    }

    let check = |s: &ThemeShadow, color: u32, opacity: f32, y: i32, blur: i32, alpha: u8| {
        assert_eq!(s.color(), hex(color));
        assert!((s.opacity() - opacity).abs() < f32::EPSILON);
        assert_eq!((s.y_offset(), s.blur_radius()), (y, blur));
        let egui = s.to_egui();
        assert_eq!(egui.offset, [0, y as i8]);
        assert_eq!((egui.blur, egui.spread), (blur as u8, 0));
        assert_eq!(egui.color.a(), alpha);
    };
    let l = &ThemeLibrary::light().metrics;
    let d = &ThemeLibrary::dark().metrics;
    check(&l.shadow_low, 0x1F2937, 0.10, 1, 6, 26);
    check(&l.shadow_medium, 0x111827, 0.14, 4, 16, 36);
    check(&d.shadow_low, 0x000000, 0.40, 2, 12, 102);
    check(&d.shadow_medium, 0x000000, 0.55, 6, 28, 140);

    let clamped = ThemeShadow::new(Color32::BLACK, 1.7, 0, -3);
    assert_eq!(clamped.opacity(), 1.0);
    assert_eq!(clamped.blur_radius(), 0);
    assert_eq!(ThemeShadow::new(Color32::BLACK, -0.5, 0, 3).opacity(), 0.0);
}

#[test]
fn palette_aliases() {
    let p: &ThemePalette = &ThemeLibrary::dark().palette;
    assert_eq!(p.selection_background(), p.state_selection_bg);
    assert_eq!(p.selection_foreground(), p.state_selection_fg);
    assert_eq!(p.disabled_surface(), p.state_disabled_bg);
    assert_eq!(
        p.status_color(theme::ProgressTone::Warning),
        p.status_warning
    );
}

#[test]
fn default_dark_and_toggle() {
    assert_eq!(ThemeMode::default(), ThemeMode::Dark);
    assert_eq!(ThemeLibrary::for_mode(ThemeMode::Light).id, "light");
    assert_eq!(ThemeLibrary::for_mode(ThemeMode::Dark).id, "dark");
    assert!(ThemeLibrary::dark().dark);
    assert!(!ThemeLibrary::light().dark);

    let ctx = egui::Context::default();
    a3s_ui::install(&ctx);
    assert_eq!(theme::current(&ctx).id, "dark");
    assert_eq!(theme::toggle(&ctx), ThemeMode::Light);
    assert_eq!(theme::current(&ctx).id, "light");
    assert_eq!(theme::toggle(&ctx), ThemeMode::Dark);
    assert_eq!(theme::current(&ctx).id, "dark");
    theme::set_mode(&ctx, ThemeMode::Light);
    assert_eq!(theme::current(&ctx).id, "light");
}

#[test]
fn style_mapping() {
    for def in [ThemeLibrary::light(), ThemeLibrary::dark()] {
        let p = &def.palette;
        let style = theme::style_for(def);
        let v = &style.visuals;
        let six = CornerRadius::same(6);

        assert_eq!(v.dark_mode, def.dark);
        assert_eq!(v.panel_fill, p.surface);
        assert_eq!(v.window_fill, p.surface);
        assert_eq!(v.window_stroke, Stroke::new(1.0, p.border));
        assert_eq!(v.window_corner_radius, six);
        assert_eq!(v.menu_corner_radius, six);
        assert_eq!(v.window_shadow, def.metrics.shadow_medium.to_egui());
        assert_eq!(v.popup_shadow, def.metrics.shadow_low.to_egui());
        assert_eq!(v.faint_bg_color, p.surface_variant);
        assert_eq!(v.extreme_bg_color, p.surface);
        assert_eq!(v.text_edit_bg_color, Some(p.surface));
        assert_eq!(v.code_bg_color, p.surface_muted);
        assert_eq!(v.hyperlink_color, p.accent_primary);
        assert_eq!(v.warn_fg_color, p.status_warning);
        assert_eq!(v.error_fg_color, p.status_danger);
        assert_eq!(v.weak_text_color, Some(p.text_secondary));
        assert_eq!(v.override_text_color, None);
        assert_eq!(v.selection.bg_fill, p.state_selection_bg);
        assert_eq!(v.selection.stroke, Stroke::new(1.0, p.state_selection_fg));
        assert_eq!(v.text_cursor.stroke, Stroke::new(2.0, p.accent_primary));
        assert!(v.striped);
        assert!(!v.indent_has_left_vline);
        assert!(!v.window_highlight_topmost);
        assert_eq!(v.disabled_alpha, 0.5);

        assert_eq!(theme::widget_corner_radius(&def.metrics), six);
        assert_eq!(
            theme::text_edit_corner_radius(&def.metrics),
            CornerRadius::same(4)
        );

        let w = &v.widgets;
        let rows = [
            (
                &w.noninteractive,
                p.surface,
                Stroke::new(1.0, p.border_muted),
                Stroke::new(1.0, p.text_primary),
            ),
            (
                &w.inactive,
                p.surface_variant,
                Stroke::new(1.0, p.border),
                Stroke::new(1.0, p.text_primary),
            ),
            (
                &w.hovered,
                p.state_hover_overlay,
                Stroke::new(1.0, p.border),
                Stroke::new(1.5, p.text_primary),
            ),
            (
                &w.active,
                p.state_selection_bg,
                Stroke::new(1.0, p.state_focus),
                Stroke::new(2.0, p.text_primary),
            ),
            (
                &w.open,
                p.surface_variant,
                Stroke::new(1.0, p.border),
                Stroke::new(1.0, p.text_primary),
            ),
        ];
        for (visuals, fill, bg_stroke, fg_stroke) in rows {
            assert_eq!(visuals.bg_fill, fill);
            assert_eq!(visuals.weak_bg_fill, fill);
            assert_eq!(visuals.bg_stroke, bg_stroke);
            assert_eq!(visuals.fg_stroke, fg_stroke);
            assert_eq!(visuals.corner_radius, six);
            assert_eq!(visuals.expansion, 0.0);
        }

        let s = &style.spacing;
        assert_eq!(s.item_spacing, egui::vec2(8.0, 4.0));
        assert_eq!(s.button_padding, egui::vec2(12.0, 4.0));
        assert_eq!(s.interact_size, egui::vec2(40.0, 24.0));
        assert_eq!(
            s.window_margin,
            Margin {
                left: 16,
                right: 16,
                top: 8,
                bottom: 8
            }
        );
        assert_eq!(s.menu_margin, Margin::same(4));
        assert_eq!(s.indent, 18.0);
        assert!(!s.scroll.floating);
        assert_eq!(s.scroll.bar_width, 10.0);
        assert_eq!(s.scroll.bar_inner_margin, 4.0);
    }
}

/// Ported `ThemeTokensTest.borderDefaultsUseBorderInstances`
/// (`src/test/java/fr/soe/a3s/ui/theme/ThemeTokensTest.java:12-25`).
#[test]
fn border_defaults_use_border_instances() {
    let expected_margin = Margin {
        left: 16,
        right: 16,
        top: 8,
        bottom: 8,
    };
    let style = theme::style_for(ThemeLibrary::dark());
    assert_eq!(style.spacing.window_margin, expected_margin);

    let frame = frames::table_scroll_frame(ThemeLibrary::dark());
    assert_eq!(frame.stroke, Stroke::NONE);
    assert_eq!(frame.inner_margin, Margin::ZERO);
    assert_eq!(frame.outer_margin, Margin::ZERO);

    let ctx = egui::Context::default();
    ctx.set_style_of(Theme::Dark, style);
    assert_eq!(
        ctx.style_of(Theme::Dark).spacing.window_margin,
        expected_margin
    );
    assert_eq!(
        frames::table_scroll_frame(theme::current(&ctx)).stroke,
        Stroke::NONE
    );
}
