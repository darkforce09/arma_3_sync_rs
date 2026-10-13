//! `CheckSchedule` arithmetic, `ManualClock` and `SystemClock`.

use std::time::{Duration, Instant, SystemTime};

use a3s_types::constants::CheckRepositoriesFrequency;
use a3s_viewer_context::{CheckSchedule, Clock as _, ManualClock, SystemClock};

use crate::support::{minutes, start_time};

#[test]
fn schedule_periods() {
    assert_eq!(
        CheckSchedule::period(CheckRepositoriesFrequency::Freq1),
        Some(minutes(15))
    );
    assert_eq!(
        CheckSchedule::period(CheckRepositoriesFrequency::Freq2),
        Some(minutes(30))
    );
    assert_eq!(
        CheckSchedule::period(CheckRepositoriesFrequency::Freq3),
        Some(minutes(60))
    );
    assert_eq!(
        CheckSchedule::period(CheckRepositoriesFrequency::Disabled),
        None
    );
}

#[test]
fn disabled_checks_once_at_startup() {
    let t = start_time();
    let mut schedule = CheckSchedule::at_startup(CheckRepositoriesFrequency::Disabled, t);
    assert_eq!(schedule.next_due(), Some(t));
    schedule.on_cycle_started(t);
    assert_eq!(schedule.next_due(), None);
}

#[test]
fn next_check_counts_from_the_start_of_a_cycle() {
    let t = start_time();
    let mut schedule = CheckSchedule::at_startup(CheckRepositoriesFrequency::Freq1, t);
    assert_eq!(schedule.next_due(), Some(t));
    schedule.on_cycle_started(t + minutes(2));
    assert_eq!(schedule.next_due(), Some(t + minutes(17)));
    assert_eq!(schedule.frequency(), CheckRepositoriesFrequency::Freq1);
}

#[test]
fn a_changed_frequency_waits_one_period() {
    let t = start_time();
    assert_eq!(
        CheckSchedule::after_change(CheckRepositoriesFrequency::Freq2, t).next_due(),
        Some(t + minutes(30))
    );
    assert_eq!(
        CheckSchedule::after_change(CheckRepositoriesFrequency::Disabled, t).next_due(),
        None
    );
}

#[tokio::test]
async fn manual_clock_wakes_sleepers_at_their_deadline() {
    let t = start_time();
    let clock = ManualClock::new(t);
    let mut sleep = clock.sleep_until(t + Duration::from_secs(10));
    assert!(futures::poll!(&mut sleep).is_pending());
    assert_eq!(clock.sleepers(), 1);
    clock.wait_for_sleepers(1).await;

    clock.advance(Duration::from_secs(9));
    assert!(futures::poll!(&mut sleep).is_pending());
    clock.advance(Duration::from_secs(1));
    assert!(futures::poll!(&mut sleep).is_ready());
    assert_eq!(clock.sleepers(), 0);

    let mut past = clock.sleep_until(t);
    assert!(futures::poll!(&mut past).is_ready());

    let dropped = clock.sleep_until(t + minutes(1));
    assert_eq!(clock.sleepers(), 1);
    drop(dropped);
    assert_eq!(clock.sleepers(), 0);

    let mut later = clock.sleep_until(t + minutes(5));
    clock.set(t + minutes(5));
    assert!(futures::poll!(&mut later).is_ready());
}

#[tokio::test]
async fn system_clock_sleeps_until_the_deadline() {
    let clock = SystemClock;
    let started = Instant::now();
    clock
        .sleep_until(SystemTime::now() - Duration::from_secs(1))
        .await;
    assert!(started.elapsed() < Duration::from_millis(50));

    let started = Instant::now();
    clock
        .sleep_until(SystemTime::now() + Duration::from_millis(50))
        .await;
    assert!(started.elapsed() >= Duration::from_millis(50));
}
