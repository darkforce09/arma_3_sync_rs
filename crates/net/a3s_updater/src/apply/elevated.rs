//! The elevated install: the unelevated application starts its own binary with administrator
//! rights and the hidden [`ELEVATED_SUBCOMMAND`], which installs the archive the parent already
//! downloaded, then waits for it.
//!
//! The child trusts nothing a user process can change. Its arguments only name the archive,
//! the requested version, the release channel and a nonce. It resolves the release itself
//! through the installation's own `updater.toml` (no user configuration, no environment
//! overrides), requires that release to be the requested version and newer than its own, and
//! verifies the archive against the SHA-256 it resolved. It writes its startup marker, result
//! and log only into a directory it creates inside the installation,
//! `.a3s-elevated-<pid>-<nonce>`, which a less-privileged user can read but not redirect; the
//! parent finds it by the nonce. It exits with 0 on success and 1 on failure.

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::hash::{BuildHasher as _, Hasher as _};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_foundation::paths::Os;

use super::install::install_verified_archive;
use super::replace::{ELEVATED_PREFIX, SelfReplacer, cleanup_leftovers};
use super::{ApplyError, UpdateContext, UpdateEvent, console};
use crate::elevation::{ElevatedLaunch, ElevatedLauncher, LaunchExit, is_windows};
use crate::{
    CancellationToken, ConfigOverrides, DiagnosticLog, HttpClient, Release, ResolveRequest,
    UpdateConfig, UpdateError, UpdateSource, host_target, is_newer, resolve_release,
};

/// First argument of the elevated updater process.
pub const ELEVATED_SUBCOMMAND: &str = "__a3s-elevated-install";

/// How long the parent waits for the elevated updater: answering the UAC prompt plus the
/// install.
pub const ELEVATED_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// How often the parent looks for the startup marker.
const MARKER_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Most bytes of the result file the parent reads.
const MAX_RESULT_BYTES: u64 = 64 * 1024;

/// Files of the child's work directory.
const MARKER_NAME: &str = "elevated.startup";
const RESULT_NAME: &str = "elevated.result";
const LOG_NAME: &str = "elevated-updater.log";

/// Hex digits of a nonce.
const NONCE_LENGTH: usize = 32;

const OPTION_ARCHIVE: &str = "--archive";
const OPTION_VERSION: &str = "--version";
const OPTION_NONCE: &str = "--nonce";
const OPTION_SOURCE: &str = "--source";
const FLAG_DEV: &str = "--dev";
const FLAG_DEBUG: &str = "--debug";

/// The arguments of the elevated updater. There is no installation directory and no checksum:
/// the child installs next to its own executable and resolves the checksum itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElevatedInstallArgs {
    /// The downloaded archive (absolute).
    pub archive: PathBuf,

    /// The version the parent downloaded; the child installs only this version.
    pub version: String,

    /// Names the child's work directory (32 lowercase hex digits).
    pub nonce: String,

    /// Resolve the development channel.
    pub dev: bool,
    pub source: UpdateSource,

    /// Write a diagnostic log into the work directory.
    pub debug: bool,
}

impl ElevatedInstallArgs {
    /// A new random nonce.
    pub fn new_nonce() -> String {
        let mut nonce = String::with_capacity(NONCE_LENGTH);
        // `RandomState` keys come from the operating system's random source.
        for part in 0..2u8 {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u8(part);
            hasher.write_u32(std::process::id());
            nonce.push_str(&format!("{:016x}", hasher.finish()));
        }
        nonce
    }

