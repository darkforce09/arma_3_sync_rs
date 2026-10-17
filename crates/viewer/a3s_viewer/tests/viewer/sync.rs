//! The player sync loop: adding a repository from its auto-config, Update (of every or of the
//! ticked mods, and before playing), the Activity view, the report and Update issues sheets, and
//! the question while Arma 3 runs.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use a3s_foundation::error::ErrorKind;
use a3s_sync::{EventError, SyncEvent, TransferProgress};
use a3s_test_servers::Fixture as Server;
use a3s_types::AutoConfig;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Profile, Repository, TreeLeaf, TreeNode};
use a3s_viewer::{Screen, SheetKind, Shell};
use a3s_viewer_context::sync::{self, DownloadRecord, DownloadResult, NodeStatus};
use a3s_viewer_context::{Command, TaskError, TaskKey, TaskKind, repositories};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use crate::support::{
    RecordingLauncher, Start, fixture, mutate, offline_platform, profile_with_group, set_status,
};

/// How long a test waits for background work.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Steps the window until `done` holds; panics after [`TIMEOUT`].
fn wait_until(harness: &mut Harness<'static, Shell>, what: &str, done: impl Fn(&Shell) -> bool) {
    let start = Instant::now();
    while !done(harness.state()) {
        assert!(
            start.elapsed() < TIMEOUT,
            "timed out waiting for {what}; sheet {:?}, tasks {:?}",
            harness.state().sheet(),
            harness.state().viewer().running_tasks()
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.step();
}

/// Checks `name`, as "Check now" does; the Repository screen then compares it with its folder.
fn check(harness: &Harness<'static, Shell>, name: &str) {
    let command = Command::Repositories(repositories::Command::Check {
        names: vec![name.to_owned()],
    });
    harness.state().viewer().send(command).expect("check");
}

/// A runtime for the test servers, kept alive for the whole test.
fn server_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("test-server")
        .enable_all()
        .build()
        .expect("runtime")
}

/// A local FTP server serving the Java-built sample repository.
fn ftp_repository(runtime: &tokio::runtime::Runtime) -> Server {
    let server = runtime.block_on(a3s_test_servers::ftp::fixture(false));
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_built/ftp_zip/1_initial/repo");
    copy_tree(&sample, &server.base);
    server
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

#[test]
fn add_repository_fills_from_the_autoconfig_and_creates_it() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let autoconfig = AutoConfig {
        repository_name: Some("Local group".to_owned()),
        protocol: Some(server.protocol.clone()),
        favorite_servers: Vec::new(),
    };
    a3s_repo_files::write_a3s(&server.base.join(".a3s/autoconfig"), &autoconfig)
        .expect("write autoconfig");
    let port = server.protocol.port.clone();

    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness.get_by_label("Add repository").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::AddRepository));

    harness.get_by_label("Auto-config link").focus();
    harness.run();
    harness
        .get_by_label("Auto-config link")
        .type_text(&format!("ftp://127.0.0.1:{port}/.a3s/autoconfig"));
    let start = Instant::now();
    while harness.get_by_label("Name").value().as_deref() != Some("Local group") {
        assert!(start.elapsed() < TIMEOUT, "the fields were not filled");
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    // The sheet is centred again once its size settled.
    harness.run();
    assert_eq!(
        harness.get_by_label("Address").value().as_deref(),
        Some("127.0.0.1")
    );
    assert_eq!(harness.get_by_label("Port").value(), Some(port.clone()));

    harness.get_by_label("Add").click();
    harness.run_ok();
    let shell = harness.state();
    assert_eq!(shell.sheet(), None);
    assert_eq!(
        *shell.screen(),
        Screen::Repository("Local group".to_owned())
    );
    let state = shell.viewer().read();
    let stored = state.client.repository("Local group").expect("created");
    assert_eq!(stored.protocol.url, "127.0.0.1");
    assert_eq!(stored.protocol.port, port);
    assert_eq!(stored.protocol.protocol_type, server.protocol.protocol_type);
}

