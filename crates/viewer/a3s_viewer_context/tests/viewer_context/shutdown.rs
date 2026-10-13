//! The exit sequence: stop, cancel, wait, save, shut down.

use std::time::{Duration, Instant};

use a3s_types::constants::CheckRepositoriesFrequency;
use a3s_viewer_context::{
    Command, ContextError, ContextOptions, TaskError, TaskKey, TaskKind, TaskOutcome, sync,
};
use tokio::sync::oneshot;

use crate::support::{
    Fixture, FixtureOptions, count_started, http_repository, minutes, outcome_of, store,
};

#[test]
fn prepare_exit_cancels_everything_and_stops_the_schedule() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    fixture.ctx.start_background();
    fixture.until_finished(TaskKind::PeriodicCheck);
    fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::Download), false, |task| async move {
            task.cancel.cancelled().await;
            Err(TaskError::Cancelled)
        })
        .expect("spawn");

    let started = Instant::now();
    assert!(fixture.ctx.prepare_exit().is_empty());
    assert!(started.elapsed() < Duration::from_secs(1));
    let events = fixture.until_finished(TaskKind::Download);
    assert_eq!(
        outcome_of(&events, TaskKind::Download),
        TaskOutcome::Cancelled
    );

    fixture.clock.advance(minutes(120));
    let events = fixture.quiet();
    assert_eq!(count_started(&events, TaskKind::PeriodicCheck), 0);

    // Staying after a failed save: the schedule starts again.
    fixture.ctx.start_background();
    let events = fixture.until_finished(TaskKind::PeriodicCheck);
    assert_eq!(count_started(&events, TaskKind::PeriodicCheck), 1);
}

#[test]
fn prepare_exit_returns_tasks_that_ignore_their_token() {
    let fixture = Fixture::with(FixtureOptions {
        options: ContextOptions {
            worker_threads: 2,
            synchronizer: None,
            shutdown_timeout: Duration::from_millis(200),
            ..ContextOptions::default()
        },
        ..FixtureOptions::default()
    });
    let (_never, never_received) = oneshot::channel::<()>();
    let id = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::RptTail), false, |_| async move {
            never_received.await.ok();
            Ok(())
        })
        .expect("spawn");

    let started = Instant::now();
    let left = fixture.ctx.prepare_exit();
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, id);
}

#[test]
fn save_on_exit_writes_the_configuration() {
    let fixture = Fixture::new(Vec::new());
    fixture.ctx.save_on_exit(600, 800).expect("saved");
    let configuration = store(fixture.dir.path())
        .read_configuration()
        .expect("written");
    assert_eq!((configuration.height, configuration.width), (600, 800));
}

#[test]
fn shutdown_is_idempotent_and_closes_everything() {
    let mut fixture = Fixture::new(Vec::new());
    fixture.ctx.shutdown();
    fixture.ctx.shutdown();
    let sent = fixture.ctx.send(Command::Sync(sync::Command::AutoUpdate {
        repositories: Vec::new(),
    }));
    assert!(matches!(sent, Err(ContextError::Closed)));
    let spawned = fixture
        .ctx
        .cx()
        .spawn_task(TaskKey::new(TaskKind::Build), false, |_| async { Ok(()) });
    assert!(matches!(spawned, Err(ContextError::Closed)));
}

#[test]
fn a_context_with_its_own_runtime_drops_on_a_plain_thread() {
    let fixture = Fixture::new(Vec::new());
    std::thread::Builder::new()
        .name("drop-context".to_owned())
        .spawn(move || drop(fixture))
        .expect("spawn thread")
        .join()
        .expect("dropped without panic");
}
