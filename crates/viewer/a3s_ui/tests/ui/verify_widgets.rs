//! Adversarial verification of the widgets: Java rounding quirks, extreme progress ranges, tri-state
//! edge cases, check-tree hotspot geometry and keys, check-list toggling, dialog keyboard
//! routing and the font fallback before [`a3s_ui::install`].

use std::panic::{AssertUnwindSafe, catch_unwind};

use a3s_ui::dialog::{Dialog, DialogAction, DialogButton, ProgressDialog, ProgressDialogAction};
use a3s_ui::gallery::SampleTree;
use a3s_ui::theme::{self, ThemeLibrary, ThemeMode};
use a3s_ui::widgets::{
    CheckItem, CheckList, CheckState, CheckTree, CheckTreeEvent, CheckTreeModel, Progress,
    ProgressBar, percent_text,
};
use egui::accesskit::{Role, Toggled};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

// ---------------------------------------------------------------------------------------------
// Progress percent text (ui/UiStyle.java:181-188)

/// Java `Math.round(float)` (JDK 7+) is exact: `Math.round(0.49999997f) == 0`. Computing
/// `floor(x + 0.5f)` in f32 rounds `0.49999997f + 0.5f` up to `1.0f`, giving 1.
/// `(55925 * 100f) / 11185001` is exactly `0.49999997f`.
#[test]
fn percent_matches_java_math_round_float_just_below_half() {
    assert_eq!(percent_text(55_925, 0, 11_185_001), "0%");
    assert_eq!(percent_text(55_926, 0, 11_185_201), "0%");
}

#[test]
fn percent_half_up_edges_and_large_values() {
    for ((value, min, max), expected) in [
        ((0, 0, 200), "0%"),
        ((1, 0, 200), "1%"),     // 0.5 -> 1
        ((199, 0, 200), "100%"), // 99.5 -> 100
        ((200, 0, 200), "100%"),
        ((3, 0, 1000), "0%"), // 0.3
        ((5, 0, 1000), "1%"), // 0.5
        ((995, 0, 1000), "100%"),
        ((-1, -2, 0), "50%"),
        // Sizes beyond 2 GiB and i32.
        ((3_000_000_000, 0, 6_000_000_000), "50%"),
        ((i64::MAX, 0, i64::MAX), "100%"),
        ((i64::MAX, 0, 100), "100%"),
        ((i64::MIN, 0, 100), "0%"),
    ] {
        assert_eq!(
            percent_text(value, min, max),
            expected,
            "{value}, {min}, {max}"
        );
    }
}

/// A full i64 range must not overflow `value - min` (debug builds panic).
#[test]
fn percent_and_fraction_full_i64_range_do_not_panic() {
    let text = catch_unwind(|| percent_text(0, i64::MIN, i64::MAX));
    assert!(text.is_ok(), "percent_text(0, i64::MIN, i64::MAX) panicked");
    let fraction = catch_unwind(|| {
        Progress::Determinate {
            value: 0,
            min: i64::MIN,
            max: i64::MAX,
        }
        .fraction()
    });
    let fraction = fraction.expect("fraction() panicked").expect("determinate");
    assert!((0.0..=1.0).contains(&fraction));
}

// ---------------------------------------------------------------------------------------------
// Tri-state rule (ui/main/tree/CheckTreeCellRenderer.java:48-70)

#[test]
fn tri_state_extra_edges() {
    // Many children, exactly one unselected -> indeterminate.
    let children = (0..10_000).map(|i| i != 1234);
    assert_eq!(
        CheckState::for_node(true, false, true, children),
        CheckState::Indeterminate
    );
    // Exactly one selected among many -> indeterminate.
    let children = (0..10_000).map(|i| i == 0);
    assert_eq!(
        CheckState::for_node(false, false, true, children),
        CheckState::Indeterminate
    );
    // A leaf never looks at children, even if the caller passes some.
    assert_eq!(
        CheckState::for_node(true, true, true, [true, false]),
        CheckState::Checked
    );
    // The root never becomes indeterminate, whatever the children.
    assert_eq!(
        CheckState::for_node(true, false, false, (0..100).map(|i| i % 2 == 0)),
        CheckState::Checked
    );
}

// ---------------------------------------------------------------------------------------------
// Check tree widget

struct TreeState<M: CheckTreeModel> {
    tree: CheckTree,
    model: M,
    events: Vec<CheckTreeEvent<M::Key>>,
}

