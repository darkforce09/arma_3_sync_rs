//! A shell over client data in a temporary folder, driven by `egui_kittest`.

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_platform::process::FixedProcessProbe;
use a3s_platform::registry::MemoryRegistry;
use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_repo_files::services::ClientData;
use a3s_types::constants::{ModsetType, RepositoryStatus};
use a3s_types::local::{
    Configuration, Preferences, Profile, Repository, TreeDirectory, TreeLeaf, TreeNode,
};
use a3s_types::{Changelog, Changelogs, Protocol, ProtocolType};
use a3s_viewer::{RequestSender, SecondaryWindow, Shell, ShellSetup, WelcomeSetup};
use a3s_viewer_context::tools::Platform;
use a3s_viewer_context::{AppState, ContextOptions, ViewerContext};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
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

    /// Repositories stored besides `Alpha` and `Beta`.
    pub repositories: Vec<Repository>,
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
        [repository("Alpha"), beta]
            .into_iter()
            .chain(start.repositories)
            .collect(),
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

/// Steps frames until `done` holds; panics after ten seconds.
pub fn wait_for(
    harness: &mut Harness<'static, Shell>,
    what: &str,
    done: impl Fn(&Harness<'static, Shell>) -> bool,
) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits for a node labelled exactly `label`.
pub fn wait_for_label(harness: &mut Harness<'static, Shell>, label: &str) {
    wait_for(harness, label, |h| h.query_by_label(label).is_some());
}

pub fn mutate(viewer: &ViewerContext, f: impl FnOnce(&mut AppState)) {
    viewer
        .mutate(None, |state| {
            f(state);
            Ok::<(), Infallible>(())
        })
        .ok();
}

/// A platform without registry or environment; `running` are the running processes.
pub fn offline_platform(running: &[&str]) -> Platform {
    Platform {
        env: None,
        registry: Arc::new(MemoryRegistry::default()),
        processes: Arc::new(FixedProcessProbe(Some(
            running.iter().map(|name| (*name).to_owned()).collect(),
        ))),
    }
}

pub fn open(harness: &mut Harness<'static, Shell>, window: SecondaryWindow) {
    let ctx = harness.ctx.clone();
    harness.state().open(&ctx, window);
    harness.step();
}

pub fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    fs::write(path, bytes).expect("file");
}

/// A mod folder `name` under `mods` with one pbo and the given keys.
pub fn add_mod(mods: &Path, name: &str, keys: &[&str]) {
    write(&mods.join(name).join("addons/main.pbo"), b"pbo");
    for key in keys {
        write(&mods.join(name).join("keys").join(key), key.as_bytes());
    }
}

/// Makes `mods` the active profile's mod folder and enables `enabled`.
pub fn enable_mods(viewer: &ViewerContext, mods: &Path, enabled: &[&str]) {
    let folder = mods.display().to_string();
    let enabled: Vec<String> = enabled.iter().map(|name| (*name).to_owned()).collect();
    viewer
        .mutate(None, |state| {
            state.client.update_active_profile(|profile| {
                profile.addon_search_directories = vec![folder];
                for name in enabled {
                    let mut leaf = TreeLeaf::new(name);
                    leaf.selected = true;
                    profile.tree.add_child(TreeNode::Leaf(leaf));
                }
                true
            })
        })
        .expect("profile");
}

/// A TFAR mod, an Arma 3 folder and a `TeamSpeak` 3 folder; returns the `TeamSpeak` folder.
pub fn radio_folders(viewer: &ViewerContext, root: &Path, running: &[&str]) -> PathBuf {
    let mods = root.join("mods");
    add_mod(&mods, "@task_force_radio", &[]);
    let tfar = mods.join("@task_force_radio");
    write(
        &tfar.join("plugins/task_force_radio_win64.dll"),
        b"tfar plugin",
    );
    write(&tfar.join("plugins/TFAR/sounds.txt"), b"sounds");
    write(
        &tfar.join("userconfig/task_force_radio/radio_settings.hpp"),
        b"settings",
    );
    let arma3 = root.join("arma3");
    write(&arma3.join("arma3_x64.exe"), b"exe");
    let ts3 = root.join("ts3");
    write(&ts3.join("ts3client_win64.exe"), b"exe");
    write(
        &ts3.join("changelog.txt"),
        b"=== Client Release 3.6.2 ===\n",
    );
    fs::create_dir_all(ts3.join("plugins")).expect("plugins");

    enable_mods(viewer, &mods, &["@task_force_radio"]);
    let exe = arma3.join("arma3_x64.exe").display().to_string();
    let ts3_path = ts3.display().to_string();
    let platform = offline_platform(running);
    viewer
        .mutate(None, |state| {
            state.tools.platform = platform;
            state
                .client
                .update_launcher_options(|options| options.arma3_exe_path = Some(exe))?;
            state.client.set_tfar_ts3_path(Some(ts3_path))
        })
        .expect("settings");
    ts3
}