    /// The full argument list, starting with [`ELEVATED_SUBCOMMAND`]; fails for an archive
    /// path that is not valid UTF-8.
    // Mirrors a3sUpdater/service/ElevationSupport.java:95-121
    pub fn to_args(&self) -> Result<Vec<String>, String> {
        let archive = self.archive.to_str().ok_or_else(|| {
            format!(
                "{OPTION_ARCHIVE} {} is not valid UTF-8.",
                self.archive.display()
            )
        })?;
        let mut args = vec![
            ELEVATED_SUBCOMMAND.to_owned(),
            OPTION_ARCHIVE.to_owned(),
            archive.to_owned(),
            OPTION_VERSION.to_owned(),
            self.version.clone(),
            OPTION_NONCE.to_owned(),
            self.nonce.clone(),
            OPTION_SOURCE.to_owned(),
            source_name(self.source).to_owned(),
        ];
        for (set, flag) in [(self.dev, FLAG_DEV), (self.debug, FLAG_DEBUG)] {
            if set {
                args.push(flag.to_owned());
            }
        }
        Ok(args)
    }

    /// Parses the arguments after [`ELEVATED_SUBCOMMAND`]: every option and flag at most once
    /// and in any order; `--archive`, `--version`, `--nonce` and `--source` are required. The
    /// archive must be absolute, the version non-blank and the nonce 32 hex digits (stored
    /// lowercase).
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut archive = None;
        let mut version = None;
        let mut nonce = None;
        let mut source = None;
        let mut dev = false;
        let mut debug = false;
        let mut rest = args.iter();
        while let Some(option) = rest.next() {
            let flag = match option.as_str() {
                FLAG_DEV => Some(&mut dev),
                FLAG_DEBUG => Some(&mut debug),
                _ => None,
            };
            if let Some(flag) = flag {
                if std::mem::replace(flag, true) {
                    return Err(format!("{option} is given more than once"));
                }
                continue;
            }
            let slot = match option.as_str() {
                OPTION_ARCHIVE => &mut archive,
                OPTION_VERSION => &mut version,
                OPTION_NONCE => &mut nonce,
                OPTION_SOURCE => &mut source,
                other => return Err(format!("unknown option {other}")),
            };
            let value = rest
                .next()
                .ok_or_else(|| format!("{option} needs a value"))?;
            if slot.replace(value.as_str()).is_some() {
                return Err(format!("{option} is given more than once"));
            }
        }
        let archive = PathBuf::from(required(archive, OPTION_ARCHIVE)?);
        if !archive.is_absolute() {
            return Err(format!("{OPTION_ARCHIVE} must be an absolute path"));
        }
        let version = required(version, OPTION_VERSION)?;
        if version.trim().is_empty() {
            return Err(format!("{OPTION_VERSION} is blank"));
        }
        let nonce = required(nonce, OPTION_NONCE)?;
        if nonce.len() != NONCE_LENGTH || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "{OPTION_NONCE} must be {NONCE_LENGTH} hexadecimal digits"
            ));
        }
        let source = match required(source, OPTION_SOURCE)? {
            "configured" => UpdateSource::Configured,
            "github" => UpdateSource::GitHub,
            "manifest" => UpdateSource::Manifest,
            other => return Err(format!("unknown {OPTION_SOURCE} {other}")),
        };
        Ok(Self {
            archive,
            version: version.to_owned(),
            nonce: nonce.to_ascii_lowercase(),
            dev,
            source,
            debug,
        })
    }
}

fn source_name(source: UpdateSource) -> &'static str {
    match source {
        UpdateSource::Configured => "configured",
        UpdateSource::GitHub => "github",
        UpdateSource::Manifest => "manifest",
    }
}

fn required<'a>(value: Option<&'a str>, option: &str) -> Result<&'a str, String> {
    value.ok_or_else(|| format!("{option} is missing"))
}

/// Entry point of the elevated updater: the binary calls it with the arguments after
/// [`ELEVATED_SUBCOMMAND`] and its own version before any other setup, and exits with the
/// returned code.
pub fn run_elevated_install(args: &[OsString], current_version: &str) -> i32 {
    match super::current_exe_path() {
        Ok(exe) => run_elevated_install_with(args, &exe, Os::current(), current_version),
        Err(err) => fail_early(&format!(
            "The elevated updater cannot find its own executable: {err}"
        )),
    }
}

