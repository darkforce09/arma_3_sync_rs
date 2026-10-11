//! Console update flows (the `update` command and the legacy `-update` flag) and the updater
//! flag grammar. Output goes to the given writers; write errors are ignored.

use std::io::Write;

use parking_lot::Mutex;

use super::{CheckOutcome, UpdateEvent, UpdateSession, text};
use crate::{CancellationToken, Release, UpdateSource};

/// Exit code: an update is available (check), was installed, or the hand-off succeeded.
// Mirrors service/UpdaterProcess.java:18-19
pub const EXIT_OK: i32 = 0;

/// Exit code: the check or the update failed.
pub const EXIT_FAILURE: i32 = 1;

/// Exit code: no newer release.
pub const EXIT_NO_UPDATE: i32 = 2;

/// Line writer that tracks an open `\rDownloading: N%` line, so every other line starts on a
/// fresh line.
struct Printer<'a> {
    out: &'a mut (dyn Write + Send),
    /// `None`: errors go to `out` too.
    err: Option<&'a mut (dyn Write + Send)>,
    progress_open: bool,
    last_percent: Option<u64>,
    installing: bool,
}

impl<'a> Printer<'a> {
    fn new(out: &'a mut (dyn Write + Send), err: Option<&'a mut (dyn Write + Send)>) -> Self {
        Self {
            out,
            err,
            progress_open: false,
            last_percent: None,
            installing: false,
        }
    }

    fn close_progress(&mut self) {
        if std::mem::take(&mut self.progress_open) {
            let _ignored: std::io::Result<()> = writeln!(self.out);
        }
    }

    fn out(&mut self, line: &str) {
        self.close_progress();
        let _ignored: std::io::Result<()> = writeln!(self.out, "{line}");
        let _ignored: std::io::Result<()> = self.out.flush();
    }

    fn err(&mut self, line: &str) {
        self.close_progress();
        let writer: &mut (dyn Write + Send) = match self.err.as_deref_mut() {
            Some(err) => err,
            None => &mut *self.out,
        };
        let _ignored: std::io::Result<()> = writeln!(writer, "{line}");
        let _ignored: std::io::Result<()> = writer.flush();
    }

    // Mirrors a3sUpdater/console/Console.java:43-49,101-105
    fn event(&mut self, event: UpdateEvent, version: &str) {
        match event {
            UpdateEvent::Preparing => self.out(&text::updating(version)),
            UpdateEvent::Downloading { bytes, total } => {
                let Some(percent) = text::progress_percent(bytes, total) else {
                    return;
                };
                if self.last_percent != Some(percent) {
                    self.last_percent = Some(percent);
                    self.progress_open = true;
                    let _ignored: std::io::Result<()> =
                        write!(self.out, "\r{}", text::downloading(percent));
                    let _ignored: std::io::Result<()> = self.out.flush();
                }
            }
            UpdateEvent::ElevatedInstallStarted => self.out(text::ELEVATED_CONTINUING),
            UpdateEvent::Installing if !self.installing => {
                self.installing = true;
                // One line end: it closes the progress line, or prints an empty line.
                self.progress_open = false;
                let _ignored: std::io::Result<()> = writeln!(self.out);
                self.out(text::PROCESSING);
            }
            _ => {}
        }
    }
}

/// Checks for an update and installs it. Exit codes: [`EXIT_OK`] updated, [`EXIT_NO_UPDATE`]
/// already up to date, [`EXIT_FAILURE`] failed.
// Mirrors a3sUpdater/console/Console.java:19-80
pub async fn run_console_update(
    session: &UpdateSession,
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
    cancel: &CancellationToken,
) -> i32 {
    let ctx = session.context();
    ctx.diag.info(&format!(
        "Console update started: source={:?}, dev={}",
        ctx.source, ctx.dev
    ));
    let printer = Mutex::new(Printer::new(out, Some(err)));
    match session.check(cancel).await {
        Ok(CheckOutcome::Available { release, .. }) => {
            apply_and_report(session, &release, &printer, cancel).await
        }
        Ok(CheckOutcome::UpToDate { .. }) => {
            printer.lock().out(text::NO_UPDATE);
            EXIT_NO_UPDATE
        }
        Err(error) => {
            ctx.diag.error("Console updater failed.", Some(&error));
            let mut printer = printer.lock();
            printer.err(&text::error_occurred(&error));
            printer.err(text::ABORTED);
            EXIT_FAILURE
        }
    }
}

