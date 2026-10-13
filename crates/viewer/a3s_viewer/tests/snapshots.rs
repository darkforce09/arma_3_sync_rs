// Its own test binary: wgpu rendering on a CPU adapter, Linux only, regenerated with UPDATE_SNAPSHOTS=1.
//! Headless wgpu snapshots of the main window with two repositories, one with updates; the
//! Servers screen with two favourites; the General and Game panes of the Settings window.
#![cfg(target_os = "linux")]

#[allow(
    dead_code,
    reason = "shared with the interaction tests, which use more of it"
)]
#[path = "viewer/support.rs"]
mod support;

#[allow(
    dead_code,
    reason = "shared with the interaction tests, which use more of it"
)]
#[path = "viewer/settings_support.rs"]
mod settings_support;

use std::convert::Infallible;

use a3s_types::FavoriteServer;
use a3s_ui::theme::{self, ThemeMode};
use a3s_viewer::{Screen, SettingsPane};
use egui_kittest::kittest::Queryable as _;

use crate::settings_support::{SettingsStart, settings_with};
use crate::support::{Start, fixture_with};

fn snapshot(name: &str, mode: ThemeMode) {
    let mut fixture = fixture_with(Start::default(), |builder| {
        builder.with_theme(mode.into()).wgpu()
    });
    theme::set_mode(&fixture.harness.ctx, mode);
    fixture.harness.run();
    fixture.harness.snapshot(name);
}

#[test]
fn shell_light() {
    snapshot("shell_light", ThemeMode::Light);
}

#[test]
fn shell_dark() {
    snapshot("shell_dark", ThemeMode::Dark);
}

fn servers_snapshot(name: &str, mode: ThemeMode) {
    let mut fixture = fixture_with(Start::default(), |builder| {
        builder.with_theme(mode.into()).wgpu()
    });
    fixture
        .harness
        .state()
        .viewer()
        .mutate(None, |state| {
            let client = &mut state.client;
            for (name, address, password, modset) in [
                ("Main", "10.0.0.1", Some("secret"), Some("Alpha")),
                ("Training", "10.0.0.2", None, None),
            ] {
                client
                    .add_favorite_server(FavoriteServer {
                        name: Some(name.to_owned()),
                        ip_address: Some(address.to_owned()),
                        port: 2302,
                        password: password.map(str::to_owned),
                        modset_name: modset.map(str::to_owned),
                        ..FavoriteServer::default()
                    })
                    .expect("server added");
            }
            client
                .update_configuration(|configuration| {
                    configuration.server_name = Some("Main".to_owned());
                    true
                })
                .expect("join set");
            Ok::<(), Infallible>(())
        })
        .ok();
    theme::set_mode(&fixture.harness.ctx, mode);
    fixture.harness.get_by_label("Servers").click();
    fixture.harness.run();
    assert_eq!(*fixture.harness.state().screen(), Screen::Servers);
    fixture.harness.snapshot(name);
}

#[test]
fn servers_light() {
    servers_snapshot("servers_light", ThemeMode::Light);
}

#[test]
fn servers_dark() {
    servers_snapshot("servers_dark", ThemeMode::Dark);
}

fn settings_snapshot(name: &str, pane: SettingsPane, mode: ThemeMode) {
    let mut fixture = settings_with(SettingsStart::pane(pane), |builder| {
        builder.with_theme(mode.into()).wgpu()
    });
    theme::set_mode(&fixture.harness.ctx, mode);
    fixture.harness.run();
    fixture.harness.snapshot(name);
}

#[test]
fn settings_general_light() {
    settings_snapshot(
        "settings_general_light",
        SettingsPane::General,
        ThemeMode::Light,
    );
}

#[test]
fn settings_general_dark() {
    settings_snapshot(
        "settings_general_dark",
        SettingsPane::General,
        ThemeMode::Dark,
    );
}

#[test]
fn settings_game_light() {
    settings_snapshot("settings_game_light", SettingsPane::Game, ThemeMode::Light);
}

#[test]
fn settings_game_dark() {
    settings_snapshot("settings_game_dark", SettingsPane::Game, ThemeMode::Dark);
}
