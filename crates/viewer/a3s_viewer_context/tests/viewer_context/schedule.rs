//! The periodic check loop on a manual clock, and update notifications across cycles (the
//! integration counterpart of Java's `TaskCheckRepositoriesTest`; its third case, a notification
//! without a readable server revision, cannot happen through a cycle: without server info the
//! status is `Indeterminated`, so it is covered at function level in `a3s_sync`).

use std::sync::Arc;
use std::time::Duration;

use a3s_sync::RemoteMetadata;
use a3s_types::ServerInfo;
use a3s_types::constants::CheckRepositoriesFrequency;
use a3s_viewer_context::repositories::Event as RepositoryEvent;
use a3s_viewer_context::{AppEvent, TaskEvent, TaskKind, TaskOutcome, sync};
use tokio::sync::Semaphore;

use crate::support::{
    Fixture, Script, count_started, http_repository, minutes, outcome_of, started_kind,
};

fn periodic_started(fixture: &Fixture) {
    let event = fixture.next();
    assert_eq!(
        started_kind(&event),
        Some(TaskKind::PeriodicCheck),
        "{event:?}"
    );
}

/// Waits for the end of the running cycle, returning its events.
fn finish_cycle(fixture: &Fixture) -> Vec<AppEvent> {
    let events = fixture.until_finished(TaskKind::PeriodicCheck);
    assert_eq!(
        outcome_of(&events, TaskKind::PeriodicCheck),
        TaskOutcome::Completed
    );
    events
}

/// Lets the scheduler pick up a new schedule before the clock moves.
fn settle() {
    std::thread::sleep(Duration::from_millis(100));
}

fn no_cycle(fixture: &Fixture) {
    let events = fixture.quiet();
    assert_eq!(
        count_started(&events, TaskKind::PeriodicCheck),
        0,
        "{events:#?}"
    );
}

fn gated(fixture: &Fixture, name: &str) -> Arc<Semaphore> {
    let gate = Arc::new(Semaphore::new(0));
    fixture.checker.set(
        name,
        Script {
            gate: Some(Arc::clone(&gate)),
            ..Script::default()
        },
    );
    gate
}

#[test]
fn checks_repeat_one_period_after_each_start() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    fixture.ctx.start_background();
    periodic_started(&fixture);
    finish_cycle(&fixture);

    fixture.advance_after_sleep(Duration::from_secs(15 * 60 - 1));
    no_cycle(&fixture);
    fixture.clock.advance(Duration::from_secs(1));
    periodic_started(&fixture);
    finish_cycle(&fixture);
}

#[test]
fn disabled_checks_once_at_startup() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Disabled,
    );
    fixture.ctx.start_background();
    periodic_started(&fixture);
    finish_cycle(&fixture);
    settle();
    fixture.clock.advance(minutes(120));
    no_cycle(&fixture);
}

#[test]
fn a_new_frequency_starts_one_period_after_the_change() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    fixture.ctx.start_background();
    periodic_started(&fixture);
    finish_cycle(&fixture);

    fixture
        .ctx
        .reschedule_checks(CheckRepositoriesFrequency::Freq2);
    settle();
    fixture.advance_after_sleep(minutes(15));
    no_cycle(&fixture);
    fixture.clock.advance(minutes(15));
    periodic_started(&fixture);
    finish_cycle(&fixture);

    fixture
        .ctx
        .reschedule_checks(CheckRepositoriesFrequency::Disabled);
    settle();
    fixture.clock.advance(minutes(180));
    no_cycle(&fixture);
}

#[test]
fn set_preferences_reschedules_only_on_a_new_frequency() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    fixture.ctx.start_background();
    periodic_started(&fixture);
    finish_cycle(&fixture);

    // Same frequency: the next check stays one period after the first.
    let mut preferences = fixture.ctx.read().client.preferences().clone();
    preferences.prefer_github_updates = !preferences.prefer_github_updates;
    fixture
        .ctx
        .set_preferences(preferences.clone())
        .expect("saved");
    settle();
    fixture.advance_after_sleep(minutes(15));
    periodic_started(&fixture);
    finish_cycle(&fixture);

    // New frequency: one period of 30 minutes from the change.
    preferences.check_repositories_frequency = CheckRepositoriesFrequency::Freq2;
    fixture.ctx.set_preferences(preferences).expect("saved");
    settle();
    fixture.advance_after_sleep(minutes(15));
    no_cycle(&fixture);
    fixture.clock.advance(minutes(15));
    periodic_started(&fixture);
    finish_cycle(&fixture);
}

#[test]
fn a_late_cycle_is_followed_at_once_without_overlap() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    let gate = gated(&fixture, "A");
    fixture.ctx.start_background();
    periodic_started(&fixture);
    fixture.clock.advance(minutes(20));
    no_cycle(&fixture);

    gate.add_permits(1);
    let events = fixture.until(|event| started_kind(event) == Some(TaskKind::PeriodicCheck));
    let finished = events
        .iter()
        .position(|event| {
            matches!(event, AppEvent::Task(TaskEvent::Finished { info, .. })
                if info.key.kind == TaskKind::PeriodicCheck)
        })
        .expect("first cycle finished");
    assert!(finished < events.len() - 1);
    gate.add_permits(1);
    finish_cycle(&fixture);
}