fn tree_harness<M: CheckTreeModel + 'static>(model: M) -> Harness<'static, TreeState<M>> {
    let state = TreeState {
        tree: CheckTree::new("verify_tree").default_expanded(true),
        model,
        events: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size((400.0, 500.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, state: &mut TreeState<M>| {
                let events = state.tree.show(ui, &state.model);
                state.events.extend(events);
            },
            state,
        );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

fn take<M: CheckTreeModel>(harness: &mut Harness<'_, TreeState<M>>) -> Vec<CheckTreeEvent<M::Key>> {
    std::mem::take(&mut harness.state_mut().events)
}

fn click_at<S>(harness: &mut Harness<'_, S>, pos: Pos2, button: PointerButton) {
    harness.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.run();
}

fn s(text: &str) -> String {
    text.to_owned()
}

#[test]
fn root_with_mixed_children_is_never_mixed_in_the_widget() {
    let harness = tree_harness(SampleTree::default());
    let root = harness.get_by_role_and_label(Role::CheckBox, "@mods");
    assert_eq!(root.accesskit_node().toggled(), Some(Toggled::False));
    // Leaves are plain checkboxes.
    let leaf = harness.get_by_role_and_label(Role::CheckBox, "cba_xeh.pbo");
    assert_eq!(leaf.accesskit_node().toggled(), Some(Toggled::False));
}

#[test]
fn hotspot_edges_label_and_expander() {
    let mut harness = tree_harness(SampleTree::default());
    let check = harness
        .get_by_role_and_label(Role::CheckBox, "@tfar")
        .rect();
    assert!(
        (check.width() - 22.0).abs() < 0.5,
        "hotspot = JCheckBox preferred width, got {}",
        check.width()
    );

    // Left edge of the hotspot.
    click_at(
        &mut harness,
        pos2(check.left() + 1.0, check.center().y),
        PointerButton::Primary,
    );
    assert_eq!(take(&mut harness), [CheckTreeEvent::Toggled(s("@tfar"))]);

    // Just right of the hotspot: selects the row, no toggle.
    click_at(
        &mut harness,
        pos2(check.right() + 2.0, check.center().y),
        PointerButton::Primary,
    );
    assert_eq!(take(&mut harness), [CheckTreeEvent::RowClicked(s("@tfar"))]);

    // On the label text (after the 18 pt icon and its gap).
    click_at(
        &mut harness,
        pos2(check.right() + 18.0 + 4.0 + 8.0, check.center().y),
        PointerButton::Primary,
    );
    assert_eq!(take(&mut harness), [CheckTreeEvent::RowClicked(s("@tfar"))]);

    // The expander, left of the hotspot, only collapses.
    click_at(
        &mut harness,
        pos2(check.left() - 8.0, check.center().y),
        PointerButton::Primary,
    );
    assert_eq!(
        take(&mut harness),
        [CheckTreeEvent::Expanded(s("@tfar"), false)]
    );
    assert!(harness.query_all_by_label("tfar_core.pbo").next().is_none());
}

#[test]
fn space_after_hotspot_click_toggles_each_press() {
    let mut harness = tree_harness(SampleTree::default());
    harness
        .get_by_role_and_label(Role::CheckBox, "@cba")
        .click();
    harness.run();
    assert_eq!(take(&mut harness), [CheckTreeEvent::Toggled(s("@cba"))]);

    harness.key_press(Key::Space);
    harness.run();
    harness.key_press(Key::Space);
    harness.run();
    assert_eq!(
        take(&mut harness),
        [
            CheckTreeEvent::Toggled(s("@cba")),
            CheckTreeEvent::Toggled(s("@cba"))
        ]
    );
}

#[test]
fn secondary_click_on_hotspot_does_not_toggle() {
    let mut harness = tree_harness(SampleTree::default());
    harness
        .get_by_role_and_label(Role::CheckBox, "@ace")
        .click_secondary();
    harness.run();
    let events = take(&mut harness);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, CheckTreeEvent::Toggled(_))),
        "{events:?}"
    );
    // ContextMenu carries the pointer position.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, CheckTreeEvent::ContextMenu(k, _) if k == "@ace")),
        "{events:?}"
    );
}

/// Java `JTree` moves the selection with the arrow keys, and Space then toggles the newly
/// selected row (`ui/main/AddonsPanel.java:526`).
#[test]
fn arrow_down_moves_selection_like_jtree() {
    let mut harness = tree_harness(SampleTree::default());
    harness.get_by_role_and_label(Role::Button, "@cba").click();
    harness.run();
    take(&mut harness);
    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.key_press(Key::Space);
    harness.run();
    assert_eq!(
        take(&mut harness),
        [CheckTreeEvent::Toggled(s("cba_main.pbo"))]
    );
}

/// Flat model with case-differing and non-ASCII keys.
struct FlatModel {
    keys: Vec<String>,
}

impl CheckTreeModel for FlatModel {
    type Key = String;

    fn roots(&self) -> Vec<String> {
        self.keys.clone()
    }

