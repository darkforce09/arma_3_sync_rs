//! The Mods screen: selection, search, load order, Play and Arma 3 Launcher presets.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_repo_files::services::ClientData;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Profile, TreeDirectory, TreeLeaf, TreeNode};
use a3s_ui::icon::icons;
use a3s_viewer::{SettingsPane, SheetKind, Shell};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use tempfile::TempDir;

use crate::support::{
    Fixture, RecordingLauncher, Start, fixture, mods_folder, mods_profile, set_status, step_until,
    wait_for_scan,
};

/// The Mods screen over the `Mods` profile, scanned.
fn mods_fixture() -> (Fixture, TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mods, exe) = mods_folder(dir.path());
    let mut fixture = fixture(Start {
        profiles: Some(vec![mods_profile(&mods, &exe)]),
        ..Start::default()
    });
    wait_for_scan(&mut fixture.harness);
    (fixture, dir, exe)
}

fn active_profile(harness: &Harness<'static, Shell>) -> Profile {
    harness
        .state()
        .viewer()
        .read()
        .client
        .active_profile()
        .cloned()
        .expect("active profile")
}

fn leaf_selected(profile: &Profile, group: &str, leaf: &str) -> bool {
    profile
        .tree
        .directories()
        .find(|d| d.name == group)
        .and_then(|d| d.leaves().find(|l| l.name == leaf))
        .is_some_and(|l| l.selected)
}

#[test]
fn toggling_rows_changes_the_profile_selection() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    assert!(leaf_selected(&active_profile(harness), "Core", "@ace"));

    // The group's row comes first; the same addon is listed again under its folder.
    harness
        .get_all_by_role_and_label(Role::CheckBox, "@ace")
        .next()
        .expect("@ace in Core")
        .click();
    harness.run();
    assert!(!leaf_selected(&active_profile(harness), "Core", "@ace"));

    // Space toggles the selected row.
    harness.key_press(Key::Space);
    harness.run();
    assert!(leaf_selected(&active_profile(harness), "Core", "@ace"));

    // An addon of the folder is enabled on its own.
    harness.get_by_role_and_label(Role::CheckBox, "@gm").click();
    harness.run();
    let profile = active_profile(harness);
    assert!(profile.tree.children.iter().any(|node| matches!(
        node,
        TreeNode::Leaf(leaf) if leaf.name.eq_ignore_ascii_case("@gm") && leaf.selected
    )));

    // Toggling a group sets all its addons.
    harness
        .get_by_role_and_label(Role::CheckBox, "Core")
        .click();
    harness.run();
    let profile = active_profile(harness);
    assert!(!leaf_selected(&profile, "Core", "@ace"));
    assert!(!leaf_selected(&profile, "Core", "@cba"));
}

#[test]
fn search_narrows_the_tree() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    assert!(harness.query_all_by_label("@ace").count() > 0);

    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    harness.run();
    harness.event(egui::Event::Text("cba".to_owned()));
    harness.run();
    assert_eq!(harness.query_all_by_label("@ace").count(), 0);
    assert!(harness.query_all_by_label("@cba").count() > 0);
    assert!(harness.query_all_by_label("@gm").count() == 0);
}

#[test]
fn dragging_in_the_load_order_reorders_and_saves() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let store = Arc::clone(&fixture.store);
    let harness = &mut fixture.harness;
    harness.get_by_label("Load order").click();
    harness.run();

    let handles: Vec<egui::Rect> = harness
        .get_all_by_label(icons::DOTS_SIX_VERTICAL)
        .map(|node| node.rect())
        .collect();
    assert_eq!(handles.len(), 2, "@cba and @ace are enabled");
    let from = handles[0].center();
    let to = handles[1].center() + egui::vec2(0.0, handles[1].height());
    harness.drag_at(from);
    // egui_dnd repaints while dragging and animating.
    harness.run_steps(2);
    for step in 1..=10 {
        harness.hover_at(from + (to - from) * (step as f32 / 10.0));
        harness.step();
    }
    harness.drop_at(to);
    harness.run_steps(40);

    let expected = vec!["@ace".to_owned(), "@cba".to_owned()];
    let order = |profile: &Profile| -> Vec<String> {
        profile
            .addon_names_by_priority
            .iter()
            .filter(|name| expected.contains(&name.to_lowercase()))
            .map(|name| name.to_lowercase())
            .collect()
    };
    assert_eq!(order(&active_profile(harness)), expected);
    let reloaded = ClientData::load(store).data;
    assert_eq!(
        order(reloaded.profile("Mods").expect("saved profile")),
        expected
    );
}

