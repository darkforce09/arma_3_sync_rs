//! A shell over client data in a temporary folder, driven by `egui_kittest`.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_repo_files::services::ClientData;
use a3s_types::constants::{ModsetType, RepositoryStatus};
use a3s_types::local::{
    Configuration, Preferences, Profile, Repository, TreeDirectory, TreeLeaf, TreeNode,
};
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

    /// The profiles, the first one active; `None` gives `Default` and `Training`.
    pub profiles: Option<Vec<Profile>>,
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
    let profiles = start.profiles.unwrap_or_else(|| {
        vec![
            profile_with_group("Default", "Alpha"),
            Profile::new("Training"),
        ]
    });
    let configuration = Configuration {
        profile_name: profiles[0].name.clone(),
        ..Configuration::default()
    };
    let client = ClientData::from_parts(
        Arc::clone(&store),
        configuration,
        Preferences::default(),
        profiles,
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

fn repository(name: &str) -> Repository {
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
fn profile_with_group(name: &str, repository: &str) -> Profile {
    let mut profile = Profile::new(name);
    let mut group = TreeDirectory::new(repository);
    group.modset_type = Some(ModsetType::Repository);
    group.modset_repository_name = Some(repository.to_owned());
    profile.tree.add_child(TreeNode::Directory(group));
    profile
}

/// A mods folder in `dir` with `@ace`, `@cba` and `@gm` (whose `meta.cpp` names Workshop item
/// 3147580028), and an Arma 3 executable `arma3_x64` beside it. Returns both paths.
pub fn mods_folder(dir: &Path) -> (PathBuf, PathBuf) {
    let mods = dir.join("mods");
    for (name, size) in [("@ace", 3000), ("@cba", 2000), ("@gm", 1000)] {
        let addons = mods.join(name).join("addons");
        std::fs::create_dir_all(&addons).expect("addons folder");
        std::fs::write(addons.join("main.pbo"), vec![0_u8; size]).expect("pbo");
    }
    std::fs::write(
        mods.join("@gm").join("meta.cpp"),
        "protocol = 1;\npublishedid = 3147580028;\nname = \"GM\";\n",
    )
    .expect("meta.cpp");
    let game = dir.join("game");
    std::fs::create_dir_all(&game).expect("game folder");
    let exe = game.join("arma3_x64");
    std::fs::write(&exe, b"").expect("executable");
    (mods, exe)
}

/// The `Mods` profile: user group `Core` with `@ace` and `@cba` enabled, the `Alpha`
/// repository group with `@alpha_mod`, the mods folder, `@cba` loading before `@ace`.
pub fn mods_profile(mods: &Path, exe: &Path) -> Profile {
    let mut profile = Profile::new("Mods");
    let mut core = TreeDirectory::new("Core");
    core.selected = true;
    for name in ["@ace", "@cba"] {
        core.add_child(TreeNode::Leaf(TreeLeaf {
            selected: true,
            ..TreeLeaf::new(name)
        }));
    }
    profile.tree.add_child(TreeNode::Directory(core));
    let mut alpha = TreeDirectory::new("Alpha");
    alpha.modset_type = Some(ModsetType::Repository);
    alpha.modset_repository_name = Some("Alpha".to_owned());
    alpha.add_child(TreeNode::Leaf(TreeLeaf::new("@alpha_mod")));
    profile.tree.add_child(TreeNode::Directory(alpha));
    profile.addon_search_directories = vec![mods.display().to_string()];
    profile.addon_names_by_priority = vec!["@cba".to_owned(), "@ace".to_owned()];
    profile.launcher_options.arma3_exe_path = Some(exe.display().to_string());
    profile
}

/// Steps the harness until `done` holds, sleeping between steps for background work; panics
/// after five seconds.
pub fn step_until(harness: &mut Harness<'static, Shell>, done: impl Fn(&Shell) -> bool) {
    for _ in 0..500 {
        harness.step();
        if done(harness.state()) {
            harness.run();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("condition not reached");
}

/// Steps until the addon scan of the active profile finished.
pub fn wait_for_scan(harness: &mut Harness<'static, Shell>) {
    step_until(harness, |shell| {
        let state = shell.viewer().read();
        state.profiles.scan.is_some() && state.profiles.scan_requested.is_none()
    });
}
