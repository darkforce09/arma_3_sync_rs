//! A shell over client data in a temporary folder, driven by `egui_kittest`.

use std::convert::Infallible;
use std::sync::Arc;

use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_repo_files::services::ClientData;
use a3s_types::constants::{ModsetType, RepositoryStatus};
use a3s_types::local::{Configuration, Preferences, Profile, Repository, TreeDirectory, TreeNode};
use a3s_types::{Changelog, Changelogs, Protocol, ProtocolType};
use a3s_viewer::{RequestSender, Shell, ShellSetup, WelcomeSetup};
use a3s_viewer_context::{ContextOptions, ViewerContext};
use egui_kittest::Harness;
use tempfile::TempDir;

/// The window size of every test.
pub const SIZE: (f32, f32) = (1000.0, 640.0);

/// A shell, its request sender and the folder its data lives in.
pub struct Fixture {
    pub harness: Harness<'static, Shell>,
    pub requests: RequestSender,
    pub store: Arc<LocalStore>,
    _dir: TempDir,
}

/// What the shell starts with.
#[derive(Default)]
pub struct Start {
    pub load_warnings: Vec<String>,
    pub welcome: Option<WelcomeSetup>,
}

/// Repositories `Alpha` (in the Default profile's addon groups, with updates) and `Beta`
/// (built on this computer, up to date); profiles `Default` and `Training`.
pub fn fixture(start: Start) -> Fixture {
    fixture_with(start, |builder| builder)
}

/// [`fixture`] with a customized harness, e.g. a wgpu renderer for snapshots.
pub fn fixture_with(
    start: Start,
    customize: impl FnOnce(egui_kittest::HarnessBuilder<Shell>) -> egui_kittest::HarnessBuilder<Shell>,
) -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(LocalStore::with_dirs(LocalDirs {
        config: dir.path().join("config"),
        profiles: dir.path().join("config/profiles"),
        repositories: dir.path().join("data/repositories"),
        sha1_cache: dir.path().join("data/sha1_cache"),
    }));
    let mut beta = repository("Beta");
    beta.path = Some(dir.path().join("beta").display().to_string());
    let client = ClientData::from_parts(
        Arc::clone(&store),
        Configuration::default(),
        Preferences::default(),
        vec![
            profile_with_group("Default", "Alpha"),
            Profile::new("Training"),
        ],
        vec![repository("Alpha"), beta],
    );
    let viewer = ViewerContext::new(
        client,
        ContextOptions {
            worker_threads: 1,
            synchronizer: None,
            ..ContextOptions::default()
        },
    )
    .expect("context");
    set_status(&viewer, "Alpha", RepositoryStatus::Updated);
    set_status(&viewer, "Beta", RepositoryStatus::Ok);
    // The harness steps frames itself; the channel's repaint requests go to a spare context.
    let (requests, receiver) = a3s_viewer::channel(&egui::Context::default());
    let shell = Shell::new(
        viewer,
        ShellSetup {
            load_warnings: start.load_warnings,
            welcome: start.welcome,
            requests: receiver,
            hidden: false,
        },
    );
    let builder = Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(1.0)
        // Short steps keep multi-frame clicks within egui's click time limit.
        .with_step_dt(1.0 / 60.0)
        // At 60 steps per second, window fade-ins take more than the default 4 steps.
        .with_max_steps(60);
    let mut harness = customize(builder).build_ui_state(
        |ui, shell: &mut Shell| {
            shell.logic(ui.ctx());
            shell.show(ui);
        },
        shell,
    );
    a3s_ui::install(&harness.ctx);
    harness.run();
    Fixture {
        harness,
        requests,
        store,
        _dir: dir,
    }
}

/// Sets the runtime status of `name`; an updated repository gets a changelog with three
/// changed addons.
pub fn set_status(viewer: &ViewerContext, name: &str, status: RepositoryStatus) {
    viewer
        .mutate(None, |state| {
            let runtime = state.repositories.runtime_mut(name);
            runtime.status = status;
            runtime.metadata.changelogs = (status == RepositoryStatus::Updated).then(|| {
                Arc::new(Changelogs {
                    list: vec![Changelog {
                        revision: 1,
                        new_addons: vec!["@new".to_owned()],
                        updated_addons: vec!["@ace".to_owned(), "@cba".to_owned()],
                        ..Changelog::default()
                    }],
                })
            });
            Ok::<(), Infallible>(())
        })
        .ok();
}

pub fn repository(name: &str) -> Repository {
    Repository::new(
        name,
        Protocol::new(
            ProtocolType::Http,
            "127.0.0.1/repo",
            "80",
            "anonymous",
            "",
            true,
        ),
    )
}

/// A profile whose tree holds the addon group made from `repository`.
pub fn profile_with_group(name: &str, repository: &str) -> Profile {
    let mut profile = Profile::new(name);
    let mut group = TreeDirectory::new(repository);
    group.modset_type = Some(ModsetType::Repository);
    group.modset_repository_name = Some(repository.to_owned());
    profile.tree.add_child(TreeNode::Directory(group));
    profile
}