#[test]
fn a_reschedule_lets_the_running_cycle_finish() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    let gate = gated(&fixture, "A");
    fixture.ctx.start_background();
    periodic_started(&fixture);
    fixture
        .ctx
        .reschedule_checks(CheckRepositoriesFrequency::Freq2);
    gate.add_permits(1);
    finish_cycle(&fixture);

    gate.add_permits(10);
    settle();
    fixture.advance_after_sleep(minutes(15));
    no_cycle(&fixture);
    fixture.clock.advance(minutes(15));
    periodic_started(&fixture);
    finish_cycle(&fixture);
}

#[test]
fn a_second_start_does_not_double_the_cycles() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    fixture.ctx.start_background();
    fixture.ctx.start_background();
    periodic_started(&fixture);
    finish_cycle(&fixture);
    no_cycle(&fixture);

    fixture.ctx.start_background();
    fixture.advance_after_sleep(minutes(15));
    let events = fixture.quiet();
    assert_eq!(
        count_started(&events, TaskKind::PeriodicCheck),
        1,
        "{events:#?}"
    );
}

#[test]
fn stop_cancels_the_running_cycle_and_ends_the_schedule() {
    let fixture = Fixture::with_frequency(
        vec![http_repository("A")],
        CheckRepositoriesFrequency::Freq1,
    );
    gated(&fixture, "A");
    fixture.ctx.start_background();
    periodic_started(&fixture);
    fixture.ctx.stop_background();
    let events = fixture.until_finished(TaskKind::PeriodicCheck);
    assert_eq!(
        outcome_of(&events, TaskKind::PeriodicCheck),
        TaskOutcome::Cancelled
    );
    fixture.clock.advance(minutes(60));
    no_cycle(&fixture);
}

fn server_revision(revision: i32) -> Script {
    Script {
        install: Some(RemoteMetadata {
            server_info: Some(Arc::new(ServerInfo {
                revision,
                repository_content_updated: true,
                ..ServerInfo::default()
            })),
            ..RemoteMetadata::default()
        }),
        ..Script::default()
    }
}

fn notified(events: &[AppEvent]) -> bool {
    events.iter().any(|event| {
        matches!(event, AppEvent::Repositories(RepositoryEvent::UpdatesAvailable { repositories })
            if repositories == &["R"])
    })
}

/// Runs the next cycle (one period later) and returns its events.
fn next_cycle(fixture: &Fixture) -> Vec<AppEvent> {
    fixture.advance_after_sleep(minutes(15));
    periodic_started(fixture);
    finish_cycle(fixture)
}

#[test]
fn notifies_only_once_for_the_same_remote_revision() {
    let mut repository = http_repository("R");
    repository.revision = 5;
    repository.notify = true;
    let fixture = Fixture::with_frequency(vec![repository], CheckRepositoriesFrequency::Freq1);
    fixture.checker.set("R", server_revision(12));
    fixture.ctx.start_background();
    periodic_started(&fixture);
    assert!(notified(&finish_cycle(&fixture)));
    assert!(!notified(&next_cycle(&fixture)));

    // A reschedule keeps the memory of notified revisions.
    fixture
        .ctx
        .reschedule_checks(CheckRepositoriesFrequency::Freq1);
    settle();
    assert!(!notified(&next_cycle(&fixture)));

    fixture.checker.set("R", server_revision(13));
    assert!(notified(&next_cycle(&fixture)));

    // Up to date: forgotten, so the same revision notifies again once it is an update.
    fixture
        .ctx
        .mutate(None, |state| {
            state.client.update_repository("R", |r| {
                r.revision = 13;
                Ok(true)
            })
        })
        .expect("saved");
    assert!(!notified(&next_cycle(&fixture)));
    fixture
        .ctx
        .mutate(None, |state| {
            state.client.update_repository("R", |r| {
                r.revision = 5;
                Ok(true)
            })
        })
        .expect("saved");
    assert!(notified(&next_cycle(&fixture)));
}

#[test]
fn automatic_updates_do_not_notify() {
    let mut repository = http_repository("R");
    repository.revision = 5;
    repository.notify = true;
    repository.auto = true;
    let fixture = Fixture::with_frequency(vec![repository], CheckRepositoriesFrequency::Freq1);
    fixture.checker.set("R", server_revision(12));
    fixture.ctx.start_background();
    periodic_started(&fixture);
    for events in [finish_cycle(&fixture), next_cycle(&fixture)] {
        assert!(!notified(&events));
        assert!(events.iter().any(|event| matches!(
            event,
            AppEvent::Sync(sync::Event::AutoUpdateRequested { repositories }) if repositories == &["R"]
        )));
    }
}
