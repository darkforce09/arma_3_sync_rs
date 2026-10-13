use std::time::{Duration, SystemTime};

use a3s_types::constants::CheckRepositoriesFrequency;

/// When the next periodic repository check is due, for one check frequency.
///
/// Fixed-delay like Java's `Timer.schedule(task, delay, period)`: the next check is due one
/// period after the **start** of the current one, and checks never overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckSchedule {
    frequency: CheckRepositoriesFrequency,
    next_due: Option<SystemTime>,
}

impl CheckSchedule {
    /// The period of `frequency`; `None` when checks are disabled.
    // Mirrors ui/main/MainPanel.java:1082-1083
    pub fn period(frequency: CheckRepositoriesFrequency) -> Option<Duration> {
        u64::try_from(frequency.minutes())
            .ok()
            .filter(|minutes| *minutes > 0)
            .map(|minutes| Duration::from_secs(minutes * 60))
    }

    /// The schedule at start-up: one check at once, for every frequency. With
    /// [`CheckRepositoriesFrequency::Disabled`] that check is the only one.
    // Mirrors ui/main/MainPanel.java:1076-1086
    pub fn at_startup(frequency: CheckRepositoriesFrequency, now: SystemTime) -> Self {
        Self {
            frequency,
            next_due: Some(now),
        }
    }

    /// The schedule after the frequency was changed: the first check comes one period later,
    /// none when disabled.
    // Mirrors ui/help/PreferencesDialog.java:418-427
    pub fn after_change(frequency: CheckRepositoriesFrequency, now: SystemTime) -> Self {
        Self {
            frequency,
            next_due: Self::period(frequency).map(|period| now + period),
        }
    }

    /// A check started at `started`: the next one is due one period later, or never when
    /// disabled.
    pub fn on_cycle_started(&mut self, started: SystemTime) {
        self.next_due = Self::period(self.frequency).map(|period| started + period);
    }

    /// When the next check is due; `None` for no further check.
    pub fn next_due(&self) -> Option<SystemTime> {
        self.next_due
    }

    /// The check frequency.
    pub fn frequency(&self) -> CheckRepositoriesFrequency {
        self.frequency
    }
}
