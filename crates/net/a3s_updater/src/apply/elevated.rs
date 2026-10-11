//! The elevated install: the unelevated application starts its own binary with administrator
//! rights and the hidden [`ELEVATED_SUBCOMMAND`], which only re-verifies and installs the
//! archive the parent already downloaded, then waits for it.
//!
//! Protocol: the parent passes the archive, its SHA-256, the version, a startup marker and a
//! result file (both in the parent's staging directory). The child writes the marker as soon as
//! it starts, installs into the directory of its own executable, writes an empty result on
//! success or the error message on failure, and exits with 0 or 1.

use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_foundation::paths::Os;

use super::install::install_verified_archive;
use super::replace::SelfReplacer;
use super::{ApplyError, UpdateContext, UpdateEvent, console};
use crate::elevation::{ElevatedLaunch, ElevatedLauncher, LaunchExit, is_windows};
use crate::{DiagnosticLog, Release};

/// First argument of the elevated updater process.
pub const ELEVATED_SUBCOMMAND: &str = "__a3s-elevated-install";

/// How long the parent waits for the elevated updater: answering the UAC prompt plus the
/// install.
pub const ELEVATED_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// How often the parent looks for the startup marker.
const MARKER_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Most bytes of the result file the parent reads.
const MAX_RESULT_BYTES: u64 = 64 * 1024;

const OPTION_ARCHIVE: &str = "--archive";
const OPTION_SHA256: &str = "--sha256";
const OPTION_VERSION: &str = "--version";
const OPTION_MARKER: &str = "--marker";
const OPTION_RESULT: &str = "--result";
const OPTION_DEBUG_LOG: &str = "--debug-log";

/// The arguments of the elevated updater. There is no installation directory: the child
/// installs next to its own executable, so it cannot be pointed at another directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElevatedInstallArgs {
    /// The downloaded archive (absolute).
    pub archive: PathBuf,

    /// Lowercase hex SHA-256 the archive must have.
    pub sha256: String,
    pub version: String,

    /// File the child writes as soon as it starts (absolute).
    pub marker: PathBuf,

    /// File the child writes its outcome to: empty on success, else the error (absolute).
    pub result: PathBuf,

    /// Diagnostic log the child appends to.
    pub debug_log: Option<PathBuf>,
}

impl ElevatedInstallArgs {
    /// The full argument list, starting with [`ELEVATED_SUBCOMMAND`]; fails for a path that is
    /// not valid UTF-8.
    // Mirrors a3sUpdater/service/ElevationSupport.java:95-121
    pub fn to_args(&self) -> Result<Vec<String>, String> {
        let path = |option: &str, path: &Path| {
            path.to_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{option} {} is not valid UTF-8.", path.display()))
        };
        let mut args = vec![
            ELEVATED_SUBCOMMAND.to_owned(),
            OPTION_ARCHIVE.to_owned(),
            path(OPTION_ARCHIVE, &self.archive)?,
            OPTION_SHA256.to_owned(),
            self.sha256.clone(),
            OPTION_VERSION.to_owned(),
            self.version.clone(),
            OPTION_MARKER.to_owned(),
            path(OPTION_MARKER, &self.marker)?,
            OPTION_RESULT.to_owned(),
            path(OPTION_RESULT, &self.result)?,
        ];
        if let Some(debug_log) = &self.debug_log {
            args.push(OPTION_DEBUG_LOG.to_owned());
            args.push(path(OPTION_DEBUG_LOG, debug_log)?);
        }
        Ok(args)
    }

    /// Parses the arguments after [`ELEVATED_SUBCOMMAND`]: every option once and in any order,
    /// `--debug-log` optional. Paths must be absolute, the SHA-256 64 hex digits (stored
    /// lowercase) and the version non-blank.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut archive = None;
        let mut sha256 = None;
        let mut version = None;
        let mut marker = None;
        let mut result = None;
        let mut debug_log = None;
        let mut rest = args.iter();
        while let Some(option) = rest.next() {
            let slot = match option.as_str() {
                OPTION_ARCHIVE => &mut archive,
                OPTION_SHA256 => &mut sha256,
                OPTION_VERSION => &mut version,
                OPTION_MARKER => &mut marker,
                OPTION_RESULT => &mut result,
                OPTION_DEBUG_LOG => &mut debug_log,
                other => return Err(format!("unknown option {other}")),
            };
            let value = rest
                .next()
                .ok_or_else(|| format!("{option} needs a value"))?;
            if slot.replace(value.as_str()).is_some() {
                return Err(format!("{option} is given more than once"));
            }
        }
        let sha256 = required(sha256, OPTION_SHA256)?;
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{OPTION_SHA256} must be 64 hexadecimal digits"));
        }
        let version = required(version, OPTION_VERSION)?;
        if version.trim().is_empty() {
            return Err(format!("{OPTION_VERSION} is blank"));
        }
        Ok(Self {
            archive: absolute(archive, OPTION_ARCHIVE)?,
            sha256: sha256.to_ascii_lowercase(),
            version: version.to_owned(),
            marker: absolute(marker, OPTION_MARKER)?,
            result: absolute(result, OPTION_RESULT)?,
            debug_log: debug_log
                .map(|value| absolute(Some(value), OPTION_DEBUG_LOG))
                .transpose()?,
        })
    }
}

