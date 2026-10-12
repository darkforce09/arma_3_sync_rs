//! Dialog keyboard routing, button enable flags and check-tree keyboard focus.

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton, ProgressDialog, ProgressDialogAction};
use a3s_ui::gallery::SampleTree;
use a3s_ui::widgets::{CheckTree, CheckTreeEvent, Progress};
use egui::Key;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

// ---------------------------------------------------------------------------------------------
// Progress dialog over a main window with a text field

struct BackgroundState {
    text: String,
    dialog: ProgressDialog,
    actions: Vec<ProgressDialogAction>,
}

fn background_harness(cancel_enabled: bool) -> Harness<'static, BackgroundState> {
    let mut dialog = ProgressDialog::new("bg_progress", "Downloading files...");
    dialog.progress = Progress::of(10, 100);
    dialog.cancel_enabled = cancel_enabled;
    let state = BackgroundState {
        text: String::new(),
        dialog,
        actions: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size((800.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .build_ui_state(
            |ui, state: &mut BackgroundState| {
                ui.add(egui::TextEdit::singleline(&mut state.text).id_salt("search"));
                let action = state.dialog.show(ui.ctx());
                if action != ProgressDialogAction::None {
                    state.actions.push(action);
                }
            },
            state,
        );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

#[test]
fn enter_in_background_text_edit_does_not_cancel_progress_dialog() {
    let mut harness = background_harness(true);
    harness.get_by_role(Role::TextInput).click();
    harness.run();
    harness.get_by_role(Role::TextInput).type_text("abc");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().text, "abc");
    assert!(
        harness.state().actions.is_empty(),
        "Enter cancelled the dialog"
    );
    harness.key_press(Key::Escape);
    harness.run();
    assert!(
        harness.state().actions.is_empty(),
        "Escape cancelled the dialog"
    );

    // A press on the dialog gives it the keyboard back.
    harness.get_by_label("Downloading files...").click();
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().actions, [ProgressDialogAction::Cancel]);
}

#[test]
fn disabled_progress_cancel_ignores_keys_but_close_still_cancels() {
    let mut harness = background_harness(false);
    harness.key_press(Key::Enter);
    harness.run();
    harness.key_press(Key::Escape);
    harness.run();
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(harness.state().actions.is_empty());

    harness.get_by_label("Close window").click();
    harness.run();
    assert_eq!(harness.state().actions, [ProgressDialogAction::Cancel]);
}

// ---------------------------------------------------------------------------------------------
// Two non-modal dialogs

struct TwoState {
    a: Vec<DialogAction>,
    b: Vec<DialogAction>,
}

#[test]
fn escape_goes_to_the_focused_non_modal_dialog() {
    let state = TwoState {
        a: Vec::new(),
        b: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size((800.0, 600.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .build_ui_state(
            |ui, state: &mut TwoState| {
                let a = Dialog::new("dialog_a", "First")
                    .modal(false)
                    .resizable(false)
                    .show(ui.ctx(), |ui| {
                        ui.label("Body A");
                    });
                let b = Dialog::new("dialog_b", "Second")
                    .modal(false)
                    .resizable(false)
                    .show(ui.ctx(), |ui| {
                        ui.label("Body B");
                    });
                if a != DialogAction::None {
                    state.a.push(a);
                }
                if b != DialogAction::None {
                    state.b.push(b);
                }
            },
            state,
        );
    a3s_ui::install(&harness.ctx);
    harness.run();

    // The most recently opened dialog has focus.
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.state().a.is_empty());
    assert_eq!(harness.state().b, [DialogAction::Close]);
}

// ---------------------------------------------------------------------------------------------
// Dialog enable flags

struct FlagState {
    actions: Vec<DialogAction>,
}

fn flag_harness(build: fn(Dialog<'static>) -> Dialog<'static>) -> Harness<'static, FlagState> {
    let mut harness = Harness::builder().with_size((800.0, 600.0)).build_ui_state(
        move |ui, state: &mut FlagState| {
            let action = build(Dialog::new("flags", "Flags")).show(ui.ctx(), |ui| {
                ui.label("Body");
            });
            if action != DialogAction::None {
                state.actions.push(action);
            }
        },
        FlagState {
            actions: Vec::new(),
        },
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

#[test]
fn disabled_ok_ignores_click_and_enter_but_escape_closes() {
    let mut harness = flag_harness(|d| d.ok_enabled(false).default_button(Some(DialogButton::Ok)));
    harness.get_by_label("OK").click();
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().actions.is_empty());
    harness.key_press(Key::Escape);
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Close]);
}

#[test]
fn disabled_cancel_ignores_click_and_enter() {
    let mut harness = flag_harness(|d| {
        d.cancel_enabled(false)
            .default_button(Some(DialogButton::Cancel))
    });
    harness.get_by_label("Cancel").click();
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().actions.is_empty());
    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Ok]);
}

