//! The labelled progress bar of `UiStyle.styleProgressBar` with the halo text of
//! `ContrastProgressBarUI`.

use egui::{Color32, Rect, Response, Sense, Ui, Widget, WidgetInfo, WidgetType, pos2, vec2};

use crate::fonts::{self, FontFace};
use crate::frames;
use crate::theme::{self, ProgressTone};

/// Default bar height (`ui/UiStyle.java:38`).
pub const DEFAULT_HEIGHT: f32 = 24.0;

/// Bar height used by progress dialogs (`ui/AbstractProgressDialog.java:72-73`).
pub const DIALOG_HEIGHT: f32 = 18.0;

/// Seconds for one left → right → left sweep of the indeterminate box.
const INDETERMINATE_PERIOD: f64 = 4.0;

/// The indeterminate box covers this fraction of the inner width.
const INDETERMINATE_FRACTION: f32 = 1.0 / 6.0;

/// The value shown by a [`ProgressBar`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// A known amount of work; the bar shows [`percent_text`].
    Determinate {
        /// Current value, clamped into `[min, max]`.
        value: i64,
        /// Lower bound.
        min: i64,
        /// Upper bound.
        max: i64,
    },
    /// Unknown amount of work: a bouncing box with optional caller text (Q1).
    Indeterminate {
        /// Text painted over the bar, or none.
        text: Option<String>,
    },
}

impl Progress {
    /// A determinate progress from 0 to `max`.
    pub fn of(value: i64, max: i64) -> Self {
        Self::Determinate { value, min: 0, max }
    }

    /// The fill fraction in `[0, 1]`; `None` when indeterminate. Never overflows, even for the
    /// full `i64` range.
    pub fn fraction(&self) -> Option<f32> {
        match *self {
            Self::Determinate { value, min, max } => {
                Some(span(value, min, max).map_or(0.0, |(done, range)| {
                    // f64 keeps 53 bits; the quotient is in [0, 1] by construction.
                    ((done as f64) / (range as f64)).clamp(0.0, 1.0) as f32
                }))
            }
            Self::Indeterminate { .. } => None,
        }
    }

    /// The text painted over the bar.
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Determinate { value, min, max } => Some(percent_text(*value, *min, *max)),
            Self::Indeterminate { text } => text.clone(),
        }
    }
}

/// The percent label of a bar, e.g. `"33%"`.
///
/// `value` is clamped to `[min, max]` like Swing's `DefaultBoundedRangeModel`; a range `<= 0`
/// gives `"0%"`; rounding is Java `Math.round(float)` (half up).
///
/// Mirrors `ui/UiStyle.java:181-188`.
pub fn percent_text(value: i64, min: i64, max: i64) -> String {
    let percent = span(value, min, max).map_or(0, |(done, range)| {
        // Java: Math.round(((value - min) * 100f) / range), all float arithmetic.
        let ratio = (done as f32 * 100.0) / range as f32;
        java_round_float(ratio)
    });
    format!("{percent}%")
}

/// `(value - min, max - min)` after clamping `value` into `[min, max]`, computed in `i128` so the
/// full `i64` range cannot overflow; `None` when the range is empty or negative.
fn span(value: i64, min: i64, max: i64) -> Option<(i128, i128)> {
    let range = i128::from(max) - i128::from(min);
    (range > 0).then(|| {
        let value = value.clamp(min, max);
        (i128::from(value) - i128::from(min), range)
    })
}

/// Java `Math.round(float)` (JDK 7+): the exact `floor(x + 1/2)` of the float value, without
/// the extra float rounding of `x + 0.5f` (`Math.round(0.49999997f) == 0`).
fn java_round_float(x: f32) -> i64 {
    // Every f32 is exact in f64, and x + 0.5 is exact in f64 for |x| < 2^52.
    (f64::from(x) + 0.5).floor() as i64
}

/// A progress bar styled like `UiStyle.styleProgressBar` (`ui/UiStyle.java:72-89`).
///
/// 1 px `borderMuted` outline, padding 4 / 8 / 4 / 8, `surfaceMuted` track (Q1), tone-coloured
/// fill and white text with a dark halo (`ui/components/ContrastProgressBarUI.java`).
#[derive(Clone, Debug)]
#[must_use = "add it to a Ui with `ui.add(bar)`"]
pub struct ProgressBar {
    progress: Progress,
    tone: ProgressTone,
    height: f32,
    desired_width: Option<f32>,
    animation_time: Option<f64>,
}

impl ProgressBar {
    /// A bar showing `progress`, tone [`ProgressTone::Info`], height [`DEFAULT_HEIGHT`].
    pub fn new(progress: Progress) -> Self {
        Self {
            progress,
            tone: ProgressTone::default(),
            height: DEFAULT_HEIGHT,
            desired_width: None,
            animation_time: None,
        }
    }

    /// Sets the fill tone.
    pub fn tone(mut self, tone: ProgressTone) -> Self {
        self.tone = tone;
        self
    }

    /// Sets the outer height in points.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Sets the outer width; the default is the available width.
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Freezes the indeterminate animation at `seconds` instead of the input clock and stops
    /// requesting repaints. Used for deterministic snapshots.
    pub fn animation_time(mut self, seconds: f64) -> Self {
        self.animation_time = Some(seconds);
        self
    }
}