#[test]
fn update_downloads_the_repository() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let destination = tempfile::tempdir().expect("destination");
    let mut repository = Repository::new("Local", server.protocol.clone());
    repository.default_download_location = Some(destination.path().display().to_string());
    let mut profile = Profile::new("Default");
    profile.addon_search_directories = vec![destination.path().display().to_string()];

    let mut fixture = fixture(Start {
        repositories: vec![repository],
        profiles: Some(vec![profile]),
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    harness.get_by_label("Local").click();
    harness.run_ok();
    harness.get_by_label("Update").click();
    harness.step();

    wait_until(harness, "the download", |shell| {
        !shell.viewer().read().sync.downloads.recent.is_empty()
    });
    let result = harness.state().viewer().read().sync.downloads.recent[0]
        .result
        .clone();
    assert_eq!(
        result,
        DownloadResult::Success {
            trashed: 0,
            userconfig: None
        }
    );
    assert!(destination.path().join("@Mod_A/mod.cpp").is_file());
    assert!(
        destination
            .path()
            .join("Collection/@Nested/addons/nested.pbo")
            .is_file()
    );
    // The second comparison after the download leaves nothing to do.
    wait_until(harness, "the up-to-date state", |shell| {
        shell
            .viewer()
            .is_running(&TaskKey::repository(TaskKind::Download, "Local"))
            .is_none()
    });
    harness.get_by_label_contains("Up to date");
    // The mods folder is scanned again, so the new mods count as installed.
    wait_until(harness, "the scan of the new mods", |shell| {
        shell
            .viewer()
            .read()
            .profiles
            .scan
            .as_ref()
            .is_some_and(|scan| {
                scan.registry
                    .keys()
                    .any(|key| key.eq_ignore_ascii_case("@Mod_A"))
            })
    });
}

#[test]
fn activity_shows_the_progress_of_a_running_download() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    let id = harness
        .state()
        .viewer()
        .cx()
        .spawn_task(
            TaskKey::repository(TaskKind::Download, "Alpha"),
            true,
            |task| async move {
                let events = task.sync_events();
                events.emit(SyncEvent::Transfer(TransferProgress {
                    transferred: 1_500_000,
                    total: 3_000_000,
                    current_file: Some("@ace/addons/ace_main.pbo".to_owned()),
                    current_file_percent: 40,
                    remaining: Some(Duration::from_secs(90)),
                }));
                events.emit(SyncEvent::Speed {
                    bytes_per_second: 1_000_000,
                });
                task.cancel.cancelled().await;
                Err(TaskError::Cancelled)
            },
        )
        .expect("task");
    wait_until(harness, "the progress", |shell| {
        shell
            .viewer()
            .task_snapshot(id)
            .is_some_and(|snapshot| snapshot.transfer.is_some() && snapshot.speed.is_some())
    });

    harness.get_by_label("Activity").click();
    harness.run();
    harness.get_by_label("50%");
    harness.get_by_label_contains("1.5 MB of 3.0 MB");
    harness.get_by_label("@ace/addons/ace_main.pbo");

    harness.get_by_label_contains("Pause").click();
    wait_until(harness, "the pause", |shell| {
        shell.viewer().running_tasks().is_empty()
    });
}