/// [`run_elevated_install`] for the executable `exe_path` of version `current_version` on
/// `os`: installs into the parent directory of `exe_path`. Returns 0 on success, 1 on failure.
// Mirrors a3sUpdater/main/ArmA3SyncUpdater.java:19-23
pub fn run_elevated_install_with(
    args: &[OsString],
    exe_path: &Path,
    os: Os,
    current_version: &str,
) -> i32 {
    let strings: Option<Vec<String>> = args.iter().map(|a| a.to_str().map(str::to_owned)).collect();
    let parsed = strings
        .as_deref()
        .ok_or_else(|| "an argument is not valid UTF-8".to_owned())
        .and_then(ElevatedInstallArgs::parse);
    let args = match parsed {
        Ok(args) => args,
        Err(detail) => {
            return fail_early(&format!("Invalid elevated updater arguments: {detail}"));
        }
    };
    let Some(install_dir) = exe_path.parent() else {
        return fail_early(&format!("{} has no parent directory.", exe_path.display()));
    };
    // Removes the work directories of earlier runs before this one is created.
    cleanup_leftovers(install_dir);
    let work = match create_work_dir(install_dir, &args.nonce) {
        Ok(work) => work,
        Err(err) => {
            return fail_early(&format!(
                "The elevated updater cannot create its work directory in {}: {err}",
                install_dir.display()
            ));
        }
    };
    let diag = if args.debug {
        DiagnosticLog::new(true, Some(work.join(LOG_NAME)))
    } else {
        DiagnosticLog::disabled()
    };
    diag.mark_elevated_startup(&work.join(MARKER_NAME));
    diag.info(&format!(
        "Elevated updater started: installation={}, version={}",
        install_dir.display(),
        args.version
    ));

    let result = work.join(RESULT_NAME);
    match install_requested(&args, install_dir, os, current_version, &diag) {
        Ok(()) => {
            write_result(&result, "");
            console::EXIT_OK
        }
        Err(err) => {
            diag.error("Elevated update failed.", Some(&err));
            if err.is_permission_failure() {
                diag.error(
                    "Elevated updater still cannot write to the installation; not retrying UAC.",
                    None,
                );
            }
            write_result(&result, &err.to_string());
            console::EXIT_FAILURE
        }
    }
}

/// Reports a failure that happened before the work directory existed.
fn fail_early(message: &str) -> i32 {
    let _ignored: io::Result<()> = writeln!(io::stderr(), "{message}");
    console::EXIT_FAILURE
}

