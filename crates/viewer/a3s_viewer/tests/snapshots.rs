// Its own test binary: wgpu rendering on a CPU adapter, Linux only, regenerated with UPDATE_SNAPSHOTS=1.
//! Headless wgpu snapshots of the main window with two repositories, one with updates; the
//! Repository, Activity and Publishing screens; the Mods screen's tree and load order; the Servers
//! screen with two favourites; the General and Game panes of the Settings window; the tool
//! windows.
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

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use a3s_admin::{EventSelection, EventsDoc};
use a3s_foundation::error::ErrorKind;
use a3s_sync::{EventError, SyncEvent, TransferProgress};
use a3s_types::{Changelog, Changelogs, Event, Events, FavoriteServer, ServerInfo};
use a3s_ui::theme::{self, ThemeMode};
use a3s_viewer::{Screen, SecondaryWindow, SettingsPane};
use a3s_viewer_context::admin::{
    self, LocalScan, ModChanges, PublishRun, Remote, RemoteRepository, StepDetail, StepState,
    StepStatus,
};
use a3s_viewer_context::sync::{
    DownloadRecord, DownloadResult, NodeStatus, PlanState, StatusNode, StatusTree, SyncPlan,
};
use a3s_viewer_context::{TaskError, TaskKey, TaskKind};
use egui_kittest::kittest::Queryable as _;

use crate::settings_support::{SettingsStart, settings_with};
use crate::support::{
    Fixture, Start, add_mod, enable_mods, fixture_with, mods_folder, mods_profile, mutate, open,
    radio_folders, wait_for_label, wait_for_scan, write,
};

fn snapshot(name: &str, mode: ThemeMode, setup: impl FnOnce(&mut Fixture)) {
    let mut fixture = fixture_with(Start::default(), |builder| {
        builder.with_theme(mode.into()).wgpu()
    });
    theme::set_mode(&fixture.harness.ctx, mode);
    fixture.harness.run();
    setup(&mut fixture);
    fixture.harness.run();
    fixture.harness.snapshot(name);
}

#[test]
fn shell_light() {
    snapshot("shell_light", ThemeMode::Light, |_| {});
}

#[test]
fn shell_dark() {
    snapshot("shell_dark", ThemeMode::Dark, |_| {});
}

#[test]
fn repository_light() {
    snapshot("repository_light", ThemeMode::Light, show_repository);
}

#[test]
fn repository_dark() {
    snapshot("repository_dark", ThemeMode::Dark, show_repository);
}

#[test]
fn activity_light() {
    snapshot("activity_light", ThemeMode::Light, show_activity);
}

#[test]
fn activity_dark() {
    snapshot("activity_dark", ThemeMode::Dark, show_activity);
}

/// Alpha at server revision 2 with an event, two changelogs and a comparison.
fn show_repository(fixture: &mut Fixture) {
    let viewer = fixture.harness.state().viewer();
    mutate(viewer, |state| {
        let runtime = state.repositories.runtime_mut("Alpha");
        runtime.metadata.server_info = Some(Arc::new(ServerInfo {
            revision: 2,
            ..ServerInfo::default()
        }));
        runtime.metadata.changelogs = Some(Arc::new(Changelogs {
            list: vec![
                Changelog {
                    revision: 1,
                    build_date: Some(1_704_067_200_000),
                    new_addons: vec!["@new".to_owned()],
                    updated_addons: vec!["@cba".to_owned()],
                    ..Changelog::default()
                },
                Changelog {
                    revision: 2,
                    build_date: Some(1_704_153_600_000),
                    updated_addons: vec!["@ace".to_owned()],
                    deleted_addons: vec!["@old".to_owned()],
                    ..Changelog::default()
                },
            ],
        }));
        runtime.metadata.events = Some(Arc::new(Events {
            list: vec![Event {
                name: "Sunday operation".to_owned(),
                description: Some("Weekly co-op with the full mod set.".to_owned()),
                addon_names: BTreeMap::from([
                    ("@ace".to_owned(), false),
                    ("@cba".to_owned(), false),
                    ("@new".to_owned(), true),
                ]),
                ..Event::default()
            }],
        }));
        state.sync.downloads.plans.insert(
            "Alpha".to_owned(),
            PlanState {
                server_revision: Some(2),
                addon_scan: false,
                result: Ok(SyncPlan {
                    download_size: 1_320_000_000,
                    file_count: 14,
                    delete_count: 1,
                    tree: Arc::new(StatusTree {
                        roots: vec![
                            node("@ace", NodeStatus::Update, 1_200_000_000, 12),
                            node("@cba", NodeStatus::UpToDate, 0, 0),
                            node("@new", NodeStatus::NotDownloaded, 120_000_000, 2),
                            node("@old", NodeStatus::Removed, 0, 0),
                        ],
                    }),
                }),
            },
        );
    });
    // The Mods screen also shows an "Alpha" group: leave it first.
    fixture.harness.get_by_label("Servers").click();
    fixture.harness.run();
    fixture.harness.get_by_label("Alpha").click();
}

