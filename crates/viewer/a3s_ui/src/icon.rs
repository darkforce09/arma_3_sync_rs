//! The 42 bundled SVG icons, tone/state tinting, size scaling, a per-theme texture cache and
//! the application icon.
//!
//! Ports `ui/icon/UiIcon.java`, `IconTone.java`, `IconState.java` and `ui/icon/Icons.java`.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::load::{SizeHint, SizedTexture};
use egui::{Color32, ColorImage, Context, IconData, Id, TextureHandle, TextureOptions, vec2};
use resvg::tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use crate::theme::{ThemeLibrary, ThemeMode, ThemePalette};

/// Colour role of an icon. Mirrors `ui/icon/IconTone.java`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconTone {
    /// `textPrimary`.
    Primary,
    /// `textSecondary`.
    Muted,
    /// `accentPrimary`.
    Accent,
    /// `statusSuccess`.
    Success,
    /// `statusWarning`.
    Warning,
    /// `statusDanger`.
    Danger,
    /// `statusInfo`.
    Info,
}

/// Interaction state of an icon. Mirrors `ui/icon/IconState.java`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum IconState {
    /// Tone colour (theme-aware icons) or original colours.
    #[default]
    Normal,
    /// `textDisabled`, applied to every icon.
    Disabled,
    /// `stateSelectionFg`, applied to theme-aware icons only.
    Selected,
}

macro_rules! ui_icons {
    ($($variant:ident => $stem:literal, $aware:literal, $tone:ident;)*) => {
        /// The bundled icons, in Java declaration order (`ui/icon/UiIcon.java:4-45`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum UiIcon {
            $(
                #[doc = concat!("`", $stem, ".svg`")]
                $variant,
            )*
        }

        impl UiIcon {
            /// Every icon in Java declaration order.
            pub const ALL: [Self; 42] = [$(Self::$variant,)*];

            /// File stem of the SVG, e.g. `"folder-open"`.
            pub fn file_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $stem,)*
                }
            }

            /// Whether the icon is recoloured with its tone in the normal state.
            pub fn is_theme_aware(self) -> bool {
                match self {
                    $(Self::$variant => $aware,)*
                }
            }

            /// Colour role of the icon.
            pub fn tone(self) -> IconTone {
                match self {
                    $(Self::$variant => IconTone::$tone,)*
                }
            }

            /// The embedded SVG source.
            pub fn svg_bytes(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!("../assets/icons/", $stem, ".svg")),)*
                }
            }
        }
    };
}

ui_icons! {
    App => "app", false, Accent;
    Community => "community", false, Accent;
    Add => "add", true, Primary;
    ArrowDoubleUp => "arrow-double-up", true, Primary;
    ArrowUp => "arrow-up", true, Primary;
    ArrowDown => "arrow-down", true, Primary;
    Check => "check", true, Success;
    Close => "close", true, Danger;
    CloseStrong => "close-strong", true, Danger;
    Delete => "delete", true, Danger;
    Duplicate => "duplicate", true, Primary;
    Donate => "donate", true, Accent;
    Download => "download", true, Primary;
    Edit => "edit", true, Primary;
    Events => "events", true, Primary;
    File => "file", true, Primary;
    Folder => "folder", false, Warning;
    FolderOpen => "folder-open", false, Warning;
    Globe => "globe", true, Info;
    Help => "help", true, Info;
    Info => "info", true, Info;
    Key => "key", true, Primary;
    Link => "link", true, Info;
    Moon => "moon", false, Primary;
    Package => "package", true, Muted;
    Pause => "pause", true, Primary;
    Play => "play", true, Success;
    Power => "power", true, Accent;
    Proxy => "proxy", true, Primary;
    Radio => "radio", true, Primary;
    Refresh => "refresh", true, Info;
    Repository => "repository", true, Primary;
    Report => "report", true, Info;
    Save => "save", true, Primary;
    Settings => "settings", true, Primary;
    Shield => "shield", true, Info;
    Shortcut => "shortcut", true, Primary;
    Stop => "stop", true, Danger;
    Sun => "sun", false, Warning;
    Update => "update", true, Info;
    Upload => "upload", true, Accent;
    Warning => "warning", true, Warning;
}

impl UiIcon {
    /// Java size of the window and tray icon (`ui/UIConstants.java:24-25`); scales to 24 px.
    pub const WINDOW_ICON_SIZE: u32 = 16;

