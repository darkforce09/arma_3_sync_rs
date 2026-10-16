//! The tool windows over fake mod, `TeamSpeak` and log folders: server keys, radio plugin and
//! game log.

use std::fs;
use std::io::Write as _;

use a3s_viewer::SecondaryWindow;
use egui_kittest::kittest::Queryable as _;

use crate::support::{
    Start, add_mod, enable_mods, fixture, mutate, offline_platform, open, radio_folders, wait_for,
    wait_for_label, write,
};

#[test]
fn server_keys_are_found_and_copied() {
    let mut fixture = fixture(Start::default());
    let dir = tempfile::tempdir().expect("temp dir");
    let mods = dir.path().join("mods");
    add_mod(&mods, "@ace", &["ace_3.bikey"]);
    add_mod(&mods, "@cba", &["cba_3.bikey"]);
    add_mod(&mods, "@unsigned", &[]);
    add_mod(&mods, "@disabled", &["disabled.bikey"]);
    let target = dir.path().join("server/keys");
    fs::create_dir_all(&target).expect("target");
    let viewer = fixture.harness.state().viewer();
    enable_mods(viewer, &mods, &["@ace", "@cba", "@unsigned"]);
    viewer
        .mutate(None, |state| {
            state
                .client
                .set_bikey_target_directory(Some(target.display().to_string()))
        })
        .expect("target");

    let harness = &mut fixture.harness;
    open(harness, SecondaryWindow::ServerKeys);
    wait_for_label(harness, "Copy 2 keys");
    harness.get_by_label("1 mod without a key");
    harness.get_by_label("Copy 2 keys").click();
    wait_for(harness, "the copied keys", |_| {
        target.join("ace_3.bikey").is_file() && target.join("cba_3.bikey").is_file()
    });
    wait_for(harness, "the result toast", |h| {
        h.query_by_label_contains("Copied 2 keys").is_some()
    });
    assert!(!target.join("disabled.bikey").exists());
}

#[test]
fn server_keys_of_a_folder_are_copied() {
    let mut fixture = fixture(Start::default());
    let dir = tempfile::tempdir().expect("temp dir");
    let server_mods = dir.path().join("server_mods");
    add_mod(&server_mods, "@ace", &["ace_3.bikey"]);
    add_mod(&server_mods, "@cba", &["cba_3.bikey"]);
    // As long as the temporary folders on macOS and Windows: the path widens the window after
    // its first frame and moves the controls.
    let target = dir.path().join("a".repeat(80)).join("server/keys");
    fs::create_dir_all(&target).expect("target");
    let viewer = fixture.harness.state().viewer();
    viewer
        .mutate(None, |state| {
            state
                .client
                .set_bikey_target_directory(Some(target.display().to_string()))?;
            state
                .client
                .set_bikey_source_directory(Some(server_mods.display().to_string()))
        })
        .expect("folders");

    let harness = &mut fixture.harness;
    open(harness, SecondaryWindow::ServerKeys);
    // By widget, not by the position drawn in the window's first frame.
    harness.get_by_label("A folder").click_accesskit();
    harness.run();
    wait_for_label(harness, "Copy 2 keys");
    harness.get_by_label("Copy 2 keys").click();
    wait_for(harness, "the copied keys", |_| {
        target.join("ace_3.bikey").is_file() && target.join("cba_3.bikey").is_file()
    });
}

#[test]
fn radio_plugin_installs_into_teamspeak() {
    let mut fixture = fixture(Start::default());
    let dir = tempfile::tempdir().expect("temp dir");
    let viewer = fixture.harness.state().viewer();
    let ts3 = radio_folders(viewer, dir.path(), &["ts3client_win64.exe"]);

    let harness = &mut fixture.harness;
    open(harness, SecondaryWindow::RadioPlugin);
    wait_for(harness, "the running warning", |h| {
        h.query_by_label_contains("TeamSpeak 3 is running")
            .is_some()
    });

    mutate(harness.state().viewer(), |state| {
        state.tools.platform = offline_platform(&[]);
    });
    wait_for(harness, "TeamSpeak to stop", |h| {
        h.query_by_label_contains("TeamSpeak 3 is running")
            .is_none()
    });
    harness.get_by_label("Install plugin").click();
    wait_for(harness, "the installed plugin", |_| {
        ts3.join("plugins/task_force_radio_win64.dll").is_file()
    });
    wait_for_label(harness, "Update plugin");
    assert!(ts3.join("plugins/TFAR/sounds.txt").is_file());
    assert!(
        dir.path()
            .join("arma3/userconfig/task_force_radio/radio_settings.hpp")
            .is_file()
    );
}

#[test]
fn game_log_follows_filters_and_shows_errors_only() {
    let mut fixture = fixture(Start::default());
    let dir = tempfile::tempdir().expect("temp dir");
    let rpt = dir.path().join("Arma3_x64_2026-10-13_20-00-00.rpt");
    write(
        &rpt,
        b"Starting the game\nWarning Message: No entry 'bin\\config.bin'\n",
    );
    let folder = dir.path().display().to_string();
    fixture
        .harness
        .state()
        .viewer()
        .mutate(None, |state| state.client.set_rpt_path(Some(folder)))
        .expect("folder");

    let harness = &mut fixture.harness;
    open(harness, SecondaryWindow::GameLog);
    wait_for_label(harness, "Starting the game");

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&rpt)
        .expect("open");
    file.write_all(b"Error in expression <player>\nMission loaded\n")
        .expect("append");
    drop(file);
    wait_for_label(harness, "Error in expression <player>");
    harness.get_by_label("Mission loaded");

    filter_field(harness).focus();
    harness.step();
    filter_field(harness).type_text("mission");
    wait_for(harness, "the filter", |h| {
        h.query_by_label("Starting the game").is_none()
    });
    harness.get_by_label("Mission loaded");

    for _ in "mission".chars() {
        harness.key_press(egui::Key::Backspace);
        harness.step();
    }
    wait_for_label(harness, "Starting the game");
    harness.get_by_label("Errors only").click();
    wait_for(harness, "errors only", |h| {
        h.query_by_label("Mission loaded").is_none()
            && h.query_by_label("Error in expression <player>").is_some()
    });
    assert!(harness.query_by_label_contains("Warning Message").is_none());
}

fn filter_field<'h>(
    harness: &'h egui_kittest::Harness<'static, a3s_viewer::Shell>,
) -> egui_kittest::Node<'h> {
    harness.get_by(|node| {
        node.placeholder()
            .is_some_and(|text| text.contains("Filter"))
    })
}
