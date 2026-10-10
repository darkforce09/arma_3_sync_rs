//! Port of `src/test/java/fr/soe/a3s/ui/main/tasks/TaskCheckRepositoriesTest.java`.

use a3s_sync::status::NotificationTracker;

const NAME: &str = "test-repository";

#[test]
fn a16_notifies_only_once_for_the_same_remote_revision() {
    let mut tracker = NotificationTracker::new();
    assert!(tracker.should_notify(NAME, true, false, Some(12)));
    assert!(!tracker.should_notify(NAME, true, false, Some(12)));
    assert!(tracker.should_notify(NAME, true, false, Some(13)));
}

#[test]
fn a16_automatic_updates_do_not_create_a_stale_pending_notification() {
    let mut tracker = NotificationTracker::new();
    assert!(!tracker.should_notify(NAME, true, true, Some(12)));
}

#[test]
fn a16_notifications_remain_available_when_remote_revision_cannot_be_read() {
    let mut tracker = NotificationTracker::new();
    assert!(tracker.should_notify(NAME, true, false, None));
}

#[test]
fn a16_disabled_notifications_and_forget() {
    let mut tracker = NotificationTracker::new();
    assert!(!tracker.should_notify(NAME, false, false, Some(1)));
    assert!(tracker.should_notify(NAME, true, false, Some(1)));
    tracker.forget(NAME);
    assert!(tracker.should_notify(NAME, true, false, Some(1)));
}