    /// Java size of the large application picture (`ui/UIConstants.java:26`).
    pub const PICTURE_SIZE: u32 = 256;
}

/// The embedded 512×512 rendered application logo (`app-rendered.png`).
pub const APP_RENDERED_PNG: &[u8] = include_bytes!("../assets/icons/app-rendered.png");

/// Requested sizes above this are used unscaled (`ui/icon/Icons.java:25`).
const SCALE_THRESHOLD: u32 = 64;

/// Default of the icon scale (`ui/icon/Icons.java:117`).
const DEFAULT_SCALE: f64 = 1.5;

/// Upper bound of the icon scale; larger finite values are clamped to it.
const MAX_SCALE: f64 = 8.0;

/// The icon scale factor: env var `A3S_ICON_SCALE` (Java system property `a3s.iconScale`), read
/// once. A missing, unparsable, non-finite or non-positive value gives 1.5; values above 8 are
/// clamped to 8.
pub fn icon_scale() -> f64 {
    static SCALE: OnceLock<f64> = OnceLock::new();
    *SCALE.get_or_init(|| parse_scale(std::env::var("A3S_ICON_SCALE").ok().as_deref()))
}

// Mirrors ui/icon/Icons.java:115-121
fn parse_scale(value: Option<&str>) -> f64 {
    value
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .map_or(DEFAULT_SCALE, |v| v.min(MAX_SCALE))
}

/// Scales a Java icon size: sizes above 64 are kept, others become
/// `max(8, round_half_up(size * scale))`. The result is in egui points.
pub fn scaled_size(size: u32) -> u32 {
    scaled_size_with(size, icon_scale())
}

// Mirrors ui/icon/Icons.java:108-113
pub(crate) fn scaled_size_with(size: u32, scale: f64) -> u32 {
    if size > SCALE_THRESHOLD {
        return size;
    }
    // Java Math.round(double) = floor(x + 0.5).
    let scaled = (f64::from(size) * scale + 0.5).floor();
    (scaled.clamp(0.0, f64::from(u32::MAX)) as u32).max(8)
}

/// The tint applied to `icon` in `state`, or `None` when the original SVG colours are kept.
///
/// Only theme-aware icons and disabled icons are tinted (`ui/icon/Icons.java:62-68`), except
/// [`UiIcon::App`], which is always the untinted `app-rendered.png` (Java `Icons.image(APP)`
/// returns the PNG whatever the state, `ui/icon/Icons.java:44-50`). The tint
/// colour follows `ui/icon/Icons.java:82-98`.
pub fn tint_for(palette: &ThemePalette, icon: UiIcon, state: IconState) -> Option<Color32> {
    if icon == UiIcon::App || (!icon.is_theme_aware() && state != IconState::Disabled) {
        return None;
    }
    Some(match state {
        IconState::Disabled => palette.text_disabled,
        IconState::Selected => palette.state_selection_fg,
        IconState::Normal => match icon.tone() {
            IconTone::Accent => palette.accent_primary,
            IconTone::Danger => palette.status_danger,
            IconTone::Info => palette.status_info,
            IconTone::Muted => palette.text_secondary,
            IconTone::Success => palette.status_success,
            IconTone::Warning => palette.status_warning,
            IconTone::Primary => palette.text_primary,
        },
    })
}

/// A rasterized icon with straight (unmultiplied) RGBA pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StraightImage {
    pub(crate) size: [usize; 2],
    pub(crate) rgba: Vec<u8>,
}

impl StraightImage {
    fn into_color_image(self) -> ColorImage {
        ColorImage::from_rgba_unmultiplied(self.size, &self.rgba)
    }
}

/// Replaces every paint colour with `tint`, keeping the coverage: the egui equivalent of
/// `FlatSVGIcon.ColorFilter(c -> tone)` (`ui/icon/Icons.java:123-125`).
fn recolor(rgba: &mut [u8], tint: Color32) {
    let [tr, tg, tb, ta] = tint.to_srgba_unmultiplied();
    for px in rgba.chunks_exact_mut(4) {
        let alpha = (u16::from(px[3]) * u16::from(ta) / 255) as u8;
        px.copy_from_slice(&[tr, tg, tb, alpha]);
    }
}

