//! kittest interaction tests for the check tree, check list and dialogs.

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton, ProgressDialog, ProgressDialogAction};
use a3s_ui::gallery::SampleTree;
use a3s_ui::widgets::{
    CheckItem, CheckList, CheckTree, CheckTreeEvent, Progress, checked_indexes, checked_labels,
};
use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

struct TreeState {
    tree: CheckTree,
    model: SampleTree,
    events: Vec<CheckTreeEvent<String>>,
}

fn tree_harness() -> Harness<'static, TreeState> {
    let state = TreeState {
        tree: CheckTree::new("test_tree").default_expanded(true),
        model: SampleTree::default(),
        events: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size((400.0, 500.0))
        // Short steps keep the two clicks of a double-click within egui's 0.3 s window.
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
        |ui, state: &mut TreeState| {
            let events = state.tree.show(ui, &state.model);
            state.events.extend(events);
        },
        state,
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

fn take_events(harness: &mut Harness<'_, TreeState>) -> Vec<CheckTreeEvent<String>> {
    std::mem::take(&mut harness.state_mut().events)
}

#[test]
fn check_tree_hotspot_label_space_context_collapse() {
    let mut harness = tree_harness();

    harness
        .get_by_role_and_label(Role::CheckBox, "@ace")
        .click();
    harness.run();
    assert_eq!(
        take_events(&mut harness),
        [CheckTreeEvent::Toggled("@ace".to_owned())]
    );

    harness.get_by_role_and_label(Role::Button, "@ace").click();
    harness.run();
    let events = take_events(&mut harness);
    assert_eq!(events, [CheckTreeEvent::RowClicked("@ace".to_owned())]);
    assert!(harness.state().tree.is_selected(&"@ace".to_owned()));

    harness.key_press(Key::Space);
    harness.run();
    assert_eq!(
        take_events(&mut harness),
        [CheckTreeEvent::Toggled("@ace".to_owned())]
    );

    let tfar_row = harness.get_by_role_and_label(Role::Button, "@tfar").rect();
    harness
        .get_by_role_and_label(Role::Button, "@tfar")
        .click_secondary();
    harness.run();
    assert_eq!(
        take_events(&mut harness),
        [CheckTreeEvent::ContextMenu(
            "@tfar".to_owned(),
            tfar_row.center()
        )]
    );
    assert!(harness.state().tree.is_selected(&"@tfar".to_owned()));

    assert_eq!(
        harness
            .get_by_role_and_label(Role::CheckBox, "@cba")
            .accesskit_node()
            .toggled(),
        Some(Toggled::Mixed)
    );
    assert_eq!(
        harness
            .get_by_role_and_label(Role::CheckBox, "@ace")
            .accesskit_node()
            .toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        harness
            .get_by_role_and_label(Role::CheckBox, "@tfar")
            .accesskit_node()
            .toggled(),
        Some(Toggled::False)
    );

    assert!(harness.query_all_by_label("cba_main.pbo").next().is_some());
    harness.get_by_label("Toggle @cba").click();
    harness.run();
    assert_eq!(
        take_events(&mut harness),
        [CheckTreeEvent::Expanded("@cba".to_owned(), false)]
    );
    assert!(harness.query_all_by_label("cba_main.pbo").next().is_none());
    assert!(harness.query_all_by_label("cba_xeh.pbo").next().is_none());
    assert!(harness.query_all_by_label("tfar_core.pbo").next().is_some());

    harness.get_by_label("Toggle @cba").click();
    harness.run();
    assert_eq!(
        take_events(&mut harness),
        [CheckTreeEvent::Expanded("@cba".to_owned(), true)]
    );
    assert!(harness.query_all_by_label("cba_main.pbo").next().is_some());
}

#[test]
fn check_tree_double_click() {
    let mut harness = tree_harness();
    let node = harness.get_by_role_and_label(Role::Button, "@tfar");
    node.click();
    node.click();
    harness.run();
    let events = take_events(&mut harness);
    assert!(
        events.contains(&CheckTreeEvent::RowDoubleClicked("@tfar".to_owned())),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, CheckTreeEvent::Toggled(_)))
    );
}

#[test]
fn space_without_focus_does_nothing() {
    let mut harness = tree_harness();
    harness.key_press(Key::Space);
    harness.run();
    assert!(take_events(&mut harness).is_empty());
}

struct ListState {
    list: CheckList,
    items: Vec<CheckItem>,
}

#[test]
fn check_list_toggles_and_highlights() {
    let state = ListState {
        list: CheckList::new("test_list"),
        items: vec![
            CheckItem::new("Profile A", false),
            CheckItem::new("Profile B", true),
            CheckItem::new("Profile C", false),
        ],
    };
    let mut harness = Harness::builder().with_size((300.0, 200.0)).build_ui_state(
        |ui, state: &mut ListState| {
            state.list.show(ui, &mut state.items);
        },
        state,
    );
    a3s_ui::install(&harness.ctx);
    harness.run();

    harness.get_by_label("Profile A").click();
    harness.run();
    let state = harness.state();
    assert!(state.items[0].checked);
    assert_eq!(state.list.selected(), Some(0));
    assert_eq!(checked_labels(&state.items), ["Profile A", "Profile B"]);
    assert_eq!(checked_indexes(&state.items), [0, 1]);
    assert_eq!(
        harness.get_by_label("Profile A").accesskit_node().toggled(),
        Some(Toggled::True)
    );

    harness.get_by_label("Profile B").click_secondary();
    harness.run();
    assert!(
        harness.state().items[1].checked,
        "secondary click does not toggle"
    );

    harness.get_by_label("Profile B").click();
    harness.run();
    assert_eq!(checked_labels(&harness.state().items), ["Profile A"]);
    assert_eq!(harness.state().list.selected(), Some(1));
}

