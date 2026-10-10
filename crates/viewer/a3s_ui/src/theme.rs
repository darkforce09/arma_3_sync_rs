//! Theme tokens (palette, metrics, shadows) for the light and dark themes, and their mapping
//! onto [`egui::Style`].
//!
//! Ports `ui/theme/ThemePalette.java`, `ThemeMetrics.java`, `ThemeShadow.java`,
//! `ThemeDefinition.java`, `ThemeLibrary.java`, the accessors of `ThemeTokens.java` /
//! `ui/UiColors.java` and the light/dark switching of `ui/ThemeManager.java`. Persisting the
//! chosen mode is the caller's job.

mod style;

use std::sync::LazyLock;

use egui::{Color32, Context, Shadow};

pub use self::style::{style_for, text_edit_corner_radius, visuals_for, widget_corner_radius};

/// The 21 colour tokens of a theme, in `ThemePalette` constructor order
/// (`ui/theme/ThemePalette.java:10-58`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    /// Window background. Defined but never read by the Java UI.
    pub window_background: Color32,
    /// Default panel / component surface.
    pub surface: Color32,
    /// Slightly raised surface (buttons, alternate table rows, status bar).
    pub surface_variant: Color32,
    /// Muted surface (scroll and progress track).
    pub surface_muted: Color32,
    /// Default border.
    pub border: Color32,
    /// Subtle border (separators, progress outline, pane divider).
    pub border_muted: Color32,
    /// Primary text.
    pub text_primary: Color32,
    /// Secondary text (tabs, table headers, menu bar).
    pub text_secondary: Color32,
    /// Disabled text and disabled icon tint.
    pub text_disabled: Color32,
    /// Default icon colour. Defined but never read by the Java UI (PRIMARY icons use
    /// [`Self::text_primary`]).
    pub icon_default: Color32,
    /// Primary accent (default buttons, links, caret).
    pub accent_primary: Color32,
    /// Secondary accent (default-button hover).
    pub accent_secondary: Color32,
    /// Success status.
    pub status_success: Color32,
    /// Warning status.
    pub status_warning: Color32,
    /// Danger status.
    pub status_danger: Color32,
    /// Info status.
    pub status_info: Color32,
    /// Translucent hover overlay.
    pub state_hover_overlay: Color32,
    /// Focus ring.
    pub state_focus: Color32,
    /// Selection background.
    pub state_selection_bg: Color32,
    /// Selection foreground.
    pub state_selection_fg: Color32,
    /// Disabled component background.
    pub state_disabled_bg: Color32,
}

impl ThemePalette {
    /// All 21 tokens with their Java names, in constructor order.
    pub fn named_tokens(&self) -> [(&'static str, Color32); 21] {
        [
            ("windowBackground", self.window_background),
            ("surface", self.surface),
            ("surfaceVariant", self.surface_variant),
            ("surfaceMuted", self.surface_muted),
            ("border", self.border),
            ("borderMuted", self.border_muted),
            ("textPrimary", self.text_primary),
            ("textSecondary", self.text_secondary),
            ("textDisabled", self.text_disabled),
            ("iconDefault", self.icon_default),
            ("accentPrimary", self.accent_primary),
            ("accentSecondary", self.accent_secondary),
            ("statusSuccess", self.status_success),
            ("statusWarning", self.status_warning),
            ("statusDanger", self.status_danger),
            ("statusInfo", self.status_info),
            ("stateHoverOverlay", self.state_hover_overlay),
            ("stateFocus", self.state_focus),
            ("stateSelectionBg", self.state_selection_bg),
            ("stateSelectionFg", self.state_selection_fg),
            ("stateDisabledBg", self.state_disabled_bg),
        ]
    }

    /// Alias of [`Self::state_selection_bg`]. Mirrors `ui/UiColors.java:73`.
    pub fn selection_background(&self) -> Color32 {
        self.state_selection_bg
    }

    /// Alias of [`Self::state_selection_fg`]. Mirrors `ui/UiColors.java:77`.
    pub fn selection_foreground(&self) -> Color32 {
        self.state_selection_fg
    }

    /// Alias of [`Self::state_disabled_bg`]. Mirrors `ui/UiColors.java:81`.
    pub fn disabled_surface(&self) -> Color32 {
        self.state_disabled_bg
    }

    /// Colour of a progress / status tone. Mirrors `ui/UiStyle.java:148-161,223-225`.
    pub fn status_color(&self, tone: ProgressTone) -> Color32 {
        match tone {
            ProgressTone::Success => self.status_success,
            ProgressTone::Info => self.status_info,
            ProgressTone::Warning => self.status_warning,
            ProgressTone::Danger => self.status_danger,
        }
    }
}

/// A drop shadow description. Mirrors `ui/theme/ThemeShadow.java`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeShadow {
    color: Color32,
    opacity: f32,
    y_offset: i32,
    blur_radius: i32,
}

