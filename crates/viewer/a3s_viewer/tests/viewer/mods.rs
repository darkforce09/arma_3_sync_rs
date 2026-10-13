//! The Mods screen: selection, search, load order, Play and Arma 3 Launcher presets.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_foundation::paths::Os;
use a3s_launch::{EventSink, LaunchError, LaunchHandle, LaunchPlan, RunOptions};
use a3s_repo_files::services::ClientData;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{ExternalApplication, Profile, TreeNode};
use a3s_ui::icon::icons;
use a3s_viewer::{SettingsPane, SheetKind, Shell};
use a3s_viewer_context::launch::{GameLauncher, LaunchEnvironment};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use parking_lot::Mutex;
use tempfile::TempDir;

use crate::support::{
    Fixture, Start, fixture, mods_folder, mods_profile, set_status, step_until, wait_for_scan,
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

/// Records the planned game launch instead of starting it.
#[derive(Default)]
struct RecordingLauncher {
    plan: Mutex<Option<LaunchPlan>>,
}

impl GameLauncher for RecordingLauncher {
    fn environment(&self) -> LaunchEnvironment {
        LaunchEnvironment {
            os: Os::Linux,
            steam: None,
            arma3_documents_dir: None,
        }
    }

    fn start_companions(&self, _os: Os, _apps: &[ExternalApplication], _events: &EventSink) {}

    fn start_game(
        &self,
        plan: LaunchPlan,
        _options: RunOptions,
        _events: EventSink,
    ) -> Result<Option<LaunchHandle>, LaunchError> {
        *self.plan.lock() = Some(plan);
        Ok(None)
    }
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
