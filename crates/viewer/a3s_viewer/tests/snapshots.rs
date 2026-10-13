// Its own test binary: wgpu rendering on a CPU adapter, Linux only, regenerated with UPDATE_SNAPSHOTS=1.
//! Headless wgpu snapshots of the main window with two repositories, one with updates, and of
//! the Repository, Activity and Publishing screens.
#![cfg(target_os = "linux")]

#[allow(
    dead_code,
    reason = "shared with the interaction tests, which use more of it"
)]
#[path = "viewer/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use a3s_admin::{EventSelection, EventsDoc};
use a3s_foundation::error::ErrorKind;
use a3s_sync::{EventError, SyncEvent, TransferProgress};
use a3s_types::{Changelog, Changelogs, Event, Events, ServerInfo};
use a3s_ui::theme::{self, ThemeMode};
use a3s_viewer_context::admin::{
    LocalScan, ModChanges, PublishRun, Remote, RemoteRepository, StepDetail, StepState, StepStatus,
};
use a3s_viewer_context::sync::{
    AddonChange, ChangeKind, DownloadRecord, DownloadResult, PlanState, SyncPlan,
};
use a3s_viewer_context::{TaskError, TaskKey, TaskKind, ViewerContext};
use egui_kittest::kittest::Queryable as _;

use crate::support::{Fixture, Start, fixture_with};

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
                result: Ok(SyncPlan {
                    download_size: 1_320_000_000,
                    file_count: 14,
                    changes: vec![
                        change("@ace", ChangeKind::Updated, 1_200_000_000),
                        change("@new", ChangeKind::New, 120_000_000),
                        change("@old", ChangeKind::Removed, 0),
                    ],
                }),
            },
        );
    });
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
            record("Beta", now, DownloadResult::Success { trashed: 1 }),
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

fn change(name: &str, kind: ChangeKind, size: u64) -> AddonChange {
    AddonChange {
        name: name.to_owned(),
        kind,
        size,
    }
}

fn record(repository: &str, finished: SystemTime, result: DownloadResult) -> DownloadRecord {
    DownloadRecord {
        repository: repository.to_owned(),
        event: None,
        finished,
        result,
    }
}

fn mutate(viewer: &ViewerContext, f: impl FnOnce(&mut a3s_viewer_context::AppState)) {
    viewer
        .mutate(None, |state| {
            f(state);
            Ok::<(), Infallible>(())
        })
        .ok();
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
        }));
        publishing.events = Some(events);
        publishing.run = Some(run);
    });
    fixture
        .harness
        .get_all_by_label("Beta")
        .last()
        .expect("publishing row")
        .click();
    fixture.harness.run();
    fixture.harness.get_by_label(tab).click();
}