fn required<'a>(value: Option<&'a str>, option: &str) -> Result<&'a str, String> {
    value.ok_or_else(|| format!("{option} is missing"))
}

fn absolute(value: Option<&str>, option: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(value, option)?);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("{option} must be an absolute path"))
    }
}

/// Entry point of the elevated updater: the binary calls it with the arguments after
/// [`ELEVATED_SUBCOMMAND`] before any other setup, and exits with the returned code.
pub fn run_elevated_install(args: &[OsString]) -> i32 {
    match super::current_exe_path() {
        Ok(exe) => run_elevated_install_with(args, &exe, Os::current()),
        Err(err) => {
            let _ignored: std::io::Result<()> = writeln!(
                std::io::stderr(),
                "The elevated updater cannot find its own executable: {err}"
            );
            console::EXIT_FAILURE
        }
    }
}

/// [`run_elevated_install`] for the executable `exe_path` on `os`: installs into the parent
/// directory of `exe_path`. Returns 0 on success, 1 on failure.
// Mirrors a3sUpdater/main/ArmA3SyncUpdater.java:19-23
pub fn run_elevated_install_with(args: &[OsString], exe_path: &Path, os: Os) -> i32 {
    let strings: Option<Vec<String>> = args.iter().map(|a| a.to_str().map(str::to_owned)).collect();
    let parsed = strings
        .as_deref()
        .ok_or_else(|| "an argument is not valid UTF-8".to_owned())
        .and_then(ElevatedInstallArgs::parse);
    let args = match parsed {
        Ok(args) => args,
        Err(detail) => {
            let message = format!("Invalid elevated updater arguments: {detail}");
            let _ignored: std::io::Result<()> = writeln!(std::io::stderr(), "{message}");
            if let Some(result) = strings.as_deref().and_then(result_option) {
                write_result(&result, &message);
            }
            return console::EXIT_FAILURE;
        }
    };

    let diag = match &args.debug_log {
        Some(path) => DiagnosticLog::new(true, Some(path.clone())),
        None => DiagnosticLog::disabled(),
    };
    diag.mark_elevated_startup(&args.marker);
    let Some(install_dir) = exe_path.parent() else {
        write_result(
            &args.result,
            &format!("{} has no parent directory.", exe_path.display()),
        );
        return console::EXIT_FAILURE;
    };
    diag.info(&format!(
        "Elevated updater started: installation={}, version={}",
        install_dir.display(),
        args.version
    ));

    match install_verified_archive(
        &args.archive,
        &args.sha256,
        install_dir,
        os,
        &SelfReplacer::new(),
        &diag,
    ) {
        Ok(_) => {
            write_result(&args.result, "");
            console::EXIT_OK
        }
        Err(err) => {
            if err.is_permission_failure() {
                diag.error(
                    "Elevated updater still cannot write to the installation; not retrying UAC.",
                    Some(&err),
                );
            }
            write_result(&args.result, &err.to_string());
            console::EXIT_FAILURE
        }
    }
}

/// The `--result` value of arguments that failed to parse, if it is an absolute path.
fn result_option(args: &[String]) -> Option<PathBuf> {
    let index = args.iter().position(|a| a == OPTION_RESULT)?;
    Some(PathBuf::from(args.get(index + 1)?)).filter(|path| path.is_absolute())
}

fn write_result(path: &Path, message: &str) {
    if let Err(err) = std::fs::write(path, message) {
        tracing::warn!(path = %path.display(), error = %err, "cannot write the elevated updater result");
    }
}