/// Creates `<install_dir>/.a3s-elevated-<pid>-<nonce>`; fails when anything, a link or
/// junction included, already has that name.
fn create_work_dir(install_dir: &Path, nonce: &str) -> io::Result<PathBuf> {
    let dir = install_dir.join(format!("{ELEVATED_PREFIX}{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir)?;
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || is_link(&meta) {
        return Err(io::Error::other(format!(
            "{} is not a plain directory",
            dir.display()
        )));
    }
    Ok(dir)
}

#[cfg(windows)]
fn is_link(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// Resolves the release through the installation's configuration, checks it against the
/// request and installs the archive with the resolved checksum.
fn install_requested(
    args: &ElevatedInstallArgs,
    install_dir: &Path,
    os: Os,
    current_version: &str,
    diag: &DiagnosticLog,
) -> Result<(), ApplyError> {
    let release = resolve_as_installation(install_dir, args, diag)?;
    if release.version.trim() != args.version.trim() {
        return Err(ApplyError::ElevatedInstallFailed(format!(
            "The latest release is {}, not the requested {}.",
            release.version, args.version
        )));
    }
    if !is_newer(&release.version, current_version) {
        return Err(ApplyError::ElevatedInstallFailed(format!(
            "Release {} is not newer than the installed version {current_version}.",
            release.version
        )));
    }
    let sha256 = release.sha256.ok_or(ApplyError::Unverified {
        version: release.version,
    })?;
    install_verified_archive(
        &args.archive,
        &sha256,
        install_dir,
        os,
        &SelfReplacer::new(),
        diag,
    )
    .map(drop)
}

/// The latest release as the installation's own `updater.toml` describes it: no user
/// configuration directory and no environment overrides, which a less-privileged user could
/// change.
fn resolve_as_installation(
    install_dir: &Path,
    args: &ElevatedInstallArgs,
    diag: &DiagnosticLog,
) -> Result<Release, ApplyError> {
    let target = host_target().ok_or(ApplyError::NoTarget)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            ApplyError::Metadata(UpdateError::Io {
                context: "Could not start the update runtime".to_owned(),
                source: e,
            })
        })?;
    runtime.block_on(async {
        let config = UpdateConfig::load(install_dir, &ConfigOverrides::default())
            .map_err(ApplyError::Metadata)?;
        let http = HttpClient::new(&config).map_err(ApplyError::Metadata)?;
        let request = ResolveRequest {
            install_dir,
            dev: args.dev,
            preference: args.source,
            target,
        };
        resolve_release(&config, &http, &request, diag, &CancellationToken::new())
            .await
            .map_err(ApplyError::Metadata)
    })
}

fn write_result(path: &Path, message: &str) {
    if let Err(err) = std::fs::write(path, message) {
        tracing::warn!(path = %path.display(), error = %err, "cannot write the elevated updater result");
    }
}

/// The child's work directory for `nonce` in `install_dir`, once it exists.
fn find_work_dir(install_dir: &Path, nonce: &str) -> Option<PathBuf> {
    let suffix = format!("-{nonce}");
    std::fs::read_dir(install_dir)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(ELEVATED_PREFIX)
                && name.ends_with(&suffix)
                && entry.file_type().is_ok_and(|t| t.is_dir())
        })
        .map(|entry| entry.path())
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
    let nonce = ElevatedInstallArgs::new_nonce();
    let args = ElevatedInstallArgs {
        archive: std::path::absolute(archive).map_err(|e| not_started(&e))?,
        version: release.version.clone(),
        nonce: nonce.clone(),
        dev: ctx.dev,
        source: ctx.source,
        debug: ctx.diag.is_enabled(),
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
    let mut work: Option<PathBuf> = None;
    let mut started = false;
    let mut announce_start = || {
        if work.is_none() {
            work = find_work_dir(&ctx.install_dir, &nonce);
        }
        let marker = work.as_ref().map(|dir| dir.join(MARKER_NAME));
        if !started && marker.is_some_and(|marker| marker.is_file()) {
            started = true;
            on_event(UpdateEvent::ElevatedInstallStarted);
            on_event(UpdateEvent::Installing);
        }
        started
    };
    let joined = loop {
        tokio::select! {
            joined = &mut task => break joined,
            () = tokio::time::sleep(MARKER_POLL_INTERVAL) => {
                announce_start();
            }
        }
    };
    let started = announce_start();
    let exit = match joined {
        Ok(exit) => exit,
        Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
        Err(err) => Err(io::Error::other(err.to_string())),
    };

    let result = work.as_ref().map(|dir| dir.join(RESULT_NAME));
    let outcome = map_exit(exit, started, result.as_deref());
    match &outcome {
        Ok(()) => ctx
            .diag
            .info("UAC restart confirmed: elevated updater finished with exit=0"),
        Err(err) => ctx
            .diag
            .error(&format!("UAC restart rejected: {err}"), None),
    }
    if let Some(dir) = work.as_ref().filter(|_| ctx.diag.is_enabled()) {
        ctx.diag.info(&format!(
            "Elevated updater log: {}",
            dir.join(LOG_NAME).display()
        ));
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
    exit: io::Result<LaunchExit>,
    started: bool,
    result: Option<&Path>,
) -> Result<(), ApplyError> {
    match exit {
        Ok(LaunchExit::Exited(0)) => Ok(()),
        Ok(LaunchExit::Exited(_)) if !started => Err(ApplyError::ElevatedInstallFailed(
            "The elevated updater did not start.".to_owned(),
        )),
        Ok(LaunchExit::Exited(code)) => {
            let message = result.map(read_result).unwrap_or_default();
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
        let _ignored: io::Result<usize> = file.take(MAX_RESULT_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}