/// A running download of Alpha and three recent results.
fn show_activity(fixture: &mut Fixture) {
    let viewer = fixture.harness.state().viewer();
    let id = viewer
        .cx()
        .spawn_task(
            TaskKey::repository(TaskKind::Download, "Alpha"),
            true,
            |task| async move {
                let events = task.sync_events();
                events.emit(SyncEvent::Transfer(TransferProgress {
                    transferred: 450_000_000,
                    total: 1_320_000_000,
                    current_file: Some("@ace/addons/ace_main.pbo".to_owned()),
                    current_file_percent: 40,
                    remaining: Some(Duration::from_secs(600)),
                }));
                events.emit(SyncEvent::Speed {
                    bytes_per_second: 12_500_000,
                });
                task.cancel.cancelled().await;
                Err(TaskError::Cancelled)
            },
        )
        .expect("task");
    let start = Instant::now();
    while viewer
        .task_snapshot(id)
        .is_none_or(|snapshot| snapshot.speed.is_none())
    {
        assert!(start.elapsed() < Duration::from_secs(10), "no progress");
        std::thread::sleep(Duration::from_millis(10));
    }
    let now = SystemTime::now();
    mutate(viewer, |state| {
        state.sync.downloads.recent = vec![
            record(
                "Beta",
                now,
                DownloadResult::Success {
                    trashed: 1,
                    userconfig: None,
                },
            ),
            record(
                "Alpha",
                now,
                DownloadResult::Issues(vec![
                    EventError::new(ErrorKind::ReadTimeout, "Read timed out")
                        .with_path("@cba/addons/cba_main.pbo"),
                ]),
            ),
            record(
                "Beta",
                now,
                DownloadResult::Failed(EventError::new(
                    ErrorKind::ConnectionFailed,
                    "Connection refused",
                )),
            ),
        ];
    });
    fixture.harness.get_by_label("Activity").click();
}

/// A top-level folder of a status tree with `files` to download of `size` bytes in all.
fn node(name: &str, status: NodeStatus, size: u64, files: usize) -> StatusNode {
    StatusNode {
        name: name.to_owned(),
        path: name.to_owned(),
        status,
        is_folder: true,
        hidden: false,
        download_size: size,
        files_to_download: files,
        files_to_remove: usize::from(status == NodeStatus::Removed),
        children: Vec::new(),
    }
}