impl ThemeShadow {
    /// Creates a shadow; `opacity` is clamped to `[0, 1]` and `blur_radius` to `>= 0`.
    ///
    /// Mirrors `ui/theme/ThemeShadow.java:16-21`.
    pub fn new(color: Color32, opacity: f32, y_offset: i32, blur_radius: i32) -> Self {
        Self {
            color,
            opacity: opacity.clamp(0.0, 1.0),
            y_offset,
            blur_radius: blur_radius.max(0),
        }
    }

    /// Shadow colour (opaque RGB).
    pub fn color(&self) -> Color32 {
        self.color
    }

    /// Opacity in `[0, 1]`.
    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Vertical offset in points.
    pub fn y_offset(&self) -> i32 {
        self.y_offset
    }

    /// Blur radius in points, never negative.
    pub fn blur_radius(&self) -> i32 {
        self.blur_radius
    }

    /// Converts to an egui shadow: offset `[0, y]`, blur = radius, no spread, colour with alpha
    /// `round(opacity * 255)`.
    pub fn to_egui(&self) -> Shadow {
        let [r, g, b, _] = self.color.to_array();
        let alpha = (self.opacity * 255.0).round() as u8;
        Shadow {
            offset: [0, saturate_i8(self.y_offset)],
            blur: self.blur_radius.clamp(0, i32::from(u8::MAX)) as u8,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(r, g, b, alpha),
        }
    }
}

fn saturate_i8(value: i32) -> i8 {
    value.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8
}

/// Spacing, radius and effect tokens. Mirrors `ui/theme/ThemeMetrics.java:8-19`.
///
/// Values are integers as in Java; convert with `as f32` where egui needs floats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    /// 4
    pub spacing_xs: i32,
    /// 8
    pub spacing_sm: i32,
    /// 12
    pub spacing_md: i32,
    /// 16
    pub spacing_lg: i32,
    /// 24
    pub spacing_xl: i32,
    /// 32
    pub spacing_xxl: i32,
    /// 8 (a `FlatLaf` arc, i.e. a diameter)
    pub radius_sm: i32,
    /// 12 (a `FlatLaf` arc, i.e. a diameter)
    pub radius_md: i32,
    /// 18 (unused by Java)
    pub radius_lg: i32,
    /// 1
    pub focus_width: i32,
    /// Low elevation shadow (popups).
    pub shadow_low: ThemeShadow,
    /// Medium elevation shadow (windows).
    pub shadow_medium: ThemeShadow,
}

/// A complete theme: id, dark flag, palette and metrics. Mirrors
/// `ui/theme/ThemeDefinition.java:8-18`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeDefinition {
    /// `"light"` or `"dark"`.
    pub id: &'static str,
    /// Whether this is the dark theme.
    pub dark: bool,
    /// Colour tokens.
    pub palette: ThemePalette,
    /// Spacing and effect tokens.
    pub metrics: ThemeMetrics,
}

impl ThemeDefinition {
    /// The [`ThemeMode`] this definition belongs to.
    pub fn mode(&self) -> ThemeMode {
        if self.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        }
    }
}

/// Light or dark mode. The default is [`ThemeMode::Dark`] (`ui/ThemeManager.java:20,34`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    /// The light theme.
    Light,
    /// The dark theme (default).
    #[default]
    Dark,
}

impl ThemeMode {
    /// The other mode.
    pub fn toggled(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    /// Whether this is [`ThemeMode::Dark`].
    pub fn is_dark(self) -> bool {
        self == Self::Dark
    }
}

impl From<ThemeMode> for egui::Theme {
    fn from(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::Light,
            ThemeMode::Dark => Self::Dark,
        }
    }
}

impl From<egui::Theme> for ThemeMode {
    fn from(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Light => Self::Light,
            egui::Theme::Dark => Self::Dark,
        }
    }
}

/// Tone of a progress bar or status text. Mirrors `ui/UiStyle.java:26-28`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ProgressTone {
    /// `statusSuccess`.
    Success,
    /// `statusInfo` (default).
    #[default]
    Info,
    /// `statusWarning`.
    Warning,
    /// `statusDanger`.
    Danger,
}

impl ProgressTone {
    /// All tones in Java declaration order.
    pub const ALL: [Self; 4] = [Self::Success, Self::Info, Self::Warning, Self::Danger];
}

/// The canonical light and dark theme definitions. Mirrors `ui/theme/ThemeLibrary.java`.
pub struct ThemeLibrary;

static LIGHT: LazyLock<ThemeDefinition> = LazyLock::new(build_light);
static DARK: LazyLock<ThemeDefinition> = LazyLock::new(build_dark);

impl ThemeLibrary {
    /// The light theme. Mirrors `ui/theme/ThemeLibrary.java:16-18`.
    pub fn light() -> &'static ThemeDefinition {
        &LIGHT
    }

    /// The dark theme. Mirrors `ui/theme/ThemeLibrary.java:20-22`.
    pub fn dark() -> &'static ThemeDefinition {
        &DARK
    }

    /// The theme for `mode`. Mirrors `ui/theme/ThemeLibrary.java:24-26`.
    pub fn for_mode(mode: ThemeMode) -> &'static ThemeDefinition {
        match mode {
            ThemeMode::Light => Self::light(),
            ThemeMode::Dark => Self::dark(),
        }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