/// Rasterizes `icon` to fit a `px`×`px` square, then applies `tint`.
///
/// [`UiIcon::App`] is always drawn from `app-rendered.png`.
pub(crate) fn rasterize(icon: UiIcon, px: u32, tint: Option<Color32>) -> StraightImage {
    let px = px.max(1);
    let mut image = if icon == UiIcon::App {
        app_rgba(px)
    } else {
        let color_image = egui_extras::image::load_svg_bytes_with_size(
            icon.svg_bytes(),
            SizeHint::Size {
                width: px,
                height: px,
                maintain_aspect_ratio: true,
            },
            &resvg::usvg::Options::default(),
        )
        .unwrap_or_else(|err| panic!("embedded asset {}.svg is valid: {err}", icon.file_name()));
        StraightImage {
            size: color_image.size,
            rgba: color_image
                .pixels
                .iter()
                .flat_map(|p| p.to_srgba_unmultiplied())
                .collect(),
        }
    };
    if let Some(tint) = tint {
        recolor(&mut image.rgba, tint);
    }
    image
}

fn app_pixmap() -> &'static Pixmap {
    static PIXMAP: OnceLock<Pixmap> = OnceLock::new();
    PIXMAP.get_or_init(|| {
        Pixmap::decode_png(APP_RENDERED_PNG).expect("embedded asset app-rendered.png is valid")
    })
}

/// Scales `src` to `width`×`height` with bicubic filtering. Large reductions halve the image
/// step by step first so that bicubic sampling does not alias (Java uses `SCALE_SMOOTH`).
fn scale_pixmap(src: &Pixmap, width: u32, height: u32) -> Pixmap {
    let mut current = src.clone();
    while current.width() / 2 >= width && current.height() / 2 >= height {
        current = resample(&current, current.width() / 2, current.height() / 2);
    }
    if current.width() == width && current.height() == height {
        current
    } else {
        resample(&current, width, height)
    }
}

fn resample(src: &Pixmap, width: u32, height: u32) -> Pixmap {
    let mut out = Pixmap::new(width.max(1), height.max(1)).expect("icon size is non-zero");
    let paint = PixmapPaint {
        quality: FilterQuality::Bicubic,
        ..PixmapPaint::default()
    };
    let sx = out.width() as f32 / src.width() as f32;
    let sy = out.height() as f32 / src.height() as f32;
    out.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &paint,
        Transform::from_scale(sx, sy),
        None,
    );
    out
}

fn app_rgba(px: u32) -> StraightImage {
    let pixmap = scale_pixmap(app_pixmap(), px, px);
    StraightImage {
        size: [pixmap.width() as usize, pixmap.height() as usize],
        rgba: pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
    }
}

/// The application icon for windows and the tray, at the scaled Java size: `app_icon_data(16)`
/// is 24×24 (`ui/UIConstants.java:24-26`, `ui/icon/Icons.java:40-56,70-80`).
pub fn app_icon_data(size: u32) -> IconData {
    let px = scaled_size(size);
    let image = app_rgba(px);
    IconData {
        rgba: image.rgba,
        width: px,
        height: px,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    icon: UiIcon,
    size: u32,
    state: IconState,
    dark: bool,
    pixels_per_point_bits: u32,
}

/// Texture cache, keyed like `Icons.CACHE` plus the pixel density
/// (`ui/icon/Icons.java:23,36,127`).
#[derive(Clone, Default)]
struct IconCache(HashMap<CacheKey, TextureHandle>);

fn cache_id() -> Id {
    Id::new("a3s_ui::icon::cache")
}

/// Clears the icon texture cache. Mirrors `ui/icon/Icons.java:58-60`.
///
/// Not needed for theme switches: the cache is keyed by theme.
pub fn invalidate(ctx: &Context) {
    ctx.data_mut(|data| data.remove_temp::<IconCache>(cache_id()));
}

/// The texture of `icon` at Java size `size` (scaled per [`scaled_size`]) in `state`, for the
/// context's current theme and pixel density.
pub fn icon_texture(ctx: &Context, icon: UiIcon, size: u32, state: IconState) -> TextureHandle {
    let scaled = scaled_size(size);
    let pixels_per_point = ctx.pixels_per_point();
    let mode = ThemeMode::from(ctx.theme());
    let key = CacheKey {
        icon,
        size: scaled,
        state,
        dark: mode.is_dark(),
        pixels_per_point_bits: pixels_per_point.to_bits(),
    };
    let cached = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<IconCache>(cache_id())
            .0
            .get(&key)
            .cloned()
    });
    if let Some(handle) = cached {
        return handle;
    }

    let px = (scaled as f32 * pixels_per_point).ceil() as u32;
    let tint = tint_for(&ThemeLibrary::for_mode(mode).palette, icon, state);
    let image = rasterize(icon, px, tint).into_color_image();
    let name = format!(
        "a3s_ui/icon/{}/{scaled}/{state:?}/{}",
        icon.file_name(),
        if key.dark { "dark" } else { "light" }
    );
    let handle = ctx.load_texture(name, image, TextureOptions::LINEAR);
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<IconCache>(cache_id())
            .0
            .insert(key, handle.clone());
    });
    handle
}