#[derive(Clone, Copy)]
enum Content {
    Label,
    SingleLine,
    MultiLine,
}

struct DialogState {
    modal: bool,
    content: Content,
    text: String,
    actions: Vec<DialogAction>,
    build: fn(Dialog<'static>) -> Dialog<'static>,
}

fn dialog_harness(
    modal: bool,
    content: Content,
    build: fn(Dialog<'static>) -> Dialog<'static>,
) -> Harness<'static, DialogState> {
    let state = DialogState {
        modal,
        content,
        text: String::new(),
        actions: Vec::new(),
        build,
    };
    let mut harness = Harness::builder().with_size((800.0, 600.0)).build_ui_state(
        |ui, state: &mut DialogState| {
            ui.label("Main window");
            let dialog =
                (state.build)(Dialog::new("test_dialog", "Test dialog").modal(state.modal));
            let content = state.content;
            let text = &mut state.text;
            let action = dialog.show(ui.ctx(), |ui| match content {
                Content::Label => {
                    ui.label("Dialog content");
                }
                Content::SingleLine => {
                    ui.add(egui::TextEdit::singleline(text).id_salt("single"));
                }
                Content::MultiLine => {
                    ui.add(egui::TextEdit::multiline(text).id_salt("multi"));
                }
            });
            if action != DialogAction::None {
                state.actions.push(action);
            }
        },
        state,
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

fn identity(dialog: Dialog<'static>) -> Dialog<'static> {
    dialog
}

#[test]
fn modal_ok_cancel_close() {
    let mut harness = dialog_harness(true, Content::Label, identity);
    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Ok]);

    let mut harness = dialog_harness(true, Content::Label, identity);
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Cancel]);

    let mut harness = dialog_harness(true, Content::Label, identity);
    harness.get_by_label("Close window").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Close]);

    let mut harness = dialog_harness(true, Content::Label, identity);
    harness.key_press(Key::Escape);
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Close]);
}

#[test]
fn backdrop_click_does_nothing() {
    let mut harness = dialog_harness(true, Content::Label, identity);
    harness.get_by_label("Main window").click();
    harness.run();
    assert!(harness.state().actions.is_empty());
}

#[test]
fn custom_labels_and_hidden_ok() {
    let harness = dialog_harness(true, Content::Label, |d| {
        d.ok_label("Credits").cancel_label("Close")
    });
    assert!(harness.query_all_by_label("Credits").next().is_some());
    assert!(harness.query_all_by_label("Close").next().is_some());
    assert!(harness.query_all_by_label("OK").next().is_none());

    let harness = dialog_harness(true, Content::Label, |d| d.show_ok(false));
    assert!(harness.query_all_by_label("OK").next().is_none());
    assert!(harness.query_all_by_label("Cancel").next().is_some());
}

#[test]
fn enter_fires_default_button_from_single_line_edit() {
    let mut harness = dialog_harness(true, Content::SingleLine, |d| {
        d.default_button(Some(DialogButton::Ok))
    });
    harness.get_by_role(Role::TextInput).focus();
    harness.run();
    harness.get_by_role(Role::TextInput).type_text("hello");
    harness.run();
    assert!(harness.state().actions.is_empty());
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().text, "hello");
    assert_eq!(harness.state().actions, [DialogAction::Ok]);
}

#[test]
fn enter_in_multiline_edit_does_not_fire() {
    let mut harness = dialog_harness(true, Content::MultiLine, |d| {
        d.default_button(Some(DialogButton::Ok))
    });
    harness.get_by_role(Role::MultilineTextInput).focus();
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().actions.is_empty());
}

#[test]
fn non_modal_window_close() {
    let mut harness = dialog_harness(false, Content::Label, identity);
    assert!(harness.query_all_by_label("Test dialog").next().is_some());
    harness.get_by_label("Close window").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Close]);

    let mut harness = dialog_harness(false, Content::Label, identity);
    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Ok]);
}

struct ProgressState {
    dialog: ProgressDialog,
    actions: Vec<ProgressDialogAction>,
}

fn progress_harness() -> Harness<'static, ProgressState> {
    let mut dialog = ProgressDialog::new("test_progress", "Checking repository...");
    dialog.progress = Progress::Determinate {
        value: 50,
        min: 0,
        max: 100,
    };
    let state = ProgressState {
        dialog,
        actions: Vec::new(),
    };
    let mut harness = Harness::builder().with_size((800.0, 600.0)).build_ui_state(
        |ui, state: &mut ProgressState| {
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
fn progress_dialog() {
    let mut harness = progress_harness();
    assert!(
        harness
            .query_all_by_label(ProgressDialog::TITLE)
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by_label("Checking repository...")
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_by_role_and_label(Role::ProgressIndicator, "50%")
            .is_some()
    );
    assert!(!harness.state().dialog.canceled);
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().actions, [ProgressDialogAction::Cancel]);

    let mut harness = progress_harness();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().actions, [ProgressDialogAction::Cancel]);

    let mut harness = progress_harness();
    harness.get_by_label("Close window").click();
    harness.run();
    assert_eq!(harness.state().actions, [ProgressDialogAction::Cancel]);

    let mut harness = progress_harness();
    harness.key_press_modifiers(Modifiers::NONE, Key::Escape);
    harness.run();
    assert_eq!(harness.state().actions, [ProgressDialogAction::Cancel]);
}