    fn children(&self, _key: &String) -> Vec<String> {
        Vec::new()
    }

    fn is_leaf(&self, _key: &String) -> bool {
        true
    }

    fn label(&self, key: &String) -> String {
        key.clone()
    }

    fn check_state(&self, key: &String) -> CheckState {
        CheckState::from_selected(key.starts_with('@'))
    }
}

#[test]
fn case_differing_and_unicode_keys_are_distinct_rows() {
    let keys = vec![
        s("@Mod"),
        s("@mod"),
        s("Ünïcødé 日本語 ✓"),
        s("@mod\u{0301}"),
        s("dir\\with\\backslashes"),
    ];
    let mut harness = tree_harness(FlatModel { keys: keys.clone() });
    for key in &keys {
        assert!(
            harness
                .query_by_role_and_label(Role::CheckBox, key)
                .is_some(),
            "{key} is shown"
        );
    }
    for key in &keys {
        harness.get_by_role_and_label(Role::CheckBox, key).click();
        harness.run();
        assert_eq!(take(&mut harness), [CheckTreeEvent::Toggled(key.clone())]);
    }
    // Selecting "@mod" must not select "@Mod".
    harness.get_by_role_and_label(Role::Button, "@mod").click();
    harness.run();
    take(&mut harness);
    assert!(harness.state().tree.is_selected(&s("@mod")));
    assert!(!harness.state().tree.is_selected(&s("@Mod")));
}

/// One root with many leaves: rendering must be virtualised.
struct BigModel {
    leaves: usize,
}

impl CheckTreeModel for BigModel {
    type Key = usize;

    fn roots(&self) -> Vec<usize> {
        vec![usize::MAX]
    }

    fn children(&self, key: &usize) -> Vec<usize> {
        if *key == usize::MAX {
            (0..self.leaves).collect()
        } else {
            Vec::new()
        }
    }

    fn is_leaf(&self, key: &usize) -> bool {
        *key != usize::MAX
    }

    fn label(&self, key: &usize) -> String {
        if *key == usize::MAX {
            s("root")
        } else {
            format!("leaf{key}")
        }
    }

    fn check_state(&self, key: &usize) -> CheckState {
        CheckState::from_selected(key.is_multiple_of(2))
    }
}

#[test]
fn fifty_thousand_rows_are_virtualised() {
    let harness = tree_harness(BigModel { leaves: 50_000 });
    let shown = harness.query_all_by_role(Role::CheckBox).count();
    assert!(shown > 5 && shown < 200, "{shown} checkbox nodes built");
    assert!(harness.query_all_by_label("leaf0").next().is_some());
    assert!(harness.query_all_by_label("leaf49999").next().is_none());
}

// ---------------------------------------------------------------------------------------------
// Check list (ui/CheckBoxList.java)

struct ListState {
    list: CheckList,
    items: Vec<CheckItem>,
}

