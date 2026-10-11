//! User-visible texts of the update flows; the console prints them and a GUI renders them.

/// Title of the updater window.
// Mirrors a3sUpdater/ui/UIConstants.java:6
pub const WINDOW_TITLE: &str = "ArmA3Sync Updater";

/// Suggested size of the updater window, in points.
// Mirrors a3sUpdater/ui/Updater.java:47
pub const WINDOW_SIZE: [f32; 2] = [430.0, 150.0];

/// Updater window label while the release is resolved.
// Mirrors a3sUpdater/ui/Updater.java:24
pub const LABEL_CHECKING: &str = "Checking update metadata...";

/// Updater window label on `WaitingForElevation` (indeterminate progress).
// Mirrors a3sUpdater/ui/Updater.java:70-71
pub const LABEL_WAITING_FOR_UAC: &str = "Administrator rights required. Waiting for UAC...";

/// Updater window label on `Installing`.
// Mirrors a3sUpdater/ui/Updater.java:90
pub const LABEL_INSTALLING: &str = "Verifying and installing update...";

/// Updater window label after the user confirmed the cancellation.
// Mirrors a3sUpdater/ui/Updater.java:166-174
pub const LABEL_CANCELLING: &str = "Cancelling update...";

/// The updater window's only button.
pub const BUTTON_CANCEL: &str = "Cancel";

/// Message of the Yes/No cancel confirmation (title [`TITLE_UPDATE`]).
pub const CANCEL_CONFIRM_MESSAGE: &str = "Cancel update process?";

/// Title of the check, confirm, cancel and success dialogs.
// Mirrors ui/main/MainPanel.java:1012-1041
pub const TITLE_UPDATE: &str = "Update";

/// Title of an error dialog.
pub const TITLE_ERROR: &str = "Error";

/// Title of the dialog for an update that needs administrator rights.
pub const TITLE_ADMIN_REQUIRED: &str = "Update requires administrator rights";

/// Tray notification when a check finds an update.
pub const TRAY_AVAILABLE: &str = "A new update is available";

/// OK/Cancel question when a check finds an update.
pub const CONFIRM_UPDATE: &str = "A new update is available. Proceed update?";

/// Result of a check without a newer release.
// Mirrors a3sUpdater/console/Console.java:26
pub const NO_UPDATE: &str = "No new update available.";

/// Logged when the application checks for updates.
pub const CHECKING_FOR_UPDATES: &str = "Checking for updates...";

/// Console line before the installation starts.
// Mirrors a3sUpdater/console/Console.java:49
pub const PROCESSING: &str = "Processing update...";

/// Console line when the elevated updater took over.
// Mirrors a3sUpdater/console/Console.java:34
pub const ELEVATED_CONTINUING: &str =
    "Administrator rights requested. The elevated updater is continuing the update.";

/// First line of an error that needs administrator rights.
// Mirrors a3sUpdater/console/Console.java:38
pub const ADMIN_REQUIRED: &str = "Administrator rights are required to update this installation.";

/// Last line of a failed update.
// Mirrors a3sUpdater/console/Console.java:75
pub const ABORTED: &str = "Update process aborted.";

// Mirrors a3sUpdater/console/Console.java:43
pub fn updating(version: &str) -> String {
    format!("Updating ArmA3Sync to version {version}...")
}

// Mirrors a3sUpdater/console/Console.java:51
pub fn success_message(version: &str) -> String {
    format!("ArmA3Sync has been successfully updated to version {version}.")
}

// Mirrors a3sUpdater/console/Console.java:88
pub fn update_available(version: &str) -> String {
    format!("Update available: {version}")
}

// Mirrors a3sUpdater/console/Console.java:96
pub fn check_failed(error: &dyn std::fmt::Display) -> String {
    format!("Update check failed: {error}")
}

// Mirrors a3sUpdater/console/Console.java:74
pub fn error_occurred(error: &dyn std::fmt::Display) -> String {
    format!("An error occurred: {error}")
}

/// Console progress, printed after a `\r` without a line end.
// Mirrors a3sUpdater/console/Console.java:101-105
pub fn downloading(percent: u64) -> String {
    format!("Downloading: {percent}%")
}

/// Download progress in percent, at most 100; `None` when the total is unknown or zero.
// Mirrors a3sUpdater/ui/Updater.java:96-102
pub fn progress_percent(bytes: u64, total: Option<u64>) -> Option<u64> {
    let total = total.filter(|&total| total > 0)?;
    let percent = (u128::from(bytes) * 100 / u128::from(total)).min(100);
    Some(u64::try_from(percent).unwrap_or(100))
}