#[test]
fn play_plans_the_game_with_the_enabled_mods_in_load_order() {
    let (mut fixture, _dir, exe) = mods_fixture();
    let harness = &mut fixture.harness;
    let launcher = Arc::new(RecordingLauncher::default());
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.launch.launcher = launcher.clone();
            Ok::<_, Infallible>(())
        })
        .ok();
    set_status(harness.state().viewer(), "Alpha", RepositoryStatus::Ok);
    harness.run();
    harness.get_by_label_contains("Play").click();
    step_until(harness, |_| launcher.plan.lock().is_some());

    let plan = launcher.plan.lock().clone().expect("plan");
    assert_eq!(Path::new(&plan.command.program), exe.as_path());
    let mods: Vec<String> = plan
        .command
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .filter(|arg| arg.starts_with("-mod="))
        .collect();
    assert_eq!(mods.len(), 2, "{mods:?}");
    assert!(mods[0].ends_with("@cba;"), "{mods:?}");
    assert!(mods[1].ends_with("@ace;"), "{mods:?}");
}

#[test]
fn play_without_arma_3_offers_to_choose_it() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state
                .client
                .update_launcher_options(|options| options.arma3_exe_path = None)
        })
        .expect("cleared executable");
    set_status(harness.state().viewer(), "Alpha", RepositoryStatus::Ok);
    harness.run();
    harness.get_by_label_contains("Play").click();
    harness.run();
    harness.get_by_label("Choose folder…").click();
    harness.run();
    assert_eq!(harness.state().settings_pane(), Some(SettingsPane::Game));
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|err| err.to_string())
    }
}

#[test]
fn a_dropped_preset_is_compared_and_its_matched_mods_enabled() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    let preset = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_presets/p1_golden/preset.html");
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(preset)));
    step_until(harness, |shell| {
        shell.sheet() == Some(SheetKind::LauncherPreset)
    });
    // The Workshop mod is @gm on this computer; the CDLC is missing.
    assert!(harness.query_all_by_label("@gm").count() > 0);

    harness.get_by_label("Enable matched").click();
    harness.run();
    assert_eq!(harness.state().sheet(), None);
    let profile = active_profile(harness);
    let group = profile
        .tree
        .directories()
        .find(|group| {
            group
                .leaves()
                .any(|leaf| leaf.name.eq_ignore_ascii_case("@gm"))
        })
        .expect("imported group");
    assert!(group.leaves().all(|leaf| leaf.selected));
    assert_eq!(group.leaves().count(), 1, "only the matched mod");
}

/// The row of `label` (its label, not its checkbox).
fn row(harness: &Harness<'static, Shell>, label: &str) -> egui::Rect {
    harness
        .get_all_by_label(label)
        .find(|node| node.accesskit_node().role() != Role::CheckBox)
        .expect("row")
        .rect()
}

fn group<'p>(profile: &'p Profile, name: &str) -> Option<&'p TreeDirectory> {
    profile.tree.directories().find(|d| d.name == name)
}