fn list_harness(items: Vec<CheckItem>) -> Harness<'static, ListState> {
    let state = ListState {
        list: CheckList::new("verify_list"),
        items,
    };
    let mut harness = Harness::builder()
        .with_size((300.0, 200.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, state: &mut ListState| {
                state.list.show(ui, &mut state.items);
            },
            state,
        );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

#[test]
fn check_list_toggles_anywhere_on_the_row() {
    let mut harness = list_harness(vec![CheckItem::new("A", false), CheckItem::new("B", false)]);
    let row = harness.get_by_label("B").rect();
    click_at(
        &mut harness,
        pos2(row.right() - 3.0, row.center().y),
        PointerButton::Primary,
    );
    assert!(
        harness.state().items[1].checked,
        "far right of the row toggles"
    );
    click_at(
        &mut harness,
        pos2(row.left() + 3.0, row.center().y),
        PointerButton::Primary,
    );
    assert!(!harness.state().items[1].checked, "left edge toggles back");
    assert_eq!(harness.state().list.selected(), Some(1));
    assert!(!harness.state().items[0].checked);
}

#[test]
fn check_list_empty_does_not_panic() {
    let harness = list_harness(Vec::new());
    assert_eq!(harness.state().list.selected(), None);
}

// ---------------------------------------------------------------------------------------------
// Dialogs

struct DialogState {
    default_button: Option<DialogButton>,
    modal: bool,
    actions: Vec<DialogAction>,
}

fn dialog_harness(
    modal: bool,
    default_button: Option<DialogButton>,
) -> Harness<'static, DialogState> {
    let state = DialogState {
        default_button,
        modal,
        actions: Vec::new(),
    };
    let mut harness = Harness::builder().with_size((800.0, 600.0)).build_ui_state(
        |ui, state: &mut DialogState| {
            ui.label("Main window");
            let action = Dialog::new("verify_dialog", "Verify")
                .modal(state.modal)
                .default_button(state.default_button)
                .show(ui.ctx(), |ui| {
                    ui.label("Body");
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

#[test]
fn enter_without_default_button_does_nothing() {
    for modal in [true, false] {
        let mut harness = dialog_harness(modal, None);
        harness.key_press(Key::Enter);
        harness.run();
        assert!(harness.state().actions.is_empty(), "modal = {modal}");
    }
}

#[test]
fn enter_fires_default_cancel_and_non_modal_default_ok() {
    let mut harness = dialog_harness(true, Some(DialogButton::Cancel));
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Cancel]);

    let mut harness = dialog_harness(false, Some(DialogButton::Ok));
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Ok]);
}

#[test]
fn escape_closes_non_modal_window() {
    let mut harness = dialog_harness(false, None);
    harness.key_press(Key::Escape);
    harness.run();
    assert_eq!(harness.state().actions, [DialogAction::Close]);
}

struct StackState {
    progress: ProgressDialog,
    progress_actions: Vec<ProgressDialogAction>,
    modal_actions: Vec<DialogAction>,
}

/// A progress dialog stays open while a modal dialog is shown on top of it (same frame,
/// progress dialog drawn first as a caller naturally would).
fn stacked_harness() -> Harness<'static, StackState> {
    let mut progress = ProgressDialog::new("verify_stack_progress", "Downloading...");
    progress.progress = Progress::of(10, 100);
    let state = StackState {
        progress,
        progress_actions: Vec::new(),
        modal_actions: Vec::new(),
    };
    let mut harness = Harness::builder().with_size((800.0, 600.0)).build_ui_state(
        |ui, state: &mut StackState| {
            let p = state.progress.show(ui.ctx());
            if p != ProgressDialogAction::None {
                state.progress_actions.push(p);
            }
            let m = Dialog::new("verify_stack_modal", "Confirm").show(ui.ctx(), |ui| {
                ui.label("Are you sure?");
            });
            if m != DialogAction::None {
                state.modal_actions.push(m);
            }
        },
        state,
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    harness
}

/// A modal blocks the windows behind it: Enter must not reach (and cancel) the progress dialog
/// behind the modal. Java's default button only fires in the focused window.
#[test]
fn enter_does_not_cancel_progress_dialog_behind_a_modal() {
    let mut harness = stacked_harness();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(
        harness.state().progress_actions.is_empty(),
        "progress dialog behind a modal was cancelled by Enter"
    );
}

/// Escape belongs to the topmost dialog (the modal), not the progress dialog behind it.
#[test]
fn escape_goes_to_the_modal_not_the_progress_dialog_behind_it() {
    let mut harness = stacked_harness();
    harness.key_press(Key::Escape);
    harness.run();
    assert!(
        harness.state().progress_actions.is_empty(),
        "progress dialog behind a modal was cancelled by Escape"
    );
    assert_eq!(harness.state().modal_actions, [DialogAction::Close]);
}

// ---------------------------------------------------------------------------------------------
// Fonts before install / theme switching

/// egui applies new fonts at the start of the next pass while `set_style_of` is immediate, so a
/// heading drawn in the same frame as `install` uses the `Geist-SemiBold` family before it is
/// loaded.
#[test]
fn heading_in_the_same_frame_as_install_does_not_panic() {
    let ctx = egui::Context::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            a3s_ui::install(ui.ctx());
            ui.heading("Arma3Sync");
        });
        output.textures_delta.clear();
    }));
    assert!(result.is_ok(), "heading panicked in the install frame");
}

#[test]
fn widgets_before_install_use_egui_defaults() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        ui.add(ProgressBar::new(Progress::of(1, 3)));
        ui.add(ProgressBar::new(Progress::Indeterminate {
            text: Some(s("Connecting...")),
        }));
        let _: DialogAction = Dialog::new("pre_install", "Before install").show(ui.ctx(), |ui| {
            ui.label("x");
        });
    });
    output.textures_delta.clear();
}

#[test]
fn installed_styles_follow_both_palettes() {
    let ctx = egui::Context::default();
    a3s_ui::install(&ctx);
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let def = ThemeLibrary::for_mode(mode);
        let style = ctx.style_of(mode.into());
        assert_eq!(style.visuals.panel_fill, def.palette.surface, "{mode:?}");
        assert_eq!(
            style.visuals.widgets.hovered.bg_fill,
            def.palette.state_hover_overlay
        );
        assert_eq!(style.visuals.dark_mode, def.dark);
    }
    theme::set_mode(&ctx, ThemeMode::Light);
    assert_eq!(theme::palette(&ctx), &ThemeLibrary::light().palette);
}
