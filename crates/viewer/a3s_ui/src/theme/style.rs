//! Token → egui mapping, the egui counterpart of `ThemeTokens.applyToUiDefaults`
//! (`ui/theme/ThemeTokens.java:37-177`).

use egui::style::{ScrollStyle, Selection, TextCursorStyle, WidgetVisuals, Widgets};
use egui::{CornerRadius, Margin, Stroke, Style, Visuals, vec2};

use super::{ThemeDefinition, ThemeMetrics, ThemePalette};
use crate::fonts;

/// `FlatLaf` `*.arc` keys are diameters; egui corner radii are radii.
fn arc_to_radius(arc: i32) -> CornerRadius {
    CornerRadius::same((arc / 2).clamp(0, i32::from(u8::MAX)) as u8)
}

fn margin(value: i32) -> i8 {
    value.clamp(0, i32::from(i8::MAX)) as i8
}

/// Builds the complete egui style of a theme: visuals, spacing and text styles.
///
/// The heading text style uses the `Geist-SemiBold` font family, so the context must have the
/// Geist fonts loaded (egui panics on unknown families). Prefer [`crate::install`], called
/// before the first frame, which handles the one pass in which the fonts are not loaded yet.
pub fn style_for(def: &ThemeDefinition) -> Style {
    let m = &def.metrics;
    let mut style = Style {
        visuals: visuals_for(def),
        text_styles: fonts::text_styles(),
        ..Style::default()
    };

    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(m.spacing_sm as f32, m.spacing_xs as f32);
    spacing.button_padding = vec2(m.spacing_md as f32, m.spacing_xs as f32);
    // Table.rowHeight / Tree.rowHeight / TabbedPane.tabHeight / TableHeader.height
    // (ui/theme/ThemeTokens.java:111,126,133,144).
    spacing.interact_size = vec2(40.0, m.spacing_xl as f32);
    // OptionPane.messageAreaBorder = EmptyBorder(sm, lg, sm, lg) (ui/theme/ThemeTokens.java:83-84).
    spacing.window_margin = Margin {
        left: margin(m.spacing_lg),
        right: margin(m.spacing_lg),
        top: margin(m.spacing_sm),
        bottom: margin(m.spacing_sm),
    };
    spacing.menu_margin = Margin::same(margin(m.spacing_xs));
    // FlatLaf tree indent: 7 (left child indent) + 11 (right child indent).
    spacing.indent = 18.0;
    // Java scroll bars are solid. `ScrollBar.track` (surfaceMuted) has no egui counterpart: egui
    // paints the scroll track with `extreme_bg_color`, which must stay `surface` for text fields,
    // so the track colour token is not mapped.
    spacing.scroll = ScrollStyle {
        floating: false,
        bar_width: 10.0,
        // ScrollBar.trackInsets (ui/theme/ThemeTokens.java:105).
        bar_inner_margin: m.spacing_xs as f32,
        ..ScrollStyle::solid()
    };
    style
}

/// Builds the egui visuals of a theme (`ui/theme/ThemeTokens.java:37-177`).
pub fn visuals_for(def: &ThemeDefinition) -> Visuals {
    let p = &def.palette;
    let m = &def.metrics;
    let base = if def.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let widget_radius = arc_to_radius(m.radius_md);

    Visuals {
        dark_mode: def.dark,
        override_text_color: None,
        weak_text_color: Some(p.text_secondary),
        widgets: widgets(p, m),
        selection: Selection {
            bg_fill: p.state_selection_bg,
            stroke: Stroke::new(1.0, p.state_selection_fg),
        },
        hyperlink_color: p.accent_primary,
        faint_bg_color: p.surface_variant,
        extreme_bg_color: p.surface,
        text_edit_bg_color: Some(p.surface),
        code_bg_color: p.surface_muted,
        warn_fg_color: p.status_warning,
        error_fg_color: p.status_danger,
        window_corner_radius: widget_radius,
        window_shadow: m.shadow_medium.to_egui(),
        window_fill: p.surface,
        window_stroke: Stroke::new(1.0, p.border),
        window_highlight_topmost: false,
        menu_corner_radius: widget_radius,
        panel_fill: p.surface,
        popup_shadow: m.shadow_low.to_egui(),
        text_cursor: TextCursorStyle {
            stroke: Stroke::new(2.0, p.accent_primary),
            ..TextCursorStyle::default()
        },
        indent_has_left_vline: false,
        striped: true,
        disabled_alpha: 0.5,
        ..base
    }
}

fn widgets(p: &ThemePalette, m: &ThemeMetrics) -> Widgets {
    let radius = arc_to_radius(m.radius_md);
    let visuals = |fill, bg_stroke, fg_stroke| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke,
        corner_radius: radius,
        fg_stroke,
        expansion: 0.0,
    };
    Widgets {
        noninteractive: WidgetVisuals {
            bg_fill: p.surface,
            weak_bg_fill: p.surface,
            ..visuals(
                p.surface,
                Stroke::new(1.0, p.border_muted),
                Stroke::new(1.0, p.text_primary),
            )
        },
        inactive: visuals(
            p.surface_variant,
            Stroke::new(1.0, p.border),
            Stroke::new(1.0, p.text_primary),
        ),
        hovered: visuals(
            p.state_hover_overlay,
            Stroke::new(1.0, p.border),
            Stroke::new(1.5, p.text_primary),
        ),
        active: visuals(
            p.state_selection_bg,
            Stroke::new(m.focus_width as f32, p.state_focus),
            Stroke::new(2.0, p.text_primary),
        ),
        open: visuals(
            p.surface_variant,
            Stroke::new(1.0, p.border),
            Stroke::new(1.0, p.text_primary),
        ),
    }
}

/// Corner radius for text edits (`TextComponent.arc` = radiusSm, halved): 4.
///
/// egui's `TextEdit` draws its frame with the widget radius; callers that want the `FlatLaf` text
/// arc draw their own frame with this radius.
pub fn text_edit_corner_radius(m: &ThemeMetrics) -> CornerRadius {
    arc_to_radius(m.radius_sm)
}

/// Corner radius of widgets, windows, menus and the scroll handle (`Component.arc`,
/// `Button.arc`, `ScrollBar.thumbArc` = radiusMd, halved): 6.
pub fn widget_corner_radius(m: &ThemeMetrics) -> CornerRadius {
    arc_to_radius(m.radius_md)
}

/// [`style_for`] with a proportional (Geist Regular / egui default) heading: safe while the
/// Geist fonts are not loaded yet. Used by [`crate::install`] for its first pass.
pub fn style_before_fonts(def: &ThemeDefinition) -> Style {
    let mut style = style_for(def);
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(fonts::HEADING_SIZE),
    );
    style
}
