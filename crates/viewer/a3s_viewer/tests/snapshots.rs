// Its own test binary: wgpu rendering on a CPU adapter, Linux only, regenerated with UPDATE_SNAPSHOTS=1.
//! Headless wgpu snapshots of the main window with two repositories, one with updates, and of
//! the Mods screen's tree and load order.
#![cfg(target_os = "linux")]

#[allow(
    dead_code,
    reason = "shared with the interaction tests, which use more of it"
)]
#[path = "viewer/support.rs"]
mod support;

use a3s_ui::theme::{self, ThemeMode};
use egui_kittest::kittest::Queryable as _;

use crate::support::{Start, fixture_with, mods_folder, mods_profile, wait_for_scan};

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

fn mods_snapshot(name: &str, mode: ThemeMode, load_order: bool) {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mods, exe) = mods_folder(dir.path());
    let mut fixture = fixture_with(
        Start {
            profiles: Some(vec![mods_profile(&mods, &exe)]),
            ..Start::default()
        },
        |builder| builder.with_theme(mode.into()).wgpu(),
    );
    theme::set_mode(&fixture.harness.ctx, mode);
    wait_for_scan(&mut fixture.harness);
    if load_order {
        fixture.harness.get_by_label("Load order").click();
    }
    fixture.harness.run();
    fixture.harness.remove_cursor();
    fixture.harness.run();
    fixture.harness.snapshot(name);
}

#[test]
fn mods_light() {
    mods_snapshot("mods_light", ThemeMode::Light, false);
}

#[test]
fn mods_dark() {
    mods_snapshot("mods_dark", ThemeMode::Dark, false);
}

#[test]
fn load_order_light() {
    mods_snapshot("load_order_light", ThemeMode::Light, true);
}

#[test]
fn load_order_dark() {
    mods_snapshot("load_order_dark", ThemeMode::Dark, true);
}