#[test]
fn issues_sheet_lists_the_failed_files_and_retry_downloads_again() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.sync.downloads.recent.push(DownloadRecord {
                repository: "Alpha".to_owned(),
                event: None,
                finished: SystemTime::now(),
                result: DownloadResult::Issues(vec![
                    EventError::new(ErrorKind::NotFound, "Remote file not found")
                        .with_path("@ace/addons/ace_main.pbo"),
                    EventError::new(ErrorKind::ReadTimeout, "Read timed out")
                        .with_path("@cba/addons/cba_main.pbo"),
                ]),
                report: Some("--- Download report ---".to_owned()),
            });
            Ok::<(), Infallible>(())
        })
        .ok();

    // The Mods screen also shows an "Alpha" group: leave it first.
    harness.get_by_label("Servers").click();
    harness.run();
    harness.get_by_label("Alpha").click();
    harness.run();
    harness.get_by_label("Show issues").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::UpdateIssues));
    harness.get_by_label("@ace/addons/ace_main.pbo");
    harness.get_by_label("@cba/addons/cba_main.pbo");

    harness
        .get_all_by_label("Retry")
        .next()
        .expect("a Retry button")
        .click();
    harness.step();
    // The retry is a new download of Alpha; its server is not running, so it fails.
    wait_until(harness, "the retried download", |shell| {
        shell.viewer().read().sync.downloads.recent.len() == 2
    });
    let state = harness.state().viewer().read();
    let latest = &state.sync.downloads.recent[0];
    assert_eq!(latest.repository, "Alpha");
    assert!(matches!(latest.result, DownloadResult::Failed(_)));
}

#[test]
fn unticked_mods_are_neither_downloaded_nor_removed() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let destination = tempfile::tempdir().expect("destination");
    let old = destination.path().join("@old/addons/old.pbo");
    std::fs::create_dir_all(old.parent().expect("parent")).expect("mkdir");
    std::fs::write(&old, b"old").expect("old mod");
    let mut repository = Repository::new("Local", server.protocol.clone());
    repository.default_download_location = Some(destination.path().display().to_string());
    repository.exact_match = true;

    let mut fixture = fixture(Start {
        repositories: vec![repository],
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    check(harness, "Local");
    harness.get_by_label("Local").click();
    harness.run_ok();
    let start = Instant::now();
    while harness
        .query_by_role_and_label(Role::CheckBox, "@old")
        .is_none()
    {
        assert!(start.elapsed() < TIMEOUT, "the comparison did not finish");
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    harness
        .get_by_role_and_label(Role::CheckBox, "@Mod_A")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::CheckBox, "@old")
        .click();
    harness.run();
    harness.get_by_label_contains("Update ").click();
    harness.step();

    wait_until(harness, "the download", |shell| {
        !shell.viewer().read().sync.downloads.recent.is_empty()
    });
    let result = harness.state().viewer().read().sync.downloads.recent[0]
        .result
        .clone();
    assert!(
        matches!(result, DownloadResult::Success { .. }),
        "{result:?}"
    );
    assert!(destination.path().join("@mod_b").is_dir());
    assert!(
        !destination.path().join("@Mod_A").exists(),
        "an unticked mod is not downloaded"
    );
    assert!(old.is_file(), "an unticked removed mod stays");
}

#[test]
fn update_and_play_downloads_only_the_enabled_mods() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let destination = tempfile::tempdir().expect("destination");
    let game = tempfile::tempdir().expect("game folder");
    let exe = game.path().join("arma3_x64");
    std::fs::write(&exe, b"").expect("executable");
    let mut repository = Repository::new("Local", server.protocol.clone());
    repository.default_download_location = Some(destination.path().display().to_string());
    let mut profile = profile_with_group("Default", "Local");
    profile.launcher_options.arma3_exe_path = Some(exe.display().to_string());
    profile.addon_search_directories = vec![destination.path().display().to_string()];
    if let Some(TreeNode::Directory(group)) = profile.tree.children.first_mut() {
        group.add_child(TreeNode::Leaf(TreeLeaf {
            selected: true,
            ..TreeLeaf::new("@mod_b")
        }));
        group.add_child(TreeNode::Leaf(TreeLeaf::new("@Mod_A")));
    }

    let mut fixture = fixture(Start {
        repositories: vec![repository],
        profiles: Some(vec![profile]),
        ..Start::default()
    });
    let harness = &mut fixture.harness;
    let launcher = Arc::new(RecordingLauncher::default());
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.launch.launcher = launcher.clone();
            Ok::<(), Infallible>(())
        })
        .ok();
    harness
        .get_all_by_label("Local")
        .next()
        .expect("sidebar row")
        .click();
    harness.run_ok();
    // The check loads the repository's content.
    check(harness, "Local");
    wait_until(harness, "the comparison", |shell| {
        shell
            .viewer()
            .read()
            .repositories
            .runtime("Local")
            .metadata
            .sync
            .is_some()
    });
    set_status(harness.state().viewer(), "Local", RepositoryStatus::Updated);
    harness.step();
    harness.get_by_label_contains("Update and play").click();
    harness.step();

    wait_until(harness, "the download", |shell| {
        !shell.viewer().read().sync.downloads.recent.is_empty()
    });
    assert!(destination.path().join("@mod_b").is_dir());
    assert!(
        !destination.path().join("@Mod_A").exists(),
        "a mod the profile does not enable is not downloaded"
    );
    // The game starts with the mod it just got: the mods folder was scanned again.
    wait_until(harness, "the launch", |_| launcher.plan.lock().is_some());
    let plan = launcher.plan.lock().clone().expect("plan");
    assert!(
        plan.command
            .args
            .iter()
            .any(|arg| arg.to_string_lossy().contains("@mod_b")),
        "{:?}",
        plan.command.args
    );
}