#[test]
fn non_modal_disabled_ok_and_close_button() {
    let mut harness = flag_harness(|d| d.modal(false).ok_enabled(false));
    harness.get_by_label("OK").click();
    harness.run();
    assert!(harness.state().actions.is_empty());
    harness.get_by_label("Close window").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Close]);
}

// ---------------------------------------------------------------------------------------------
// Modal on top of a progress dialog

struct StackState {
    progress: ProgressDialog,
    progress_actions: Vec<ProgressDialogAction>,
    modal_actions: Vec<DialogAction>,
}

#[test]
fn enter_goes_to_the_modal_default_button() {
    let mut progress = ProgressDialog::new("stack_progress", "Downloading...");
    progress.progress = Progress::of(10, 100);
    let mut harness = Harness::builder().with_size((800.0, 600.0)).build_ui_state(
        |ui, state: &mut StackState| {
            let p = state.progress.show(ui.ctx());
            if p != ProgressDialogAction::None {
                state.progress_actions.push(p);
            }
            let m = Dialog::new("stack_modal", "Confirm")
                .default_button(Some(DialogButton::Ok))
                .show(ui.ctx(), |ui| {
                    ui.label("Sure?");
                });
            if m != DialogAction::None {
                state.modal_actions.push(m);
            }
        },
        StackState {
            progress,
            progress_actions: Vec::new(),
            modal_actions: Vec::new(),
        },
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().progress_actions.is_empty());
    assert_eq!(harness.state().modal_actions, [DialogAction::Ok]);
}

// ---------------------------------------------------------------------------------------------
// Check tree keyboard focus

struct TreeState {
    tree: CheckTree,
    model: SampleTree,
    events: Vec<CheckTreeEvent<String>>,
    focused: bool,
}

fn tree_harness() -> Harness<'static, TreeState> {
    let mut harness = Harness::builder()
        .with_size((400.0, 500.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .build_ui_state(
            |ui, state: &mut TreeState| {
                let events = state.tree.show(ui, &state.model);
                state.events.extend(events);
                state.focused = state.tree.has_focus(ui.ctx());
            },
            TreeState {
                tree: CheckTree::new("keys_tree").default_expanded(true),
                model: SampleTree::default(),
                events: Vec::new(),
                focused: false,
            },
        );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

fn take(harness: &mut Harness<'_, TreeState>) -> Vec<CheckTreeEvent<String>> {
    std::mem::take(&mut harness.state_mut().events)
}

fn s(text: &str) -> String {
    text.to_owned()
}

#[test]
fn tree_focus_delete_and_arrows() {
    let mut harness = tree_harness();
    assert!(!harness.state().focused);
    harness.key_press(Key::Delete);
    harness.key_press(Key::ArrowDown);
    harness.run();
    assert!(take(&mut harness).is_empty(), "keys need focus");

    harness.get_by_role_and_label(Role::Button, "@ace").click();
    harness.run();
    assert!(harness.state().focused);
    assert_eq!(take(&mut harness), [CheckTreeEvent::RowClicked(s("@ace"))]);

    harness.key_press(Key::Delete);
    harness.run();
    assert_eq!(
        take(&mut harness),
        [CheckTreeEvent::Key(s("@ace"), Key::Delete)]
    );

    harness.key_press(Key::ArrowDown);
    harness.run();
    assert!(take(&mut harness).is_empty());
    assert!(harness.state().tree.is_selected(&s("ace_common.pbo")));
    assert!(harness.state().focused, "arrows keep the focus in the tree");

    harness.key_press(Key::ArrowUp);
    harness.run();
    harness.key_press(Key::ArrowUp);
    harness.run();
    assert!(harness.state().tree.is_selected(&s("@mods")));
    // Already on the first row.
    harness.key_press(Key::ArrowUp);
    harness.run();
    assert!(harness.state().tree.is_selected(&s("@mods")));
    harness.key_press(Key::Space);
    harness.run();
    assert_eq!(take(&mut harness), [CheckTreeEvent::Toggled(s("@mods"))]);
}
