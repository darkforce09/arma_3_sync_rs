//! `sync`: synchronizes the content of a repository into a folder.

use std::path::Path;
use std::sync::Arc;

use a3s_foundation::error::ErrorKind;
use a3s_sync::cleanup::RemovedFiles;
use a3s_sync::pipeline::{
    CLI_RECONNECT_DELAY, CheckRequest, ReconnectPolicy, SyncEnd, SyncJob, UserconfigPolicy,
    sync_repository,
};
use a3s_sync::plan::NoAddons;
use a3s_sync::{EventError, EventSender, Outcome, Phase, SyncEvent};
use a3s_transport::CancellationToken;
use parking_lot::Mutex;

use super::{PercentLine, report, report_repository_error};
use crate::app::{App, CommandResult, Failed};
use crate::client_store::ClientStore;

/// Resumes after a lost connection before the synchronization gives up.
const MAX_RECONNECTS: u32 = 10;

/// No timeout: connections of the command line wait as long as the server needs.
// Mirrors console/CommandLine.java:104-105
const NO_TIMEOUT: &str = "0";

/// Stores `destination` as the default download location of `name`, its exact-match setting and
/// no timeouts, then synchronizes: check, download everything, check again. The revision is
/// stored through the client data (the server's after a success, -1 after a failure).
// Mirrors console/CommandLine.java:64-115
pub(crate) fn run(app: &App, name: &str, destination: &str, exact_match: bool) -> CommandResult {
    super::repository(app, name)?;
    if !Path::new(destination).is_dir() {
        say_error!("Error: destination folder path {destination} does not exist!");
        return Err(Failed);
    }
    let repository = {
        let mut data = app.data.lock();
        data.update_repository(name, |stored| {
            stored.default_download_location = Some(destination.to_owned());
            stored.exact_match = exact_match;
            stored.protocol.connection_timeout = Some(NO_TIMEOUT.to_owned());
            stored.protocol.read_timeout = Some(NO_TIMEOUT.to_owned());
            Ok(true)
        })
        .map_err(|err| report(&err))?;
        data.repository(name).cloned().map_err(|err| report(&err))?
    };
    synchronize(app, name, &repository.protocol)
}

/// Runs the headless synchronization of the stored repository `name` with its settings.
// Mirrors console/CommandGeneral.java:210-329
fn synchronize(app: &App, name: &str, protocol: &a3s_types::Protocol) -> CommandResult {
    say!("Synchronising with repository: {name}");
    let job = SyncJob {
        check: CheckRequest {
            repository_name: name.to_owned(),
            event_name: None,
            protocol: protocol.clone(),
            // The Java console never discovers addons: its addon map stays empty.
            addons: Arc::new(NoAddons),
            // Headless runs delete like Java: nobody empties a Trash on a server.
            removed_files: RemovedFiles::Delete,
        },
        arma3_exe_path: None,
        userconfig: UserconfigPolicy::Skip,
        reconnect: ReconnectPolicy {
            max_attempts: MAX_RECONNECTS,
            delay: CLI_RECONNECT_DELAY,
        },
    };
    let store = Arc::new(ClientStore::new(Arc::clone(&app.data)));
    let printer = Mutex::new(Progress::default());
    let repository_name = name.to_owned();
    let events = EventSender::from_fn(move |event| printer.lock().print(&repository_name, &event));
    let report = app.runtime.block_on(sync_repository(
        job,
        store,
        &CancellationToken::new(),
        &events,
    ));
    let errors = match report.end {
        SyncEnd::Finished(Outcome::Success) => {
            say!("Synchronization with repository: {name} finished.");
            return Ok(());
        }
        SyncEnd::ConnectionLost => vec![EventError::new(
            ErrorKind::ConnectionFailed,
            "Connection failed.",
        )],
        SyncEnd::Finished(Outcome::Failed { error }) => vec![error],
        SyncEnd::Finished(
            Outcome::CompletedWithErrors { errors } | Outcome::TooManyErrors { errors, .. },
        ) => errors,
        SyncEnd::Finished(_cancelled) => vec![EventError::new(
            ErrorKind::Cancelled,
            "Synchronization cancelled.",
        )],
    };
    say_error!("Synchronization with repository: {name} finished with errors:");
    for error in errors {
        report_repository_error(name, error.kind, &error.to_string());
    }
    Err(Failed)
}

/// The progress lines of a synchronization.
#[derive(Debug, Default)]
struct Progress {
    checking: PercentLine,
    completion: PercentLine,
    download: PercentLine,
}

impl Progress {
    // Mirrors console/CommandGeneral.java:318-329,392-404,494-499
    fn print(&mut self, repository: &str, event: &SyncEvent) {
        match event {
            SyncEvent::Phase(Phase::CheckingRepository) => {
                self.checking.reset();
                self.completion.reset();
            }
            SyncEvent::Phase(Phase::Downloading) => self.download.reset(),
            SyncEvent::Progress {
                phase: Phase::CheckingFiles,
                percent,
            } => self
                .checking
                .update("Checking files SHA1 signatures", *percent),
            SyncEvent::Progress {
                phase: Phase::DeterminingCompletion,
                percent,
            } => self
                .completion
                .update("Checking files completion", *percent),
            SyncEvent::Transfer(transfer) => self.download.update("Download", transfer.percent()),
            SyncEvent::ConnectionLost => {
                say_error!("Synchronization with repository: {repository} - Connection failed.");
            }
            _ => {}
        }
    }
}
