//! The macOS progress bar: a thin rounded track with a tone-coloured fill and its label
//! (percent or caller text) beside it.

use egui::{
    Align2, CornerRadius, Rect, Response, Sense, Ui, Widget, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::theme::{self, ProgressTone};

/// Default height of the row the bar sits in.
pub const DEFAULT_HEIGHT: f32 = 24.0;

/// Row height used by progress dialogs.
pub const DIALOG_HEIGHT: f32 = 18.0;

/// Thickness of the track.
const TRACK_HEIGHT: f32 = 6.0;

/// Gap between the track and its label.
const LABEL_GAP: f32 = 8.0;

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
    /// Unknown amount of work: a bouncing box with optional caller text.
    Indeterminate {
        /// Label shown beside the bar, or none.
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

    /// The label shown beside the bar.
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

/// A progress bar: a thin rounded `surfaceMuted` track, centred vertically in its row, with a
/// tone-coloured fill and the label in secondary text at its right.
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

    /// Sets the height of the row in points.
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
        let label = text.filter(|t| !t.is_empty()).map(|t| {
            painter.layout_no_wrap(
                t,
                egui::TextStyle::Small.resolve(ui.style()),
                palette.text_secondary,
            )
        });
        let label_width = label.as_ref().map_or(0.0, |g| g.size().x + LABEL_GAP);
        let track = Rect::from_min_max(
            pos2(rect.left(), rect.center().y - TRACK_HEIGHT / 2.0),
            pos2(
                rect.right() - label_width,
                rect.center().y + TRACK_HEIGHT / 2.0,
            ),
        );
        let radius = CornerRadius::same((TRACK_HEIGHT / 2.0) as u8);
        if track.is_positive() {
            painter.rect_filled(track, radius, palette.surface_muted);
            let fill = if let Some(fraction) = fraction {
                Rect::from_min_size(track.min, vec2(track.width() * fraction, track.height()))
            } else {
                let time = self.animation_time.unwrap_or_else(|| {
                    ui.ctx().request_repaint();
                    ui.input(|i| i.time)
                });
                let box_width = track.width() * INDETERMINATE_FRACTION;
                let x = track.left() + (track.width() - box_width) * bounce(time);
                Rect::from_min_size(pos2(x, track.top()), vec2(box_width, track.height()))
            };
            if fill.width() > 0.0 {
                painter.rect_filled(fill, radius, palette.status_color(self.tone));
            }
        }
        if let Some(galley) = label {
            let pos = Align2::RIGHT_CENTER
                .anchor_size(pos2(rect.right(), rect.center().y), galley.size());
            painter.galley(pos.min, galley, palette.text_secondary);
        }
        response
    }
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
