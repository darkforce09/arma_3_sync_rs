//! Dialog bases: an OK/Cancel dialog (`ui/AbstractDialog.java`) and a progress dialog
//! (`ui/AbstractProgressDialog.java`).
//!
//! Dialogs are in-app egui windows, not OS viewports (Q9). The caller shows a dialog every frame
//! while it is open and closes it when an action other than `None` comes back.
//!
//! Keyboard routing follows Swing's focused window: Enter (default button) and Escape go to the
//! topmost modal dialog if one is open, otherwise to the non-modal dialog that has keyboard
//! focus. A dialog gets focus when it is first shown and when a pointer press lands on it, and
//! loses it when a press lands elsewhere (e.g. in a text field of the main window).

use std::borrow::Cow;

use egui::{
    Align2, AsId, Context, Frame, Id, Key, LayerId, Modal, Modifiers, Order, Response, RichText,
    Sense, Stroke, TextStyle, Ui, Vec2, WidgetInfo, WidgetType, Window,
};

use crate::consts;
use crate::theme;
use crate::widgets::progress::{self, Progress, ProgressBar};
use crate::widgets::{button, primary_button};

/// Gap of the empty Swing side panels and of the vertical strut above the buttons.
const DIALOG_GAP: i8 = 10;

/// Fixed width of a progress dialog (`ui/AbstractProgressDialog.java:75-76`).
pub const PROGRESS_DIALOG_WIDTH: f32 = 250.0;

/// What the user did with a [`Dialog`] this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DialogAction {
    /// Nothing happened.
    #[default]
    None,
    /// The OK button (`buttonOKPerformed`, `ui/AbstractDialog.java:54-58`).
    Ok,
    /// The Cancel button (`buttonCancelPerformed`, `ui/AbstractDialog.java:60-65`).
    Cancel,
    /// The title-bar close button or Escape (`menuExitPerformed`, `ui/AbstractDialog.java:67-72`).
    Close,
}

/// A dialog button, used to pick the default (Enter) button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogButton {
    /// The OK button.
    Ok,
    /// The Cancel button.
    Cancel,
}

/// One button of a dialog's button row.
struct RowButton<'b> {
    label: &'b str,
    which: DialogButton,
    enabled: bool,
}

/// A modal or non-modal dialog with an OK/Cancel button row. Mirrors `ui/AbstractDialog.java`.
#[derive(Clone, Debug)]
#[must_use = "call `show` every frame while the dialog is open"]
pub struct Dialog<'a> {
    id: Id,
    title: Cow<'a, str>,
    modal: bool,
    resizable: bool,
    default_size: Option<Vec2>,
    min_size: Option<Vec2>,
    ok_label: Cow<'a, str>,
    cancel_label: Cow<'a, str>,
    show_ok: bool,
    ok_enabled: bool,
    cancel_enabled: bool,
    default_button: Option<DialogButton>,
}

impl<'a> Dialog<'a> {
    /// A modal, resizable dialog with "OK" and "Cancel" and no default button.
    pub fn new(id_salt: impl AsId, title: impl Into<Cow<'a, str>>) -> Self {
        Self {
            id: Id::new(id_salt),
            title: title.into(),
            modal: true,
            resizable: true,
            default_size: None,
            min_size: None,
            ok_label: Cow::Borrowed(consts::OK),
            cancel_label: Cow::Borrowed(consts::CANCEL),
            show_ok: true,
            ok_enabled: true,
            cancel_enabled: true,
            default_button: None,
        }
    }

