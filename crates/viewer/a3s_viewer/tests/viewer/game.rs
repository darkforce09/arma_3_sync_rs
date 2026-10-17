//! The Game screen: every launch option of the active profile on one page, with the command line
//! they give.

use a3s_viewer::Screen;
use egui_kittest::kittest::Queryable as _;

use crate::support::{Start, fixture};

#[test]
fn the_command_preview_follows_the_options() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness.get_by_label("Game").click();
    harness.run();
    assert_eq!(*harness.state().screen(), Screen::Game);
    assert!(harness.query_by_label_contains("-window").is_none());

    harness.get_by_label("Windowed").click();
    harness.run();
    harness.get_by_label_contains("-window");
    let state = harness.state().viewer().read();
    let profile = state.client.active_profile().expect("active profile");
    assert!(profile.launcher_options.window_mode);
}