fn record(repository: &str, finished: SystemTime, result: DownloadResult) -> DownloadRecord {
    DownloadRecord {
        repository: repository.to_owned(),
        event: None,
        finished,
        result,
        report: None,
    }
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

fn game_snapshot(name: &str, mode: ThemeMode) {
    let mut fixture = fixture_with(Start::default(), |builder| {
        builder.with_theme(mode.into()).wgpu()
    });
    theme::set_mode(&fixture.harness.ctx, mode);
    fixture.harness.get_by_label("Game").click();
    fixture.harness.run();
    assert_eq!(*fixture.harness.state().screen(), Screen::Game);
    fixture.harness.snapshot(name);
}

#[test]
fn game_light() {
    game_snapshot("game_light", ThemeMode::Light);
}

#[test]
fn game_dark() {
    game_snapshot("game_dark", ThemeMode::Dark);
}

#[test]
fn publishing_publish_light() {
    snapshot("publishing_publish_light", ThemeMode::Light, |fixture| {
        show_publishing(fixture, "Publish");
    });
}

#[test]
fn publishing_publish_dark() {
    snapshot("publishing_publish_dark", ThemeMode::Dark, |fixture| {
        show_publishing(fixture, "Publish");
    });
}

#[test]
fn publishing_events_light() {
    snapshot("publishing_events_light", ThemeMode::Light, |fixture| {
        show_publishing(fixture, "Events");
    });
}

#[test]
fn publishing_events_dark() {
    snapshot("publishing_events_dark", ThemeMode::Dark, |fixture| {
        show_publishing(fixture, "Events");
    });
}

#[test]
fn publishing_settings_light() {
    snapshot("publishing_settings_light", ThemeMode::Light, |fixture| {
        show_publishing(fixture, "Settings");
    });
}

#[test]
fn publishing_settings_dark() {
    snapshot("publishing_settings_dark", ThemeMode::Dark, |fixture| {
        show_publishing(fixture, "Settings");
    });
}

/// Beta published at revision 12 three days ago, with local changes, one event and a run
/// that failed at Verify; then the `tab` segment.
fn show_publishing(fixture: &mut Fixture, tab: &str) {
    let viewer = fixture.harness.state().viewer();
    let built = SystemTime::now() - Duration::from_secs(3 * 86_400);
    let build_date = built
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or_default())
        .unwrap_or_default();
    let mut events = EventsDoc::new();
    events.add_event(
        "Sunday operation",
        "Weekly co-op with the full mod set.",
        &EventSelection {
            addon_names: vec![("@ace".to_owned(), false), ("@cba".to_owned(), true)],
            ..EventSelection::default()
        },
    );
    let changes = ModChanges {
        added: vec!["@tfar".to_owned()],
        updated: vec!["@ace".to_owned()],
        removed: vec!["@old".to_owned()],
    };
    let mut run = PublishRun {
        revision: Some(13),
        changes: Some(changes.clone()),
        ..PublishRun::default()
    };
    run.steps[0] = StepState {
        status: StepStatus::Done,
        detail: StepDetail::Scanned {
            files: 412,
            changed_files: 6,
            changed_bytes: 84_000_000,
        },
    };
    run.steps[1] = StepState {
        status: StepStatus::Done,
        detail: StepDetail::Built {
            revision: 13,
            rehashed: 6,
        },
    };
    run.steps[2] = StepState {
        status: StepStatus::Failed(EventError::new(
            ErrorKind::ConnectionFailed,
            "Connection refused",
        )),
        detail: StepDetail::Verifying { percent: None },
    };
    mutate(viewer, |state| {
        state
            .client
            .update_repository("Beta", |repository| {
                repository.path = Some("/srv/arma3/beta".to_owned());
                Ok(true)
            })
            .expect("path");
        let publishing = state.admin.publishing_mut("Beta");
        publishing.remote = Remote::Loaded(RemoteRepository {
            server_info: Some(Arc::new(ServerInfo {
                revision: 12,
                build_date: Some(build_date),
                ..ServerInfo::default()
            })),
            sync: None,
        });
        publishing.local = Some(Ok(LocalScan {
            files: 412,
            bytes: 23_400_000_000,
            mods: vec!["@ace".to_owned(), "@cba".to_owned(), "@tfar".to_owned()],
            changed_files: 6,
            changed_bytes: 84_000_000,
            changes,
            built_revision: Some(12),
            history: Vec::new(),
        }));
        publishing.events = Some(events);
        publishing.run = Some(run);
        for (name, address) in [("Main", "10.0.0.1"), ("Training", "10.0.0.2")] {
            state
                .client
                .add_favorite_server(FavoriteServer {
                    name: Some(name.to_owned()),
                    ip_address: Some(address.to_owned()),
                    port: 2302,
                    ..FavoriteServer::default()
                })
                .expect("server added");
        }
        admin::set_autoconfig_server(&mut state.client, "Beta", 0, true).expect("included");
    });
    fixture
        .harness
        .get_all_by_label("Beta")
        .last()
        .expect("publishing row")
        .click();
    fixture.harness.run();
    fixture.harness.get_by_label(tab).click();
    if tab == "Settings" {
        fixture.harness.run();
        fixture
            .harness
            .get_by_role_and_label(egui::accesskit::Role::CheckBox, "Training")
            .scroll_to_me();
    }
}