    /// Modal (`egui::Modal`, blocks the rest of the UI) or a movable `egui::Window`.
    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }

    /// Whether a non-modal dialog can be resized (`JDialog` default `true`).
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Initial content size.
    pub fn default_size(mut self, size: impl Into<Vec2>) -> Self {
        self.default_size = Some(size.into());
        self
    }

    /// Minimum content size.
    pub fn min_size(mut self, size: impl Into<Vec2>) -> Self {
        self.min_size = Some(size.into());
        self
    }

    /// Text of the OK button (default `"OK"`).
    pub fn ok_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.ok_label = label.into();
        self
    }

    /// Text of the Cancel button (default `"Cancel"`).
    pub fn cancel_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Whether the OK button is shown (default `true`).
    pub fn show_ok(mut self, show: bool) -> Self {
        self.show_ok = show;
        self
    }

    /// Whether the OK button is enabled (default `true`, `buttonOK.setEnabled(..)`). A disabled
    /// default button also ignores Enter.
    pub fn ok_enabled(mut self, enabled: bool) -> Self {
        self.ok_enabled = enabled;
        self
    }

    /// Whether the Cancel button is enabled (default `true`). A disabled default button also
    /// ignores Enter. The close button and Escape stay live.
    pub fn cancel_enabled(mut self, enabled: bool) -> Self {
        self.cancel_enabled = enabled;
        self
    }

    /// The button triggered by Enter (`getRootPane().setDefaultButton(..)`); it is drawn as the
    /// primary button. Default: none.
    pub fn default_button(mut self, button: Option<DialogButton>) -> Self {
        self.default_button = button;
        self
    }

    /// Shows the dialog centred on the screen and returns the user's action.
    pub fn show(self, ctx: &Context, add_contents: impl FnOnce(&mut Ui)) -> DialogAction {
        if self.modal {
            self.show_modal(ctx, add_contents)
        } else {
            self.show_window(ctx, add_contents)
        }
    }

    fn frame(ctx: &Context) -> Frame {
        Frame::window(&ctx.global_style()).inner_margin(DIALOG_GAP)
    }

    fn show_modal(self, ctx: &Context, add_contents: impl FnOnce(&mut Ui)) -> DialogAction {
        // Keys go to the topmost modal; `is_top_modal` is only known once the modal is shown,
        // so the previous frame's top modal layer decides.
        let layer = LayerId::new(Order::Foreground, self.id);
        let has_keys = ctx.memory(|mem| mem.top_modal_layer()) == Some(layer);
        let mut action = DialogAction::None;
        Modal::new(self.id).frame(Self::frame(ctx)).show(ctx, |ui| {
            self.apply_sizes(ui);
            action = with_title_bar(ui, self.id, &self.title, false, |ui| {
                self.body(ui, add_contents, has_keys)
            });
        });
        // Q8: Escape closes; clicking the backdrop does nothing.
        if action == DialogAction::None && has_keys && consume(ctx, Key::Escape) {
            action = DialogAction::Close;
        }
        action
    }

    fn show_window(self, ctx: &Context, add_contents: impl FnOnce(&mut Ui)) -> DialogAction {
        let has_keys = window_has_keys(ctx, self.id);
        let mut action = DialogAction::None;
        let mut window = Window::new(self.title.as_ref())
            .id(self.id)
            .title_bar(false)
            .collapsible(false)
            .resizable(self.resizable)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .frame(Self::frame(ctx));
        if let Some(size) = self.default_size {
            window = window.default_size(size);
        }
        if let Some(size) = self.min_size {
            window = window.min_size(size);
        }
        window.show(ctx, |ui| {
            if self.resizable {
                // Fill the user-sized window so the button row sits on its right edge.
                ui.set_min_width(ui.available_width());
            }
            action = with_title_bar(ui, self.id, &self.title, self.resizable, |ui| {
                self.body(ui, add_contents, has_keys)
            });
        });
        if action == DialogAction::None && has_keys && consume(ctx, Key::Escape) {
            action = DialogAction::Close;
        }
        action
    }

    fn apply_sizes(&self, ui: &mut Ui) {
        if let Some(size) = self.default_size {
            ui.set_min_size(size);
        }
        if let Some(size) = self.min_size {
            ui.set_min_size(size);
        }
    }

    /// Content, 10 px strut, right-aligned OK / Cancel row (`ui/AbstractDialog.java:27-50`).
    fn body(
        &self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui),
        has_keys: bool,
    ) -> DialogAction {
        add_contents(ui);
        ui.add_space(f32::from(DIALOG_GAP));

        let mut buttons = Vec::with_capacity(2);
        if self.show_ok {
            buttons.push(RowButton {
                label: &self.ok_label,
                which: DialogButton::Ok,
                enabled: self.ok_enabled,
            });
        }
        buttons.push(RowButton {
            label: &self.cancel_label,
            which: DialogButton::Cancel,
            enabled: self.cancel_enabled,
        });
        let mut action = DialogAction::None;
        let responses = right_aligned_buttons(ui, &buttons, self.default_button);
        for (response, b) in responses.iter().zip(&buttons) {
            if response.clicked() {
                action = button_action(b.which);
            }
        }

        let default_enabled =
            |default: DialogButton| buttons.iter().any(|b| b.which == default && b.enabled);
        if action == DialogAction::None
            && has_keys
            && let Some(default) = self.default_button
            && default_enabled(default)
            && enter_for_default_button(ui.ctx())
        {
            action = button_action(default);
        }
        action
    }
}

fn button_action(button: DialogButton) -> DialogAction {
    match button {
        DialogButton::Ok => DialogAction::Ok,
        DialogButton::Cancel => DialogAction::Cancel,
    }
}

