//! The Settings window: changes are written at once, removals undo, the check schedule follows
//! the frequency, and the command-line preview follows the launch options.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use a3s_types::ProtocolType;
use a3s_types::constants::{CheckRepositoriesFrequency, MinimizationType};
use a3s_viewer::SettingsPane;
use a3s_viewer_context::{AppEvent, ContextOptions, ManualClock, TaskEvent, TaskKind};
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;

use crate::settings_support::{SettingsStart, settings};

#[test]
fn keep_in_tray_is_written_at_once() {
    let mut fixture = settings(SettingsStart::pane(SettingsPane::General));
    fixture
        .harness
        .get_by_label("Keep running in the tray")
        .click();
    fixture.harness.run();

    let in_memory = fixture
        .viewer()
        .read()
        .client
        .preferences()
        .launch_panel_minimized;
    assert_eq!(in_memory, MinimizationType::Tray);
    assert_eq!(
        fixture.reload().preferences().launch_panel_minimized,
        MinimizationType::Tray
    );
}

/// Waits up to five seconds for a task of `kind` to start (or, with `finished`, to end).
fn wait_for(viewer: &a3s_viewer_context::ViewerContext, kind: TaskKind, finished: bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match viewer.next_event_timeout(left) {
            Some(AppEvent::Task(TaskEvent::Started(info)))
                if !finished && info.key.kind == kind =>
            {
                return true;
            }
            Some(AppEvent::Task(TaskEvent::Finished { info, .. }))
                if finished && info.key.kind == kind =>
            {
                return true;
            }
            Some(_) => {}
            None => return false,
        }
    }
    false
}

#[test]
fn a_new_check_frequency_reschedules_the_checks() {
    let clock = ManualClock::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000));
    let mut start = SettingsStart::pane(SettingsPane::General);
    start.repositories = Vec::new();
    start.options = ContextOptions {
        worker_threads: 1,
        synchronizer: None,
        clock: Arc::clone(&clock) as _,
        ..ContextOptions::default()
    };
    let mut fixture = settings(start);
    fixture.viewer().start_background();
    assert!(wait_for(fixture.viewer(), TaskKind::PeriodicCheck, true));

    // From every hour to every 15 minutes: the next check is 15 minutes away.
    fixture.harness.get_by_label("Check repositories").click();
    fixture.harness.run();
    fixture.harness.get_by_label("Every 15 minutes").click();
    fixture.harness.run();
    assert_eq!(
        fixture.reload().preferences().check_repositories_frequency,
        CheckRepositoriesFrequency::Freq1
    );

    let viewer = fixture.viewer();
    std::thread::sleep(Duration::from_millis(100));
    viewer.handle().block_on(clock.wait_for_sleepers(1));
    clock.advance(Duration::from_secs(15 * 60));
    assert!(wait_for(viewer, TaskKind::PeriodicCheck, false));
}

#[test]
fn profiles_are_added_renamed_and_removed_with_undo() {
    let mut fixture = settings(SettingsStart::pane(SettingsPane::Profiles));
    let harness = &mut fixture.harness;

    // Add starts a rename of the new profile.
    harness.get_by_label_contains("Add").click();
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .type_text("Squad");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    let names = fixture.viewer().read().client.profile_names();
    assert_eq!(names, ["Default", "Squad", "Training"]);
    assert!(fixture.reload().profile("Squad").is_some());

    let harness = &mut fixture.harness;
    harness.get_by_label("Squad").click();
    harness.run();
    harness.get_by_label_contains("Remove").click();
    harness.run();
    assert!(fixture.reload().profile("Squad").is_none());

    let harness = &mut fixture.harness;
    harness.get_by_label("Undo").click();
    harness.run();
    assert!(fixture.viewer().read().client.profile("Squad").is_some());
    assert!(fixture.reload().profile("Squad").is_some());
}

#[test]
fn proxy_fields_are_written_when_valid() {
    let mut fixture = settings(SettingsStart::pane(SettingsPane::Network));
    let harness = &mut fixture.harness;
    harness.get_by_label("Proxy").click();
    harness.run();
    harness.get_by_label("SOCKS 5").click();
    harness.run();
    // No server yet: nothing is written.
    assert!(fixture.reload().proxy().proxy_protocol.is_none());

    let harness = &mut fixture.harness;
    harness.get_by_label("Server").focus();
    harness.get_by_label("Server").type_text("proxy.lan");
    harness.run();
    harness.get_by_label("Port").focus();
    harness.run();
    harness.get_by_label("Port").type_text("1080");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();

    let proxy = fixture.reload().proxy().clone();
    assert!(proxy.enable_proxy);
    let protocol = proxy.proxy_protocol.expect("proxy written");
    assert_eq!(protocol.protocol_type, ProtocolType::Socks5);
    assert_eq!(protocol.url, "proxy.lan");
    assert_eq!(protocol.port, "1080");
}

#[test]
fn the_command_preview_follows_the_options() {
    let mut fixture = settings(SettingsStart::pane(SettingsPane::Game));
    let harness = &mut fixture.harness;
    assert!(harness.query_by_label_contains("-window").is_none());

    harness.get_by_label("Windowed").click();
    harness.run();
    harness.get_by_label_contains("-window");
    let written = fixture.reload();
    let options = &written
        .profile("Default")
        .expect("profile")
        .launcher_options;
    assert!(options.window_mode);
}
