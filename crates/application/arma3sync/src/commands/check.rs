//! `check`: checks that the server holds every file of a repository.

use a3s_admin::check_repository_content;
use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_sync::{EventSender, Phase, SyncEvent};
use a3s_transport::TransferCtx;
use a3s_transport::proxy::transport_options;
use parking_lot::Mutex;

use super::{PercentLine, report_repository_error};
use crate::app::{App, CommandResult, Failed};

/// Checks the remote content of the repository `name` (through the configured proxy); fails when
/// a file is missing or the check failed.
// Mirrors console/CommandGeneral.java:127-206
pub(crate) fn run(app: &App, name: &str) -> CommandResult {
    let repository = super::repository(app, name)?;
    let proxy = app.data.lock().configuration().proxy.clone();
    let options = match transport_options(&repository.protocol, &proxy) {
        Ok(options) => options,
        Err(err) => {
            report_repository_error(name, ErrorKind::InvalidInput, &err.to_string());
            return Err(Failed);
        }
    };

    let progress = Mutex::new(PercentLine::default());
    let events = EventSender::from_fn(move |event| {
        if let SyncEvent::Progress {
            phase: Phase::CheckingRemoteFiles,
            percent,
        } = event
        {
            progress.lock().update("Repository check files", percent);
        }
    });
    let ctx = TransferCtx::new();
    let result = app.runtime.block_on(check_repository_content(
        &repository,
        &options,
        &ctx,
        &events,
    ));
    match result {
        Ok(report) => {
            let synchronized = report.is_synchronized();
            for line in report.console_lines(name) {
                say!("{line}");
            }
            if synchronized { Ok(()) } else { Err(Failed) }
        }
        Err(err) => {
            say_error!("Repository {name} - synchronization finished with error.");
            report_repository_error(name, err.kind(), &err.user_message());
            Err(Failed)
        }
    }
}