#[test]
fn a_group_is_duplicated_from_its_menu() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    harness
        .get_all_by_label("Core")
        .find(|node| node.accesskit_node().role() != Role::CheckBox)
        .expect("Core row")
        .click_secondary();
    harness.run();
    harness.get_by_label("Duplicate").click();
    harness.run();
    // The copy is named at once; keeping the proposed name is fine.
    harness.get_by_label("Rename").click();
    harness.run();

    let profile = active_profile(harness);
    let copy = group(&profile, "Core copy").expect("the copy");
    assert_eq!(copy.modset_type, None);
    assert!(
        copy.leaves()
            .any(|leaf| leaf.name == "@ace" && !leaf.selected)
    );
    assert!(
        leaf_selected(&profile, "Core", "@ace"),
        "the original stays"
    );
}

#[test]
fn a_mod_from_this_computer_is_dragged_into_a_group() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    let from = row(harness, "@gm").center();
    let to = row(harness, "Core").center();
    harness.drag_at(from);
    harness.run_steps(2);
    for step in 1..=10 {
        harness.hover_at(from + (to - from) * (step as f32 / 10.0));
        harness.step();
    }
    harness.drop_at(to);
    harness.run_steps(4);

    let profile = active_profile(harness);
    assert!(
        group(&profile, "Core")
            .expect("Core")
            .leaves()
            .any(|leaf| leaf.name.eq_ignore_ascii_case("@gm") && leaf.selected)
    );
}

#[test]
fn the_mods_menu_disables_every_mod() {
    let (mut fixture, _dir, _exe) = mods_fixture();
    let harness = &mut fixture.harness;
    harness
        .get_all_by_label(icons::DOTS_THREE_CIRCLE)
        .last()
        .expect("Mods menu")
        .click();
    harness.run();
    harness.get_by_label("Disable all mods").click();
    harness.run();
    let profile = active_profile(harness);
    assert!(!leaf_selected(&profile, "Core", "@ace"));
    assert!(!leaf_selected(&profile, "Core", "@cba"));
}

/// The Mods fixture with `edit` applied to its profile, and a recording launcher.
fn play_fixture(edit: impl FnOnce(&mut Profile)) -> (Fixture, TempDir, Arc<RecordingLauncher>) {
    let (fixture, dir, _exe) = mods_fixture();
    let launcher = Arc::new(RecordingLauncher::default());
    fixture
        .harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.launch.launcher = launcher.clone();
            state.client.update_active_profile(|profile| {
                edit(profile);
                true
            })
        })
        .expect("profile");
    set_status(
        fixture.harness.state().viewer(),
        "Alpha",
        RepositoryStatus::Ok,
    );
    (fixture, dir, launcher)
}

#[test]
fn play_asks_first_when_enabled_mods_are_missing() {
    let (mut fixture, _dir, launcher) = play_fixture(|profile| {
        let mut extra = TreeDirectory::new("Extra");
        extra.add_child(TreeNode::Leaf(TreeLeaf {
            selected: true,
            ..TreeLeaf::new("@not_here")
        }));
        profile.tree.add_child(TreeNode::Directory(extra));
    });
    let harness = &mut fixture.harness;
    harness.run();
    harness.get_by_label_contains("Play").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::PlayCheck));
    assert!(
        launcher.plan.lock().is_none(),
        "nothing starts before the answer"
    );

    harness.get_by_label("Play without them").click();
    step_until(harness, |_| launcher.plan.lock().is_some());
}

#[test]
fn play_refuses_mods_enabled_twice() {
    let (mut fixture, _dir, launcher) = play_fixture(|profile| {
        let mut again = TreeDirectory::new("Again");
        again.add_child(TreeNode::Leaf(TreeLeaf {
            selected: true,
            ..TreeLeaf::new("@ace")
        }));
        profile.tree.add_child(TreeNode::Directory(again));
    });
    let harness = &mut fixture.harness;
    harness.run();
    harness.get_by_label_contains("Play").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::PlayCheck));
    harness.get_by_label_contains("enabled in two groups");
    harness.get_by_label("OK").click();
    harness.run_steps(10);
    assert!(launcher.plan.lock().is_none(), "the game does not start");
}
