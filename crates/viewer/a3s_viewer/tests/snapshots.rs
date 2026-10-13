// Its own test binary: wgpu rendering on a CPU adapter, Linux only, regenerated with UPDATE_SNAPSHOTS=1.
//! Headless wgpu snapshots of the main window with two repositories, one with updates.
#![cfg(target_os = "linux")]

#[allow(
    dead_code,
    reason = "shared with the interaction tests, which use more of it"
)]
#[path = "viewer/support.rs"]
mod support;

use a3s_ui::theme::{self, ThemeMode};

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
