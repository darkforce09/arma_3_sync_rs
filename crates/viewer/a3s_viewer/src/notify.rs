//! Desktop notifications, used while the window is hidden in the tray.

/// Shows `message` as a desktop notification on a background thread (the D-Bus call blocks);
/// a failure is logged.
pub(crate) fn desktop(message: &str) {
    let message = message.to_owned();
    let spawned = std::thread::Builder::new()
        .name("a3s-notify".to_owned())
        .spawn(move || {
            if let Err(err) = notify_rust::Notification::new()
                .appname("Arma3Sync")
                .summary("Arma3Sync")
                .body(&message)
                .show()
            {
                tracing::warn!("Cannot show the desktop notification: {err}");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("Cannot start the notification thread: {err}");
    }
}