// Mirrors ui/theme/ThemeLibrary.java:28-39
fn build_light() -> ThemeDefinition {
    let palette = ThemePalette {
        window_background: rgb(0xF4F6FB),
        surface: rgb(0xFFFFFF),
        surface_variant: rgb(0xF6F8FC),
        surface_muted: rgb(0xE9EDF5),
        border: rgb(0xDFE4EF),
        border_muted: rgb(0xE4E8F0),
        text_primary: rgb(0x1E232C),
        text_secondary: rgb(0x4B5565),
        text_disabled: rgb(0x9CA3AF),
        icon_default: rgb(0x1F2937),
        accent_primary: rgb(0x2563EB),
        accent_secondary: rgb(0x7C3AED),
        status_success: rgb(0x16A34A),
        status_warning: rgb(0xF59E0B),
        status_danger: rgb(0xDC2626),
        status_info: rgb(0x0284C7),
        state_hover_overlay: Color32::from_rgba_unmultiplied(15, 23, 42, 24),
        state_focus: rgb(0x2563EB),
        state_selection_bg: rgb(0xDCE7FF),
        state_selection_fg: rgb(0x1E3A8A),
        state_disabled_bg: rgb(0xE5E7EB),
    };
    let metrics = metrics(
        ThemeShadow::new(rgb(0x1F2937), 0.1, 1, 6),
        ThemeShadow::new(rgb(0x111827), 0.14, 4, 16),
    );
    ThemeDefinition {
        id: "light",
        dark: false,
        palette,
        metrics,
    }
}

// Mirrors ui/theme/ThemeLibrary.java:41-52
fn build_dark() -> ThemeDefinition {
    let palette = ThemePalette {
        window_background: rgb(0x10151E),
        surface: rgb(0x1B222C),
        surface_variant: rgb(0x242B38),
        surface_muted: rgb(0x2D3545),
        border: rgb(0x3A4250),
        border_muted: rgb(0x2E3645),
        text_primary: rgb(0xF3F4F6),
        text_secondary: rgb(0xD1D5DB),
        text_disabled: rgb(0x8B94A7),
        icon_default: rgb(0xCBD5F5),
        accent_primary: rgb(0x60A5FA),
        accent_secondary: rgb(0xA78BFA),
        status_success: rgb(0x22C55E),
        status_warning: rgb(0xF4A259),
        status_danger: rgb(0xF87171),
        status_info: rgb(0x38BDF8),
        state_hover_overlay: Color32::from_rgba_unmultiplied(148, 163, 184, 36),
        state_focus: rgb(0x93C5FD),
        state_selection_bg: rgb(0x1E3A8A),
        state_selection_fg: rgb(0xE0EAFF),
        state_disabled_bg: rgb(0x1F2937),
    };
    let metrics = metrics(
        ThemeShadow::new(rgb(0x000000), 0.4, 2, 12),
        ThemeShadow::new(rgb(0x000000), 0.55, 6, 28),
    );
    ThemeDefinition {
        id: "dark",
        dark: true,
        palette,
        metrics,
    }
}

// Mirrors ui/theme/ThemeLibrary.java:35,48 (identical in both themes)
fn metrics(shadow_low: ThemeShadow, shadow_medium: ThemeShadow) -> ThemeMetrics {
    ThemeMetrics {
        spacing_xs: 4,
        spacing_sm: 8,
        spacing_md: 12,
        spacing_lg: 16,
        spacing_xl: 24,
        spacing_xxl: 32,
        radius_sm: 8,
        radius_md: 12,
        radius_lg: 18,
        focus_width: 1,
        shadow_low,
        shadow_medium,
    }
}

/// The definition matching the context's active theme (`ThemeTokens.theme()`).
pub fn current(ctx: &Context) -> &'static ThemeDefinition {
    ThemeLibrary::for_mode(ctx.theme().into())
}

/// The active palette (`ThemeTokens.colors()`).
pub fn palette(ctx: &Context) -> &'static ThemePalette {
    &current(ctx).palette
}

/// The active metrics (`ThemeTokens.metrics()`).
pub fn metrics_of(ctx: &Context) -> &'static ThemeMetrics {
    &current(ctx).metrics
}

/// Switches the context to `mode`.
pub fn set_mode(ctx: &Context, mode: ThemeMode) {
    ctx.set_theme(egui::Theme::from(mode));
}

/// Flips between light and dark and returns the new mode. Mirrors
/// `ui/ThemeManager.java:41-58` without the preference storage and the snapshot animation.
///
/// Icon textures are cached per theme, so no explicit invalidation is needed.
pub fn toggle(ctx: &Context) -> ThemeMode {
    let mode = ThemeMode::from(ctx.theme()).toggled();
    set_mode(ctx, mode);
    mode
}
