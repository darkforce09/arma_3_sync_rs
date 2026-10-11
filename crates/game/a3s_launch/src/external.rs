//! Starting the external applications configured to run with the game (`TeamSpeak`, tools).

use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;

use a3s_foundation::paths::Os;
use a3s_platform::process::{ProcessProbe, is_running};
use a3s_types::local::ExternalApplication;

use crate::LaunchError;
use crate::command::plan_external;
use crate::executable::file_name;
use crate::params::tokenize_parameters;
use crate::runner::{EventSink, LaunchEvent, LaunchTarget, run_once};

/// Starts every enabled application whose executable exists and is not already running, each on
/// its own `a3s-launch-ext-<index>` thread that waits for it, and returns those threads.
///
/// Applications are taken in list order; one whose kind does not run on `os` is skipped, one
/// that cannot run at all sends an `Error` event. Exit codes are ignored. Java starts only the
/// first application (its executor is shut down inside the loop); every one is started here.
// Mirrors service/LaunchService.java:58-92
pub fn launch_external_applications(
    os: Os,
    apps: &[ExternalApplication],
    probe: &dyn ProcessProbe,
    events: &EventSink,
) -> Vec<JoinHandle<()>> {
    let mut threads = Vec::new();
    for (index, app) in apps.iter().enumerate() {
        if !app.enable {
            continue;
        }
        let Some(exe) = app.executable_path.as_deref().map(Path::new) else {
            continue;
        };
        if !exe.exists() || is_running(probe, &file_name(exe)) {
            continue;
        }
        let target = LaunchTarget::External {
            name: app.name.clone().unwrap_or_else(|| file_name(exe)),
        };
        let params = tokenize_parameters(app.parameters.as_deref().unwrap_or_default());
        let command = match plan_external(os, exe, &params) {
            Ok(Some(command)) => command,
            Ok(None) => continue,
            Err(error) => {
                send_error(events, target, error);
                continue;
            }
        };
        let thread_events = Arc::clone(events);
        let thread_target = target.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("a3s-launch-ext-{index}"))
            .spawn(move || {
                if let Err(error) = run_once(
                    &command,
                    &command.elements(),
                    &thread_target,
                    &thread_events,
                ) {
                    send_error(&thread_events, thread_target, error);
                }
            });
        match spawned {
            Ok(thread) => threads.push(thread),
            Err(err) => send_error(events, target, LaunchError::Thread(err)),
        }
    }
    threads
}

fn send_error(events: &EventSink, target: LaunchTarget, error: LaunchError) {
    events(LaunchEvent::Error {
        target,
        error: Arc::new(error),
    });
}