fn consume(ctx: &Context, key: Key) -> bool {
    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key))
}

/// Enter fires the default button when no widget keeps the keyboard (a focused single-line
/// `TextEdit` gives up focus on Enter, a multiline one keeps it).
fn enter_for_default_button(ctx: &Context) -> bool {
    !ctx.egui_wants_keyboard_input() && consume(ctx, Key::Enter)
}

fn focused_dialog_id() -> Id {
    Id::new("a3s_ui::dialog::keyboard_focus")
}

/// Updates which non-modal dialog has keyboard focus and returns whether the dialog `id` gets
/// Enter / Escape this frame: it must have focus and no modal may be open.
///
/// A dialog takes focus when it is (re)shown after not being shown in the previous pass, and
/// when a pointer press lands on its window; a press anywhere else takes the focus away.
fn window_has_keys(ctx: &Context, id: Id) -> bool {
    let pass = ctx.cumulative_pass_nr();
    let shown_pass_id = id.with("a3s_ui::dialog::shown_pass");
    let previous = ctx.data_mut(|d| {
        let previous = d.get_temp::<u64>(shown_pass_id);
        d.insert_temp(shown_pass_id, pass);
        previous
    });
    let fresh = previous.is_none_or(|p| p.saturating_add(1) < pass);

    let layer = LayerId::new(Order::Middle, id);
    let press_pos = ctx.input(|i| {
        i.pointer
            .any_pressed()
            .then(|| i.pointer.interact_pos())
            .flatten()
    });
    let press_on_me = press_pos.map(|pos| ctx.layer_id_at(pos) == Some(layer));

    let focused = ctx.data_mut(|d| {
        let focused = d.get_temp_mut_or_default::<Option<Id>>(focused_dialog_id());
        if fresh || press_on_me == Some(true) {
            *focused = Some(id);
        } else if press_on_me == Some(false) && *focused == Some(id) {
            *focused = None;
        }
        *focused == Some(id)
    });
    let modal_open = ctx.memory(|mem| mem.top_modal_layer()).is_some();
    focused && !modal_open
}

/// Lays out `buttons` left to right (keeping Java's tab order OK, Cancel) flush with the right
/// edge of the content above (Swing `FlowLayout(RIGHT)`).
fn right_aligned_buttons(
    ui: &mut Ui,
    buttons: &[RowButton<'_>],
    default: Option<DialogButton>,
) -> Vec<Response> {
    let min_width = (theme::metrics_of(ui.ctx()).spacing_xl * 2) as f32;
    let padding = ui.spacing().button_padding.x;
    let gap = ui.spacing().item_spacing.x;
    let font = TextStyle::Button.resolve(ui.style());
    let widths: f32 = buttons
        .iter()
        .map(|b| {
            let galley =
                ui.painter()
                    .layout_no_wrap(b.label.to_owned(), font.clone(), egui::Color32::WHITE);
            (galley.size().x + 2.0 * padding).max(min_width)
        })
        .sum();
    let total = widths + gap * buttons.len().saturating_sub(1) as f32;
    let row_width = ui.min_rect().width().max(total);

    ui.horizontal(|ui| {
        ui.add_space((row_width - total).max(0.0));
        buttons
            .iter()
            .map(|b| {
                ui.add_enabled_ui(b.enabled, |ui| {
                    if default == Some(b.which) {
                        primary_button(ui, b.label)
                    } else {
                        button(ui, b.label)
                    }
                })
                .inner
            })
            .collect()
    })
    .inner
}

/// Draws the dialog title row (title on the left like an OS title bar, "Close window" ✕ on
/// the right), a separator and `body`. A click on ✕ yields [`DialogAction::Close`].
///
/// Modal and non-modal dialogs share this look. `fills_width` is true when the content already
/// spans the window (resizable windows); otherwise the title row uses the content width of the
/// previous frame and asks for one more frame when it changed.
fn with_title_bar(
    ui: &mut Ui,
    id: Id,
    title: &str,
    fills_width: bool,
    body: impl FnOnce(&mut Ui) -> DialogAction,
) -> DialogAction {
    let width_id = id.with("a3s_ui::dialog::content_width");
    let stored = ui.data(|d| d.get_temp::<f32>(width_id)).unwrap_or(0.0);
    let content_width = if fills_width {
        ui.min_rect().width()
    } else {
        stored
    };
    let close = title_bar(ui, title, content_width).clicked();
    ui.separator();
    let action = body(ui);

    let width = ui.min_rect().width();
    if !fills_width && (width - stored).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(width_id, width));
        ui.ctx().request_repaint();
    }
    if close { DialogAction::Close } else { action }
}

