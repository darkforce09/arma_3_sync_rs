//! The Repository screen: its "⋯" menu, the Download settings sheet, and a failed check.

use std::convert::Infallible;

use a3s_foundation::error::ErrorKind;
use a3s_sync::EventError;
use a3s_ui::icon::icons;
use a3s_viewer::{Screen, SheetKind};
use egui_kittest::kittest::Queryable as _;

use crate::support::{Start, fixture};

/// Shows the Alpha repository through its sidebar row (the Mods tree has an Alpha group too).
fn open_alpha(harness: &mut egui_kittest::Harness<'static, a3s_viewer::Shell>) {
    harness
        .get_all_by_label("Alpha")
        .next()
        .expect("sidebar row")
        .click();
    harness.run();
}

/// Opens the "⋯" menu of the repository screen (the toolbar's menu comes first).
fn open_repository_menu(harness: &mut egui_kittest::Harness<'static, a3s_viewer::Shell>) {
    harness
        .get_all_by_label(icons::DOTS_THREE_CIRCLE)
        .last()
        .expect("repository menu")
        .click();
    harness.run();
}

#[test]
fn download_settings_change_only_their_repository() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    open_alpha(harness);
    open_repository_menu(harness);
    harness.get_by_label("Download settings…").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::DownloadSettings));

    harness.get_by_label("Notify me about updates").click();
    harness.run();
    harness
        .get_by_label("Download updates automatically")
        .click();
    harness.run();
    {
        let state = harness.state().viewer().read();
        let alpha = state.client.repository("Alpha").expect("Alpha");
        let beta = state.client.repository("Beta").expect("Beta");
        assert!(alpha.notify && alpha.auto);
        assert!(!beta.notify && !beta.auto, "Beta keeps its settings");
    }

    harness.get_by_label("Done").click();
    harness.run();
    assert_eq!(harness.state().sheet(), None);
}

#[test]
fn publish_from_this_computer_asks_for_the_source_folder() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    open_alpha(harness);
    open_repository_menu(harness);
    harness.get_by_label("Publish from this computer…").click();
    harness.run();
    assert_eq!(
        *harness.state().screen(),
        Screen::Publishing("Alpha".to_owned())
    );
    harness.get_by_label("Choose source folder…");
}

#[test]
fn publish_a_folder_opens_its_sheet() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness.get_by_label("Publish a folder").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::PublishFolder));
    harness.get_by_label("Source folder");
    assert!(harness.query_by_label("Auto-config link").is_none());
}

#[test]
fn a_failed_check_shows_its_reason() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.repositories.runtime_mut("Alpha").last_check_error = Some(EventError::new(
                ErrorKind::ConnectionFailed,
                "Connection refused",
            ));
            Ok::<(), Infallible>(())
        })
        .ok();
    open_alpha(harness);
    harness.get_by_label_contains("Cannot check the repository: Connection refused");
    harness.get_by_label("Check again");
}