/// Parent side: starts the elevated updater for `archive` and waits for it.
///
/// Emits `WaitingForElevation`, then `ElevatedInstallStarted` and `Installing` once the child's
/// startup marker appears (also when it appears only after the child exited).
// Mirrors a3sUpdater/service/ElevationSupport.java:62-182
pub(super) async fn elevated_install(
    ctx: &UpdateContext,
    release: &Release,
    archive: &Path,
    staging: &Path,
    launcher: Arc<dyn ElevatedLauncher>,
    on_event: &(dyn Fn(UpdateEvent) + Send + Sync),
) -> Result<(), ApplyError> {
    if !is_windows(ctx.os) {
        return Err(ApplyError::ElevationUnavailable);
    }
    if !ctx.exe_path.is_file() {
        return Err(ApplyError::ElevationNotGranted(format!(
            "The Arma3Sync executable was not found: {}.",
            ctx.exe_path.display()
        )));
    }
    let sha256 = release
        .sha256
        .clone()
        .ok_or_else(|| ApplyError::Unverified {
            version: release.version.clone(),
        })?;
    let staging = std::path::absolute(staging).map_err(|e| not_started(&e))?;
    let marker = staging.join("elevated.startup");
    let result = staging.join("elevated.result");
    let args = ElevatedInstallArgs {
        archive: std::path::absolute(archive).map_err(|e| not_started(&e))?,
        sha256,
        version: release.version.clone(),
        marker: marker.clone(),
        result: result.clone(),
        debug_log: ctx
            .diag
            .is_enabled()
            .then(|| ctx.diag.path().map(Path::to_path_buf))
            .flatten(),
    }
    .to_args()
    .map_err(|detail| not_started(&detail))?;
    ctx.diag.info(&format!(
        "UAC restart requested: exe={}, installation={}",
        ctx.exe_path.display(),
        ctx.install_dir.display()
    ));

    on_event(UpdateEvent::WaitingForElevation);
    let launch = ElevatedLaunch {
        program: ctx.exe_path.clone(),
        args,
        working_dir: ctx.install_dir.clone(),
    };
    let mut task =
        tokio::task::spawn_blocking(move || launcher.run(&launch, ELEVATED_INSTALL_TIMEOUT));
    let mut started = false;
    let mut announce_start = || {
        if !started && marker.exists() {
            started = true;
            on_event(UpdateEvent::ElevatedInstallStarted);
            on_event(UpdateEvent::Installing);
        }
    };
    let joined = loop {
        tokio::select! {
            joined = &mut task => break joined,
            () = tokio::time::sleep(MARKER_POLL_INTERVAL) => announce_start(),
        }
    };
    announce_start();
    let exit = match joined {
        Ok(exit) => exit,
        Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
        Err(err) => Err(std::io::Error::other(err.to_string())),
    };

    let outcome = map_exit(exit, marker.exists(), &result);
    match &outcome {
        Ok(()) => ctx
            .diag
            .info("UAC restart confirmed: elevated updater finished with exit=0"),
        Err(err) => ctx
            .diag
            .error(&format!("UAC restart rejected: {err}"), None),
    }
    outcome
}

fn not_started(detail: &dyn std::fmt::Display) -> ApplyError {
    ApplyError::ElevationNotGranted(format!(
        "The elevated updater could not be started: {detail}"
    ))
}

/// Maps how the elevated updater ended to the result of the update.
// Mirrors a3sUpdater/service/ElevationSupport.java:153-170
fn map_exit(
    exit: std::io::Result<LaunchExit>,
    marker_exists: bool,
    result: &Path,
) -> Result<(), ApplyError> {
    match exit {
        Ok(LaunchExit::Exited(0)) => Ok(()),
        Ok(LaunchExit::Exited(_)) if !marker_exists => Err(ApplyError::ElevatedInstallFailed(
            "The elevated updater did not start.".to_owned(),
        )),
        Ok(LaunchExit::Exited(code)) => {
            let message = read_result(result);
            Err(ApplyError::ElevatedInstallFailed(if message.is_empty() {
                format!("The elevated updater failed with exit code {code}.")
            } else {
                message
            }))
        }
        Ok(LaunchExit::NotStarted { detail }) => {
            let mut message = "Administrator rights were not granted or the elevated updater \
                               could not be started."
                .to_owned();
            if !detail.trim().is_empty() {
                message.push(' ');
                message.push_str(detail.trim());
            }
            Err(ApplyError::ElevationNotGranted(message))
        }
        Ok(LaunchExit::TimedOut) => Err(ApplyError::ElevatedInstallFailed(
            "The elevated updater did not finish in time.".to_owned(),
        )),
        Err(err) => Err(not_started(&err)),
    }
}

/// The child's result message, trimmed (at most 64 KiB, invalid UTF-8 replaced).
fn read_result(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ignored: std::io::Result<usize> = file.take(MAX_RESULT_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}