fn title_bar(ui: &mut Ui, title: &str, content_width: f32) -> Response {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).text_style(TextStyle::Heading));
        let size = ui.spacing().icon_width;
        let used = ui.cursor().left() - ui.min_rect().left();
        ui.add_space((content_width - used - size).max(0.0));
        close_button(ui, size)
    })
    .inner
}

fn close_button(ui: &mut Ui, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    response
        .widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Close window"));
    let visuals = ui.style().interact(&response);
    let r = rect.shrink(2.0);
    let stroke: Stroke = visuals.fg_stroke;
    ui.painter()
        .line_segment([r.left_top(), r.right_bottom()], stroke);
    ui.painter()
        .line_segment([r.right_top(), r.left_bottom()], stroke);
    response
}

/// What the user did with a [`ProgressDialog`] this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProgressDialogAction {
    /// Nothing happened.
    #[default]
    None,
    /// Cancel, Enter, Escape or the window's close button (all call `menuExitPerformed`,
    /// `ui/AbstractProgressDialog.java:81-93`).
    Cancel,
}

/// A non-modal, non-resizable "Progress" dialog: text, an 18 px INFO progress bar and a default
/// Cancel button. Mirrors `ui/AbstractProgressDialog.java`.
#[derive(Clone, Debug)]
pub struct ProgressDialog {
    id: Id,
    /// Text above the bar.
    pub text: String,
    /// Bar value.
    pub progress: Progress,
    /// Set by the caller once it has acted on a cancel (`ui/AbstractProgressDialog.java:25`).
    pub canceled: bool,
    /// Whether the Cancel button (and its Enter / Escape shortcuts) is enabled; default `true`.
    /// The window's close button always cancels.
    pub cancel_enabled: bool,
    animation_time: Option<f64>,
}

impl ProgressDialog {
    /// Title of every progress dialog (`ui/AbstractProgressDialog.java:28`).
    pub const TITLE: &'static str = "Progress";

    /// A dialog showing `text` and an indeterminate bar.
    pub fn new(id_salt: impl AsId, text: impl Into<String>) -> Self {
        Self {
            id: Id::new(id_salt),
            text: text.into(),
            progress: Progress::Indeterminate { text: None },
            canceled: false,
            cancel_enabled: true,
            animation_time: None,
        }
    }

    /// Sets [`Self::cancel_enabled`].
    pub fn cancel_enabled(mut self, enabled: bool) -> Self {
        self.cancel_enabled = enabled;
        self
    }

    /// Freezes an indeterminate bar's animation at `seconds` (for snapshots).
    pub fn animation_time(mut self, seconds: f64) -> Self {
        self.animation_time = Some(seconds);
        self
    }

    /// Shows the dialog centred on the screen and returns the user's action.
    pub fn show(&mut self, ctx: &Context) -> ProgressDialogAction {
        let has_keys = window_has_keys(ctx, self.id) && self.cancel_enabled;
        let mut cancel = false;
        let inner_width = PROGRESS_DIALOG_WIDTH - 2.0 * f32::from(DIALOG_GAP) - 2.0;
        Window::new(Self::TITLE)
            .id(self.id)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .default_width(inner_width)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .frame(Dialog::frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(inner_width);
                let action = with_title_bar(ui, self.id, Self::TITLE, true, |ui| {
                    ui.label(&self.text);
                    let mut bar = ProgressBar::new(self.progress.clone())
                        .tone(theme::ProgressTone::Info)
                        .height(progress::DIALOG_HEIGHT);
                    if let Some(time) = self.animation_time {
                        bar = bar.animation_time(time);
                    }
                    ui.add(bar);
                    ui.add_space(f32::from(DIALOG_GAP));
                    let responses = right_aligned_buttons(
                        ui,
                        &[RowButton {
                            label: consts::CANCEL,
                            which: DialogButton::Cancel,
                            enabled: self.cancel_enabled,
                        }],
                        Some(DialogButton::Cancel),
                    );
                    // Cancel is the default button (ui/AbstractProgressDialog.java:79).
                    if responses.iter().any(Response::clicked)
                        || (has_keys && enter_for_default_button(ui.ctx()))
                    {
                        DialogAction::Cancel
                    } else {
                        DialogAction::None
                    }
                });
                cancel = action != DialogAction::None;
            });
        if !cancel && has_keys && consume(ctx, Key::Escape) {
            cancel = true;
        }
        if cancel {
            ProgressDialogAction::Cancel
        } else {
            ProgressDialogAction::None
        }
    }
}