#[test]
fn an_update_asks_first_while_arma_3_runs() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.tools.platform = offline_platform(&["arma3_x64.exe"]);
            Ok::<(), Infallible>(())
        })
        .ok();
    harness
        .get_all_by_label("Alpha")
        .next()
        .expect("sidebar row")
        .click();
    harness.run();
    harness.get_by_label("Update").click();
    wait_until(harness, "the question", |shell| {
        shell.sheet() == Some(SheetKind::GameRunning)
    });
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().sheet(), None);
    assert!(
        harness
            .state()
            .viewer()
            .is_running(&TaskKey::repository(TaskKind::Download, "Alpha"))
            .is_none()
    );
}

#[test]
fn a_finished_download_shows_its_report() {
    let mut fixture = fixture(Start::default());
    let harness = &mut fixture.harness;
    harness
        .state()
        .viewer()
        .mutate(None, |state| {
            state.sync.downloads.recent.push(DownloadRecord {
                repository: "Beta".to_owned(),
                event: None,
                finished: SystemTime::now(),
                result: DownloadResult::Success {
                    trashed: 0,
                    userconfig: None,
                },
                report: Some("--- Download report ---".to_owned()),
            });
            Ok::<(), Infallible>(())
        })
        .ok();
    harness.get_by_label("Activity").click();
    harness.run();
    harness.get_by_label("Report").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::DownloadReport));
    harness.get_by_label_contains("--- Download report ---");
}

/// The Java-built sample repository at `stage` (`1_initial`, `3_modified`).
fn sample(stage: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_built/ftp_zip")
        .join(stage)
        .join("repo")
}

/// The status of `path` in the current comparison of `repository`.
fn status_of(shell: &Shell, repository: &str, path: &str) -> Option<NodeStatus> {
    let state = shell.viewer().read();
    let plan = state
        .sync
        .downloads
        .plans
        .get(repository)?
        .result
        .as_ref()
        .ok()?;
    plan.tree.get(path).map(|node| node.status)
}

/// Opens `name` and downloads all of it into its folder, until the comparison after the
/// download says it is up to date.
fn download_all(harness: &mut Harness<'static, Shell>, name: &str) {
    harness.get_by_label(name).click();
    harness.run_ok();
    harness.get_by_label("Update").click();
    harness.step();
    wait_until(harness, "the download", |shell| {
        !shell.viewer().read().sync.downloads.recent.is_empty()
    });
    wait_until(harness, "the comparison after the download", |shell| {
        status_of(shell, name, "@Mod_A") == Some(NodeStatus::UpToDate)
    });
}

