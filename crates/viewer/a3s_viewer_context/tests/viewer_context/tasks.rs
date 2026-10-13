//! Task lifecycle: events, outcomes, cancellation, panics and exclusive keys.

use std::time::{Duration, Instant};

use a3s_repo_files::A3sFile;
use a3s_sync::{EventError, RemoteError};
use a3s_viewer_context::{
    AppEvent, ContextError, TaskError, TaskEvent, TaskId, TaskKey, TaskKind, TaskOutcome,
};
use tokio::sync::oneshot;

use crate::support::Fixture;

fn finished(event: AppEvent) -> (TaskId, TaskOutcome) {
    match event {
        AppEvent::Task(TaskEvent::Finished { info, outcome }) => (info.id, outcome),
        other => panic!("expected Finished, got {other:?}"),
    }
}

fn started(event: AppEvent) -> TaskId {
    match event {
        AppEvent::Task(TaskEvent::Started(info)) => info.id,
        other => panic!("expected Started, got {other:?}"),
    }
}

/// Spawns a task that waits for its token, then reports a cancel.
fn spawn_cooperative(fixture: &Fixture, key: TaskKey, exclusive: bool) -> TaskId {
    fixture
        .ctx
        .cx()
        .spawn_task(key, exclusive, |task| async move {
            task.cancel.cancelled().await;
            Err(TaskError::Cancelled)
        })
        .expect("spawn")
}

#[test]
fn completed_task_is_listed_until_finished() {
    let fixture = Fixture::new(Vec::new());
    let (release, released) = oneshot::channel::<()>();
    let id = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::Build), false, |_| async move {
            released.await.ok();
            Ok(())
        })
        .expect("spawn");
    assert_eq!(id, TaskId(1));
    assert_eq!(started(fixture.next()), id);
    let running = fixture.ctx.running_tasks();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, id);
    assert_eq!(running[0].key, TaskKey::new(TaskKind::Build));

    release.send(()).expect("task waiting");
    assert_eq!(finished(fixture.next()), (id, TaskOutcome::Completed));
    assert!(fixture.ctx.running_tasks().is_empty());

    let second = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::Build), false, |_| async { Ok(()) })
        .expect("spawn");
    assert_eq!(second, TaskId(2));
}

#[test]
fn failed_task_reports_the_error() {
    let fixture = Fixture::new(Vec::new());
    let id = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::Download), false, |_| async {
            Err(TaskError::Remote(RemoteError::MissingMetadata(
                A3sFile::Sync,
            )))
        })
        .expect("spawn");
    started(fixture.next());
    let expected = EventError::from_remote(&RemoteError::MissingMetadata(A3sFile::Sync));
    assert_eq!(
        finished(fixture.next()),
        (id, TaskOutcome::Failed(expected))
    );
}

#[test]
fn cancelled_task_ends_promptly() {
    let fixture = Fixture::new(Vec::new());
    let id = spawn_cooperative(&fixture, TaskKey::new(TaskKind::Upload), false);
    started(fixture.next());
    let cancelled_at = Instant::now();
    assert!(fixture.ctx.cancel_task(id));
    assert_eq!(finished(fixture.next()), (id, TaskOutcome::Cancelled));
    assert!(cancelled_at.elapsed() < Duration::from_secs(1));
    assert!(!fixture.ctx.cancel_task(id));
    assert!(!fixture.ctx.cancel_task(TaskId(99)));
}

#[test]
fn an_error_after_a_cancel_counts_as_cancelled() {
    let fixture = Fixture::new(Vec::new());
    let id = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::Upload), false, |task| async move {
            task.cancel.cancelled().await;
            Err(TaskError::Remote(RemoteError::MissingMetadata(
                A3sFile::Sync,
            )))
        })
        .expect("spawn");
    started(fixture.next());
    fixture.ctx.cancel_task(id);
    assert_eq!(finished(fixture.next()), (id, TaskOutcome::Cancelled));
}

#[test]
fn a_panic_is_reported_and_the_context_survives() {
    let fixture = Fixture::new(Vec::new());
    let id = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::AddonScan), false, |_| async {
            panic!("boom");
        })
        .expect("spawn");
    started(fixture.next());
    assert_eq!(
        finished(fixture.next()),
        (id, TaskOutcome::Panicked("boom".to_owned()))
    );

    let id = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::AddonScan), false, |_| async {
            Ok(())
        })
        .expect("spawn");
    started(fixture.next());
    assert_eq!(finished(fixture.next()), (id, TaskOutcome::Completed));
}

#[test]
fn exclusive_keys() {
    let fixture = Fixture::new(Vec::new());
    let key = TaskKey::new(TaskKind::CheckRepositories);
    let first = spawn_cooperative(&fixture, key.clone(), true);

    let busy = fixture
        .ctx
        .cx()
        .spawn_task(key.clone(), true, |_| async { Ok(()) });
    match busy {
        Err(ContextError::TaskBusy { id, key: busy_key }) => {
            assert_eq!(id, first);
            assert_eq!(busy_key, key);
        }
        other => panic!("expected TaskBusy, got {other:?}"),
    }
    assert_eq!(fixture.ctx.is_running(&key), Some(first));

    // Another repository in the key, or a non-exclusive spawn, is accepted.
    spawn_cooperative(
        &fixture,
        TaskKey::repository(TaskKind::CheckRepositories, "R"),
        true,
    );
    spawn_cooperative(&fixture, key, false);
    assert_eq!(fixture.ctx.running_tasks().len(), 3);
    assert_eq!(fixture.ctx.cancel_all(), 3);
}

#[test]
fn cancel_repository_tasks_cancels_only_that_repository() {
    let fixture = Fixture::new(Vec::new());
    let r1 = spawn_cooperative(
        &fixture,
        TaskKey::repository(TaskKind::Download, "R"),
        false,
    );
    let r2 = spawn_cooperative(&fixture, TaskKey::repository(TaskKind::Build, "R"), false);
    let s = spawn_cooperative(
        &fixture,
        TaskKey::repository(TaskKind::Download, "S"),
        false,
    );
    let none = spawn_cooperative(&fixture, TaskKey::new(TaskKind::Download), false);
    for _ in 0..4 {
        started(fixture.next());
    }

    assert_eq!(fixture.ctx.cancel_repository_tasks("R"), 2);
    let mut ended = vec![finished(fixture.next()), finished(fixture.next())];
    ended.sort_by_key(|(id, _)| *id);
    assert_eq!(
        ended,
        [(r1, TaskOutcome::Cancelled), (r2, TaskOutcome::Cancelled)]
    );
    let running: Vec<TaskId> = fixture.ctx.running_tasks().iter().map(|t| t.id).collect();
    assert_eq!(running, [s, none]);
}