/// Position in `[0, 1]` of the indeterminate box at `time`: a triangle wave.
fn bounce(time: f64) -> f32 {
    let phase = (time / INDETERMINATE_PERIOD).rem_euclid(1.0) as f32;
    if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    }
}

impl Widget for ProgressBar {
    fn ui(self, ui: &mut Ui) -> Response {
        let def = theme::current(ui.ctx());
        let palette = &def.palette;
        let m = &def.metrics;
        let width = self
            .desired_width
            .unwrap_or_else(|| ui.available_size_before_wrap().x);
        let (rect, response) = ui.allocate_exact_size(vec2(width, self.height), Sense::hover());

        let text = self.progress.text();
        let fraction = self.progress.fraction();
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(
                WidgetType::ProgressIndicator,
                ui.is_enabled(),
                text.clone().unwrap_or_default(),
            );
            info.value = fraction.map(f64::from);
            info
        });

        if !ui.is_rect_visible(rect) {
            return response;
        }

        let painter = ui.painter();
        let tone = palette.status_color(self.tone);
        frames::paint_line_border(painter, rect, palette.border_muted);
        // LineBorder(1) + EmptyBorder(xs, sm, xs, sm).
        let inner = Rect::from_min_max(
            pos2(
                rect.left() + 1.0 + m.spacing_sm as f32,
                rect.top() + 1.0 + m.spacing_xs as f32,
            ),
            pos2(
                rect.right() - 1.0 - m.spacing_sm as f32,
                rect.bottom() - 1.0 - m.spacing_xs as f32,
            ),
        );
        if inner.is_positive() {
            painter.rect_filled(inner, 0.0, palette.surface_muted);
            let fill = if let Some(fraction) = fraction {
                let fill_width = (inner.width() * fraction).round();
                Rect::from_min_size(inner.min, vec2(fill_width, inner.height()))
            } else {
                let time = self.animation_time.unwrap_or_else(|| {
                    ui.ctx().request_repaint();
                    ui.input(|i| i.time)
                });
                let box_width = (inner.width() * INDETERMINATE_FRACTION).round();
                let x = inner.left() + (inner.width() - box_width) * bounce(time);
                Rect::from_min_size(pos2(x, inner.top()), vec2(box_width, inner.height()))
            };
            painter.rect_filled(fill, 0.0, tone);
        }

        if let Some(text) = text.filter(|t| !t.is_empty()) {
            paint_halo_text(ui, rect, inner, &text);
        }
        response
    }
}

/// Paints `text` centred on `inner` with an 8-fold dark halo, clipped to `clip`.
///
/// Mirrors `ui/components/ContrastProgressBarUI.java:37-45,54-71`.
fn paint_halo_text(ui: &Ui, clip: Rect, inner: Rect, text: &str) {
    const HALO: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 220);
    let painter = ui.painter_at(clip);
    // Swing deriveFont(BOLD) → Geist SemiBold (ui/UiStyle.java:76-78).
    let font = fonts::font_id_in(ui.ctx(), FontFace::SemiBold, fonts::BODY_SIZE);
    let galley = painter.layout_no_wrap(text.to_owned(), font, Color32::WHITE);
    let size = galley.size();
    let ascent = galley
        .rows
        .first()
        .and_then(|row| row.row.glyphs.first())
        .map_or(size.y * 0.75, |glyph| glyph.font_ascent);

    let x = inner.left() + ((inner.width() - size.x) / 2.0).round();
    let baseline = inner.top() + ((inner.height() + ascent) / 2.0).floor() - 2.0;
    let top = baseline - ascent;

    for dx in -1..=1 {
        for dy in -1..=1 {
            if dx != 0 || dy != 0 {
                painter.galley_with_override_text_color(
                    pos2(x + dx as f32, top + dy as f32),
                    galley.clone(),
                    HALO,
                );
            }
        }
    }
    painter.galley(pos2(x, top), galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_round_float_is_exact() {
        assert_eq!(java_round_float(0.499_999_97), 0);
        assert_eq!(java_round_float(0.5), 1);
        assert_eq!(java_round_float(99.5), 100);
        assert_eq!(java_round_float(33.333_332), 33);
        assert_eq!(
            span(0, i64::MIN, i64::MAX),
            Some((i128::from(i64::MAX) + 1, u64::MAX.into()))
        );
        assert_eq!(span(3, 5, 5), None);
    }

    #[test]
    fn bounce_is_a_triangle_wave() {
        assert_eq!(bounce(0.0), 0.0);
        assert_eq!(bounce(2.0), 1.0);
        assert_eq!(bounce(1.0), 0.5);
        assert_eq!(bounce(3.0), 0.5);
        assert_eq!(bounce(4.0), 0.0);
    }

    #[test]
    fn fraction_clamps() {
        assert_eq!(Progress::of(150, 100).fraction(), Some(1.0));
        assert_eq!(Progress::of(-3, 100).fraction(), Some(0.0));
        assert_eq!(
            Progress::Determinate {
                value: 5,
                min: 10,
                max: 10
            }
            .fraction(),
            Some(0.0)
        );
        assert_eq!(Progress::Indeterminate { text: None }.fraction(), None);
    }
}