/// A repository downloading into `folder`, with `folder` as the profile's mods folder.
fn local_repository(server: &Server, folder: &Path) -> Start {
    let mut repository = Repository::new("Local", server.protocol.clone());
    repository.default_download_location = Some(folder.display().to_string());
    let mut profile = Profile::new("Default");
    profile.addon_search_directories = vec![folder.display().to_string()];
    Start {
        repositories: vec![repository],
        profiles: Some(vec![profile]),
        ..Start::default()
    }
}

#[test]
fn an_installed_mod_with_changed_files_is_listed_with_them() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let destination = tempfile::tempdir().expect("destination");
    let mut fixture = fixture(local_repository(&server, destination.path()));
    let harness = &mut fixture.harness;
    download_all(harness, "Local");

    copy_tree(&sample("3_modified"), &server.base);
    check(harness, "Local");
    wait_until(harness, "the comparison with the new revision", |shell| {
        status_of(shell, "Local", "@Mod_A") == Some(NodeStatus::Update)
    });
    let shell = harness.state();
    assert_eq!(
        status_of(shell, "Local", "@Mod_A/addons/mod_a.pbo"),
        Some(NodeStatus::Update)
    );
    assert_eq!(
        status_of(shell, "Local", "@Mod_A/mod.cpp"),
        Some(NodeStatus::UpToDate)
    );
    assert_eq!(
        status_of(shell, "Local", "@mod_b"),
        Some(NodeStatus::Update)
    );
    assert_eq!(
        status_of(shell, "Local", "@Mod_D"),
        Some(NodeStatus::NotDownloaded)
    );
    harness.get_by_role_and_label(Role::CheckBox, "@Mod_A");
}

#[test]
fn check_now_finds_a_file_changed_on_disk_at_the_same_revision() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let destination = tempfile::tempdir().expect("destination");
    let mut fixture = fixture(local_repository(&server, destination.path()));
    let harness = &mut fixture.harness;
    download_all(harness, "Local");

    std::fs::write(
        destination.path().join("@Mod_A/mod.cpp"),
        b"changed by hand",
    )
    .expect("edit a downloaded file");
    check(harness, "Local");
    wait_until(harness, "the comparison after the check", |shell| {
        status_of(shell, "Local", "@Mod_A/mod.cpp") == Some(NodeStatus::Update)
    });
    assert_eq!(
        status_of(harness.state(), "Local", "@Mod_A"),
        Some(NodeStatus::Update)
    );
}

#[test]
fn auto_discover_counts_mods_found_in_another_mods_folder() {
    let runtime = server_runtime();
    let server = ftp_repository(&runtime);
    let first = tempfile::tempdir().expect("first folder");
    let empty = tempfile::tempdir().expect("empty folder");
    let mut fixture = fixture(local_repository(&server, first.path()));
    let harness = &mut fixture.harness;
    download_all(harness, "Local");

    // The download folder changes to an empty one; the mods stay in the profile's folder.
    let folder = empty.path().display().to_string();
    mutate(harness.state().viewer(), move |state| {
        state
            .client
            .update_repository("Local", |stored| {
                stored.default_download_location = Some(folder);
                stored.no_auto_discover = false;
                Ok(true)
            })
            .expect("store");
        sync::forget(state, "Local");
    });
    wait_until(harness, "the comparison with auto-discover", |shell| {
        let state = shell.viewer().read();
        state
            .sync
            .downloads
            .plans
            .get("Local")
            .is_some_and(|plan| plan.addon_scan)
    });
    assert_eq!(
        status_of(harness.state(), "Local", "@Mod_A"),
        Some(NodeStatus::UpToDate)
    );
    assert_eq!(
        status_of(harness.state(), "Local", "@mod_b"),
        Some(NodeStatus::UpToDate)
    );
}
