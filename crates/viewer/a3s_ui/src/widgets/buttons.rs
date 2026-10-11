//! Button helpers: plain, primary ("default" button) and icon + text.
//!
//! Every helper enforces `FlatLaf`'s `Button.minimumWidth` = spacingXl × 2 = 48
//! (`ui/theme/ThemeTokens.java:68`).

use egui::{Button, Response, RichText, Stroke, Ui, Vec2, vec2};

use crate::icon::{self, IconState, UiIcon};
use crate::theme;

fn min_size(ui: &Ui) -> Vec2 {
    let m = theme::metrics_of(ui.ctx());
    vec2((m.spacing_xl * 2) as f32, ui.spacing().interact_size.y)
}

/// A normal button with the minimum width.
pub fn button(ui: &mut Ui, text: &str) -> Response {
    let size = min_size(ui);
    ui.add(Button::new(text).min_size(size))
}

/// The primary / default button: fill `accentPrimary`, text `surface`, hover
/// `accentSecondary`, focus ring `stateFocus` (`ui/theme/ThemeTokens.java:63-68`).
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let p = &theme::current(ui.ctx()).palette;
    let size = min_size(ui);
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for (visuals, fill) in [
            (&mut widgets.inactive, p.accent_primary),
            (&mut widgets.hovered, p.accent_secondary),
            (&mut widgets.active, p.accent_secondary),
        ] {
            visuals.bg_fill = fill;
            visuals.weak_bg_fill = fill;
            visuals.fg_stroke.color = p.surface;
            visuals.bg_stroke = Stroke::new(1.0, fill);
        }
        widgets.active.bg_stroke = Stroke::new(1.0, p.state_focus);
        ui.add(Button::new(RichText::new(text).color(p.surface)).min_size(size))
    })
    .inner
}

/// A button with a leading icon (Java size `icon_size`, scaled) and text. The icon is drawn
/// in its disabled state when the `Ui` is disabled.
pub fn icon_text_button(ui: &mut Ui, icon: UiIcon, icon_size: u32, text: &str) -> Response {
    let state = if ui.is_enabled() {
        IconState::Normal
    } else {
        IconState::Disabled
    };
    let image = icon::icon_image(ui.ctx(), icon, icon_size, state);
    let size = min_size(ui);
    ui.add(Button::image_and_text(image, text).min_size(size))
}
