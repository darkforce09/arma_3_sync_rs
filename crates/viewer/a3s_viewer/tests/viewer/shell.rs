//! Routing, sheets, the Play button, the Undo toast and quitting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_types::constants::RepositoryStatus;
use a3s_viewer::{Screen, SecondaryWindow, SheetKind, WelcomeSetup, WindowRequest};
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;

use crate::support::{Start, fixture, set_status};

#[test]
fn sidebar_rows_change_the_screen() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    assert_eq!(*harness.state().screen(), Screen::Mods);

    harness.get_by_label("Servers").click();
    harness.run();
    assert_eq!(*harness.state().screen(), Screen::Servers);

    harness.get_by_label("Alpha").click();
    harness.run();
    assert_eq!(
        *harness.state().screen(),
        Screen::Repository("Alpha".to_owned())
    );

    // Beta is listed twice: under Repositories and under Publishing.
    let rows = harness.get_all_by_label("Beta").count();
    assert_eq!(rows, 2);
    harness
        .get_all_by_label("Beta")
        .last()
        .expect("publishing row")
        .click();
    harness.run();
    assert_eq!(
        *harness.state().screen(),
        Screen::Publishing("Beta".to_owned())
    );
}

#[test]
fn sheets_open_in_order_and_close() {
    let mut fixture = fixture(Start {
        load_warnings: vec!["The profiles could not be read.".to_owned()],
        welcome: Some(WelcomeSetup::default()),
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    assert_eq!(harness.state().sheet(), Some(SheetKind::Welcome));

    harness.get_by_label("Skip").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::LoadWarnings));

    harness.get_by_label("OK").click();
    harness.run();
    assert_eq!(harness.state().sheet(), None);

    harness.get_by_label("Add repository").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::AddRepository));
    harness.key_press(Key::Escape);
    harness.run();
    assert_eq!(harness.state().sheet(), None);
}

#[test]
fn play_says_update_while_the_profile_has_updates() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness.get_by_label_contains("Update and play");

    set_status(harness.state().viewer(), "Alpha", RepositoryStatus::Ok);
    harness.run();
    assert!(harness.query_by_label_contains("Update and play").is_none());
    harness.get_by_label_contains("Play");
}

#[test]
fn undo_runs_its_callback_once() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    let undone = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&undone);
    harness
        .state_mut()
        .toasts()
        .undoable("Removed Alpha", move |_viewer| {
            assert!(!flag.swap(true, Ordering::SeqCst), "undo ran twice");
        });
    harness.run_steps(2);
    harness.get_by_label("Undo").click();
    harness.run_steps(4);
    assert!(undone.load(Ordering::SeqCst));
    assert!(harness.query_by_label("Undo").is_none());
}

#[test]
fn settings_shortcut_opens_the_settings_window() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    harness.run();
    assert!(harness.state().is_open(SecondaryWindow::Settings));
    harness.get_by_label("Companion apps");
}

#[test]
fn quit_from_the_tray_saves_the_settings() {
    let mut fixture = fixture(Start::default());
    assert!(!fixture.store.configuration_path().exists());
    fixture.requests.send(WindowRequest::Quit);
    fixture.harness.run();
    assert!(fixture.store.configuration_path().exists());
    assert_eq!(fixture.harness.state().sheet(), None);
}