/// Installs `release` with console output; [`EXIT_OK`] or [`EXIT_FAILURE`].
async fn apply_and_report(
    session: &UpdateSession,
    release: &Release,
    printer: &Mutex<Printer<'_>>,
    cancel: &CancellationToken,
) -> i32 {
    let ctx = session.context();
    let version = release.version.as_str();
    let on_event = |event: UpdateEvent| printer.lock().event(event, version);
    let outcome = session.apply(release, &on_event, cancel).await;
    let mut printer = printer.lock();
    match outcome {
        Ok(_) => {
            printer.out(&text::success_message(version));
            ctx.diag.info(&format!(
                "Console update completed successfully: targetVersion={version}"
            ));
            EXIT_OK
        }
        // Mirrors a3sUpdater/console/Console.java:38-41,65-68
        Err(error) if error.needs_admin() => {
            ctx.diag
                .error(&format!("Console UAC restart failed: {error}"), None);
            printer.err(text::ADMIN_REQUIRED);
            printer.err(&error.to_string());
            EXIT_FAILURE
        }
        // Mirrors a3sUpdater/console/Console.java:54-55,74-76
        Err(error) => {
            ctx.diag.error("Console updater failed.", Some(&error));
            printer.err(&text::error_occurred(&error));
            printer.err(text::ABORTED);
            EXIT_FAILURE
        }
    }
}

/// Only checks. Exit codes: [`EXIT_OK`] an update is available, [`EXIT_NO_UPDATE`] none,
/// [`EXIT_FAILURE`] the check failed.
// Mirrors a3sUpdater/console/Console.java:82-99
pub async fn run_check_only(
    session: &UpdateSession,
    out: &mut (dyn Write + Send),
    err: &mut (dyn Write + Send),
    cancel: &CancellationToken,
) -> i32 {
    let mut printer = Printer::new(out, Some(err));
    match session.check(cancel).await {
        Ok(CheckOutcome::Available { release, .. }) => {
            printer.out(&text::update_available(&release.version));
            EXIT_OK
        }
        Ok(CheckOutcome::UpToDate { .. }) => {
            printer.out(text::NO_UPDATE);
            EXIT_NO_UPDATE
        }
        Err(error) => {
            printer.err(&text::check_failed(&error));
            EXIT_FAILURE
        }
    }
}

/// The main application's legacy update command: everything goes to `out`, and the result is
/// always [`EXIT_OK`] (scripts calling `-update` rely on it).
// Mirrors console/CommandGeneral.java:567-589
pub async fn run_legacy_update_command(
    session: &UpdateSession,
    out: &mut (dyn Write + Send),
    cancel: &CancellationToken,
) -> i32 {
    let printer = Mutex::new(Printer::new(out, None));
    match session.check(cancel).await {
        Ok(CheckOutcome::Available { release, .. }) => {
            apply_and_report(session, &release, &printer, cancel).await;
        }
        Ok(CheckOutcome::UpToDate { .. }) => printer.lock().out(text::NO_UPDATE),
        Err(error) => printer.lock().out(&text::check_failed(&error)),
    }
    EXIT_OK
}

/// What the updater flags ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdaterMode {
    /// The updater window.
    Gui,

    /// Update on the console.
    Console,

    /// Check only, reported through the exit code.
    Check,
}

/// Parsed updater flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdaterFlags {
    pub dev: bool,
    pub source: UpdateSource,
    pub mode: UpdaterMode,
}

/// Invalid updater flags.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UsageError {
    /// `-github` and `-manifest` together.
    #[error("Choose either -github or -manifest, not both.")]
    ConflictingSource,

    /// An argument that is not an updater flag.
    #[error("ArmA3Sync-Updater - command not found.")]
    Unknown(String),
}

impl UsageError {
    /// The lines to print to stderr.
    // Mirrors a3sUpdater/main/ArmA3SyncUpdater.java:29-34,47-55
    pub fn lines(&self) -> Vec<&'static str> {
        match self {
            Self::ConflictingSource => vec!["Choose either -github or -manifest, not both."],
            Self::Unknown(_) => vec![
                "ArmA3Sync-Updater - command not found.",
                "Usage: arma3sync update [-dev] [-github|-manifest] [-console|-check]",
            ],
        }
    }

    pub fn exit_code(&self) -> i32 {
        EXIT_FAILURE
    }
}

/// Parses `-dev`, `-console`, `-check`, `-github` and `-manifest` (ASCII case-insensitive, any
/// order). `-console` wins over `-check`; neither means the window. Any other argument is an
/// error.
// Mirrors a3sUpdater/main/ArmA3SyncUpdater.java:24-56
pub fn parse_updater_flags(args: &[String]) -> Result<UpdaterFlags, UsageError> {
    let (mut dev, mut console, mut check, mut github, mut manifest) =
        (false, false, false, false, false);
    for arg in args {
        let flag = match arg.to_ascii_lowercase().as_str() {
            "-dev" => &mut dev,
            "-console" => &mut console,
            "-check" => &mut check,
            "-github" => &mut github,
            "-manifest" => &mut manifest,
            _ => return Err(UsageError::Unknown(arg.clone())),
        };
        *flag = true;
    }
    if github && manifest {
        return Err(UsageError::ConflictingSource);
    }
    let source = if github {
        UpdateSource::GitHub
    } else if manifest {
        UpdateSource::Manifest
    } else {
        UpdateSource::Configured
    };
    let mode = if console {
        UpdaterMode::Console
    } else if check {
        UpdaterMode::Check
    } else {
        UpdaterMode::Gui
    };
    Ok(UpdaterFlags { dev, source, mode })
}