/// A snapshot of a tool window over the main window. The temporary folders the window shows
/// stay until the snapshot is taken; `setup` returns the labels to mask (temporary paths).
fn tool_snapshot(
    name: &str,
    mode: ThemeMode,
    setup: impl FnOnce(&mut Fixture, &std::path::Path) -> Vec<String>,
) {
    let mut fixture = fixture_with(Start::default(), |builder| {
        builder.with_theme(mode.into()).wgpu()
    });
    theme::set_mode(&fixture.harness.ctx, mode);
    fixture.harness.run();
    let dir = tempfile::tempdir().expect("temp dir");
    let masked = setup(&mut fixture, dir.path());
    // Enough frames for the window to fade in.
    fixture.harness.run_steps(40);
    for label in masked {
        // Right-aligned labels: a fixed width from their right edge hides any path length.
        let mut rect = fixture.harness.get_by_label(&label).rect();
        rect.min.x = rect.max.x - 260.0;
        fixture.harness.mask(rect);
    }
    fixture.harness.snapshot(name);
}

fn show_server_keys(fixture: &mut Fixture, dir: &std::path::Path) -> Vec<String> {
    let mods = dir.join("mods");
    add_mod(&mods, "@ace", &["ace_3.bikey"]);
    add_mod(&mods, "@cba", &["cba_3.bikey"]);
    add_mod(&mods, "@unsigned", &[]);
    let target = dir.join("keys");
    std::fs::create_dir_all(&target).expect("target");
    let viewer = fixture.harness.state().viewer();
    enable_mods(viewer, &mods, &["@ace", "@cba", "@unsigned"]);
    let target = target.display().to_string();
    viewer
        .mutate(None, |state| {
            state
                .client
                .set_bikey_target_directory(Some(target.clone()))
        })
        .expect("target");
    open(&mut fixture.harness, SecondaryWindow::ServerKeys);
    wait_for_label(&mut fixture.harness, "Copy 2 keys");
    vec![target]
}

fn show_radio_plugin(fixture: &mut Fixture, dir: &std::path::Path) -> Vec<String> {
    let viewer = fixture.harness.state().viewer();
    let ts3 = radio_folders(viewer, dir, &[]);
    open(&mut fixture.harness, SecondaryWindow::RadioPlugin);
    wait_for_label(&mut fixture.harness, "Install plugin");
    vec![ts3.display().to_string()]
}

fn show_game_log(fixture: &mut Fixture, dir: &std::path::Path) -> Vec<String> {
    write(
        &dir.join("Arma3_x64_2026-10-13_20-00-00.rpt"),
        b"20:00:01 Starting mission\n20:00:02 Warning Message: No entry 'bin\\config.bin/CfgWeapons'\n20:00:03 Error in expression <player setPos _pos>\n20:00:04 Mission loaded\n",
    );
    let folder = dir.display().to_string();
    fixture
        .harness
        .state()
        .viewer()
        .mutate(None, |state| state.client.set_rpt_path(Some(folder)))
        .expect("folder");
    open(&mut fixture.harness, SecondaryWindow::GameLog);
    wait_for_label(&mut fixture.harness, "20:00:04 Mission loaded");
    Vec::new()
}

#[test]
fn server_keys_light() {
    tool_snapshot("server_keys_light", ThemeMode::Light, show_server_keys);
}

#[test]
fn server_keys_dark() {
    tool_snapshot("server_keys_dark", ThemeMode::Dark, show_server_keys);
}

#[test]
fn radio_plugin_light() {
    tool_snapshot("radio_plugin_light", ThemeMode::Light, show_radio_plugin);
}

#[test]
fn radio_plugin_dark() {
    tool_snapshot("radio_plugin_dark", ThemeMode::Dark, show_radio_plugin);
}

#[test]
fn game_log_light() {
    tool_snapshot("game_log_light", ThemeMode::Light, show_game_log);
}

#[test]
fn game_log_dark() {
    tool_snapshot("game_log_dark", ThemeMode::Dark, show_game_log);
}