/// An image of `icon` at Java size `size` in `state`, sized to `scaled_size(size)` points
/// (aspect-fit for non-square sources such as [`UiIcon::Community`]).
pub fn icon_image(
    ctx: &Context,
    icon: UiIcon,
    size: u32,
    state: IconState,
) -> egui::Image<'static> {
    let handle = icon_texture(ctx, icon, size, state);
    let [w, h] = handle.size();
    let scaled = scaled_size(size) as f32;
    let longest = w.max(h).max(1) as f32;
    let display = vec2(w as f32, h as f32) * (scaled / longest);
    egui::Image::new(SizedTexture::new(handle.id(), display))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_size_injected_scale() {
        assert_eq!(scaled_size_with(12, 1.0), 12);
        assert_eq!(scaled_size_with(4, 1.0), 8);
        assert_eq!(scaled_size_with(12, 1.5), 18);
        assert_eq!(scaled_size_with(65, 3.0), 65);
    }

    #[test]
    fn parse_scale_fallbacks() {
        assert_eq!(parse_scale(None), 1.5);
        assert_eq!(parse_scale(Some("abc")), 1.5);
        assert_eq!(parse_scale(Some("NaN")), 1.5);
        assert_eq!(parse_scale(Some("inf")), 1.5);
        assert_eq!(parse_scale(Some("0")), 1.5);
        assert_eq!(parse_scale(Some("-2")), 1.5);
        assert_eq!(parse_scale(Some("2")), 2.0);
        assert_eq!(parse_scale(Some("100")), 8.0);
    }

    #[test]
    fn every_icon_rasterizes_at_27_px() {
        let palette = &ThemeLibrary::dark().palette;
        for icon in UiIcon::ALL {
            let image = rasterize(icon, 27, None);
            let [w, h] = image.size;
            if icon == UiIcon::Community {
                assert_eq!(w.max(h), 27, "{icon:?} fits the square");
                assert!(w < h, "community crest is portrait");
            } else {
                assert_eq!(image.size, [27, 27], "{icon:?}");
            }
            assert!(
                image.rgba.chunks_exact(4).any(|px| px[3] > 0),
                "{icon:?} has visible pixels"
            );

            if let Some(tint) = tint_for(palette, icon, IconState::Normal) {
                let tinted = rasterize(icon, 27, Some(tint));
                let [r, g, b, _] = tint.to_array();
                for px in tinted.rgba.chunks_exact(4).filter(|px| px[3] > 0) {
                    assert_eq!(&px[..3], &[r, g, b], "{icon:?} is recoloured");
                }
            }
        }
    }

    #[test]
    fn folder_keeps_original_colour() {
        let image = rasterize(UiIcon::Folder, 27, None);
        let opaque: Vec<_> = image
            .rgba
            .chunks_exact(4)
            .filter(|px| px[3] == 255)
            .collect();
        assert!(!opaque.is_empty());
        for px in opaque {
            assert_eq!(&px[..3], &[0xF9, 0xA8, 0x25]);
        }
    }

    #[test]
    fn warning_renders_status_warning_not_svg_fill() {
        let palette = &ThemeLibrary::light().palette;
        let tint = tint_for(palette, UiIcon::Warning, IconState::Normal);
        assert_eq!(tint, Some(palette.status_warning));
        let image = rasterize(UiIcon::Warning, 27, tint);
        let [r, g, b, _] = palette.status_warning.to_array();
        assert!(
            image
                .rgba
                .chunks_exact(4)
                .any(|px| px[3] == 255 && px[..3] == [r, g, b])
        );
    }

    #[test]
    fn app_png_decodes_to_512() {
        let pixmap = app_pixmap();
        assert_eq!((pixmap.width(), pixmap.height()), (512, 512));
    }
}
