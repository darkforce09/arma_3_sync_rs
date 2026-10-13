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

pub use self::style::{
    style_before_fonts, style_for, text_edit_corner_radius, visuals_for, widget_corner_radius,
};

/// The 21 colour tokens of a theme. Values follow the macOS system colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    /// Window and panel background.
    pub window_background: Color32,
    /// Sidebar and toolbar background.
    pub sidebar: Color32,
    /// Content surface: grouped boxes, lists, text fields, sheets.
    pub surface: Color32,
    /// Alternate list rows and the status bar.
    pub surface_variant: Color32,
    /// Muted surface (scroll and progress track, code background).
    pub surface_muted: Color32,
    /// Push buttons, pop-up buttons and the selected segment.
    pub control: Color32,
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
}

impl ThemePalette {
    /// All 21 tokens with their names, in declaration order.
    pub fn named_tokens(&self) -> [(&'static str, Color32); 21] {
        [
            ("windowBackground", self.window_background),
            ("sidebar", self.sidebar),
            ("surface", self.surface),
            ("surfaceVariant", self.surface_variant),
            ("surfaceMuted", self.surface_muted),
            ("control", self.control),
            ("border", self.border),
            ("borderMuted", self.border_muted),
            ("textPrimary", self.text_primary),
            ("textSecondary", self.text_secondary),
            ("textDisabled", self.text_disabled),
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

// macOS light appearance (system colours).
fn build_light() -> ThemeDefinition {
    let palette = ThemePalette {
        window_background: rgb(0xECECEC),
        sidebar: rgb(0xE3E3E3),
        surface: rgb(0xFFFFFF),
        surface_variant: rgb(0xF4F5F5),
        surface_muted: rgb(0xE5E5EA),
        control: rgb(0xFFFFFF),
        border: rgb(0xC8C8CC),
        border_muted: rgb(0xE5E5EA),
        text_primary: rgb(0x1D1D1F),
        text_secondary: rgb(0x6E6E73),
        text_disabled: rgb(0xAEAEB2),
        accent_primary: rgb(0x007AFF),
        accent_secondary: rgb(0x0062CC),
        status_success: rgb(0x34C759),
        status_warning: rgb(0xFF9500),
        status_danger: rgb(0xFF3B30),
        status_info: rgb(0x007AFF),
        state_hover_overlay: Color32::from_rgba_unmultiplied(0, 0, 0, 13),
        state_focus: rgb(0x7FBCFF),
        state_selection_bg: rgb(0x0064E1),
        state_selection_fg: rgb(0xFFFFFF),
    };
    let metrics = metrics(
        ThemeShadow::new(rgb(0x000000), 0.15, 2, 8),
        ThemeShadow::new(rgb(0x000000), 0.25, 8, 24),
    );
    ThemeDefinition {
        id: "light",
        dark: false,
        palette,
        metrics,
    }
}

// macOS dark appearance (system colours).
fn build_dark() -> ThemeDefinition {
    let palette = ThemePalette {
        window_background: rgb(0x1F1F1F),
        sidebar: rgb(0x2A2A2C),
        surface: rgb(0x2C2C2E),
        surface_variant: rgb(0x262628),
        surface_muted: rgb(0x3A3A3C),
        control: rgb(0x48484A),
        border: rgb(0x3E3E40),
        border_muted: rgb(0x343436),
        text_primary: rgb(0xECECEC),
        text_secondary: rgb(0x98989D),
        text_disabled: rgb(0x636366),
        accent_primary: rgb(0x0A84FF),
        accent_secondary: rgb(0x409CFF),
        status_success: rgb(0x30D158),
        status_warning: rgb(0xFF9F0A),
        status_danger: rgb(0xFF453A),
        status_info: rgb(0x0A84FF),
        state_hover_overlay: Color32::from_rgba_unmultiplied(255, 255, 255, 15),
        state_focus: rgb(0x409CFF),
        state_selection_bg: rgb(0x0059D1),
        state_selection_fg: rgb(0xFFFFFF),
    };
    let metrics = metrics(
        ThemeShadow::new(rgb(0x000000), 0.4, 2, 10),
        ThemeShadow::new(rgb(0x000000), 0.5, 10, 30),
    );
    ThemeDefinition {
        id: "dark",
        dark: true,
        palette,
        metrics,
    }
}

// Identical in both themes. Radii are arcs (diameters), halved for egui.
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
