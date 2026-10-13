//! The Settings window alone over client data in a temporary folder, driven by `egui_kittest`.

use std::sync::Arc;

use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_repo_files::services::{ClientData, LoadOutcome};
use a3s_types::local::{Configuration, Preferences, Profile, Repository};
use a3s_viewer::{Desktop, SettingsPane, SettingsWindow};
use a3s_viewer_context::{ContextOptions, ViewerContext};
use egui_kittest::{Harness, HarnessBuilder};
use tempfile::TempDir;

use crate::support::{profile_with_group, repository};

/// The window size of every Settings test.
pub const SETTINGS_SIZE: (f32, f32) = (820.0, 720.0);

/// The window and the context it edits.
pub struct SettingsApp {
    pub window: SettingsWindow,
    pub viewer: ViewerContext,
}

/// A Settings window and the folder its data and its desktop live in.
pub struct SettingsFixture {
    pub harness: Harness<'static, SettingsApp>,
    pub store: Arc<LocalStore>,
    _dir: TempDir,
}

impl SettingsFixture {
    /// The client data as written to disk.
    pub fn reload(&self) -> ClientData {
        let LoadOutcome { data, .. } = ClientData::load(Arc::clone(&self.store));
        data
    }

    /// The context the window edits.
    pub fn viewer(&self) -> &ViewerContext {
        &self.harness.state().viewer
    }
}

/// What the fixture starts with.
pub struct SettingsStart {
    pub pane: SettingsPane,
    pub repositories: Vec<Repository>,
    pub options: ContextOptions,
}

impl SettingsStart {
    /// `pane` over profiles `Default` (with the `Alpha` group) and `Training` and repositories
    /// `Alpha` and `Beta`.
    pub fn pane(pane: SettingsPane) -> Self {
        Self {
            pane,
            repositories: vec![repository("Alpha"), repository("Beta")],
            options: ContextOptions {
                worker_threads: 1,
                synchronizer: None,
                ..ContextOptions::default()
            },
        }
    }
}

/// The Settings window showing `start.pane`.
pub fn settings(start: SettingsStart) -> SettingsFixture {
    settings_with(start, |builder| builder)
}

/// [`settings`] with a customized harness, e.g. a wgpu renderer for snapshots.
pub fn settings_with(
    start: SettingsStart,
    customize: impl FnOnce(HarnessBuilder<SettingsApp>) -> HarnessBuilder<SettingsApp>,
) -> SettingsFixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(LocalStore::with_dirs(LocalDirs {
        config: dir.path().join("config"),
        profiles: dir.path().join("config/profiles"),
        repositories: dir.path().join("data/repositories"),
        sha1_cache: dir.path().join("data/sha1_cache"),
    }));
    let client = ClientData::from_parts(
        Arc::clone(&store),
        Configuration::default(),
        Preferences::default(),
        vec![
            profile_with_group("Default", "Alpha"),
            Profile::new("Training"),
        ],
        start.repositories,
    );
    let viewer = ViewerContext::new(client, start.options).expect("context");
    let mut window = SettingsWindow::new(Desktop::isolated(&dir.path().join("home")));
    window.set_pane(start.pane);
    let builder = Harness::builder()
        .with_size(SETTINGS_SIZE)
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60);
    let mut harness = customize(builder).build_ui_state(
        |ui, app: &mut SettingsApp| app.window.show(ui, &app.viewer),
        SettingsApp { window, viewer },
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    SettingsFixture {
        harness,
        store,
        _dir: dir,
    }
}
