//! Checking for and applying an update to the running application.
//!
//! The update session with progress events, self-replacement of the binary, the UAC hand-off
//! on Windows, the console flows, the relaunch and the texts a GUI shows.

pub mod console;
mod elevated;
mod error;
pub mod install;
pub mod relaunch;
pub mod replace;
pub mod text;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_foundation::paths::Os;
use parking_lot::Mutex;

pub use elevated::{
    ELEVATED_INSTALL_TIMEOUT, ELEVATED_SUBCOMMAND, ElevatedInstallArgs, run_elevated_install,
    run_elevated_install_with,
};
pub use error::ApplyError;
use install::{InstallKind, detect_install_kind, install_verified_archive};
use relaunch::RelaunchPlan;
use replace::SelfReplacer;

use crate::elevation::{
    ElevatedLauncher, PowerShellLauncher, can_write_installation, is_windows, requires_elevation,
};
use crate::{
    CancellationToken, ConfigOverrides, DiagnosticLog, DownloadOptions, FileReplacer, HttpClient,
    InstallReport, Release, ResolveRequest, StagingDir, UpdateConfig, UpdateError, UpdateSource,
    download_release, host_target, is_newer, resolve_release,
};

/// Everything an update needs to know about the running application.
#[derive(Clone, Debug)]
pub struct UpdateContext {
    /// The running executable, resolved once at startup (on Unix through symbolic links): after
    /// the binary was replaced, the operating system no longer reports its path.
    pub exe_path: PathBuf,
    pub install_dir: PathBuf,
    pub current_version: String,

    /// Use the development release channel.
    pub dev: bool,
    pub source: UpdateSource,
    pub overrides: ConfigOverrides,

    /// Whether this process is the elevated updater; running the application as administrator
    /// by hand does not count.
    pub elevated: bool,
    pub os: Os,

    /// Rust target triple of the release archives for this build.
    pub target: Option<&'static str>,

    /// The `APPIMAGE` environment variable.
    pub appimage_env: Option<String>,
    pub diag: DiagnosticLog,

    /// Where the download staging directory is created; `None` = the system temporary
    /// directory.
    pub staging_parent: Option<PathBuf>,
}

impl UpdateContext {
    /// The context of this process: its executable and directory, `A3S_UPDATER_*` overrides
    /// (`user_config_dir` applies unless `A3S_UPDATER_USER_CONFIG_PATH` is set), the host OS and
    /// target, and `APPIMAGE`.
    // Mirrors service/UpdaterProcess.java:49-67
    pub fn for_current_process(
        current_version: &str,
        dev: bool,
        source: UpdateSource,
        user_config_dir: Option<PathBuf>,
        diag: DiagnosticLog,
    ) -> io::Result<Self> {
        let exe_path = current_exe_path()?;
        let mut ctx = Self::new(
            exe_path.clone(),
            parent_dir(&exe_path)?,
            current_version,
            Os::current(),
        );
        ctx.overrides = ConfigOverrides::from_env();
        if ctx.overrides.user_config_dir.is_none() {
            ctx.overrides.user_config_dir = user_config_dir;
        }
        ctx.dev = dev;
        ctx.source = source;
        ctx.appimage_env = std::env::var("APPIMAGE").ok();
        ctx.diag = diag;
        Ok(ctx)
    }

    /// A context with explicit paths, version and OS; everything else at its default.
    pub fn new(exe_path: PathBuf, install_dir: PathBuf, current_version: &str, os: Os) -> Self {
        Self {
            exe_path,
            install_dir,
            current_version: current_version.to_owned(),
            dev: false,
            source: UpdateSource::Configured,
            overrides: ConfigOverrides::default(),
            elevated: false,
            os,
            target: host_target(),
            appimage_env: None,
            diag: DiagnosticLog::disabled(),
            staging_parent: None,
        }
    }
}

/// The running executable; canonical on Unix (Windows keeps the path without a `\\?\` prefix).
pub(crate) fn current_exe_path() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if cfg!(unix) {
        exe.canonicalize()
    } else {
        Ok(exe)
    }
}

fn parent_dir(path: &Path) -> io::Result<PathBuf> {
    path.parent().map(Path::to_path_buf).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} has no parent directory", path.display()),
        )
    })
}

/// The update source for the "prefer GitHub releases" preference.
// Mirrors service/UpdaterProcess.java:58,65-67
pub fn source_for_preference(prefer_github: bool) -> UpdateSource {
    if prefer_github {
        UpdateSource::GitHub
    } else {
        UpdateSource::Manifest
    }
}

/// Result of [`UpdateSession::check`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// `release` is newer than the running version `current`.
    Available { release: Release, current: String },

    /// The running version `current` is the latest or newer.
    UpToDate { latest: Release, current: String },
}

/// Progress of [`UpdateSession::apply`].
///
/// In this order: `Preparing`, `Downloading` per chunk, then `Installing`, or with UAC `WaitingForElevation`, `ElevatedInstallStarted` (once the
/// elevated updater runs) and `Installing`; `Done` last. Cancelling is possible until
/// `Installing` or `WaitingForElevation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateEvent {
    Preparing,

    /// `bytes` downloaded of `total` (unknown when the server did not say).
    Downloading {
        bytes: u64,
        total: Option<u64>,
    },
    WaitingForElevation,
    ElevatedInstallStarted,
    Installing,
    Done,
}

/// A successful update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub version: String,
    pub install_dir: PathBuf,

    /// Whether the elevated updater installed it.
    pub elevated: bool,

    /// What the installation changed (`None` after an elevated install).
    pub report: Option<InstallReport>,

    /// How to start the updated application.
    pub relaunch: RelaunchPlan,
}

/// How [`UpdateSession::apply`] will install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preflight {
    /// This process writes the installation.
    InProcess,

    /// The installation needs UAC; the elevated updater installs after the download.
    NeedsElevation,
}

/// Checks before any download whether the update can be applied and how.
pub fn preflight(ctx: &UpdateContext, release: &Release) -> Result<Preflight, ApplyError> {
    if ctx.target.is_none() {
        return Err(ApplyError::NoTarget);
    }
    let kind = detect_install_kind(&ctx.exe_path, ctx.os, ctx.appimage_env.as_deref());
    if kind != InstallKind::Directory {
        return Err(ApplyError::Unsupported { kind });
    }
    if release.sha256.is_none() {
        return Err(ApplyError::Unverified {
            version: release.version.clone(),
        });
    }
    if can_write_installation(&ctx.install_dir, &ctx.diag) {
        return Ok(Preflight::InProcess);
    }
    if requires_elevation(ctx.os, ctx.elevated, &ctx.install_dir, &ctx.diag) {
        ctx.diag
            .info("Preflight requires elevation; UAC will be requested after the download.");
        return Ok(Preflight::NeedsElevation);
    }
    if !is_windows(ctx.os) && !ctx.elevated {
        return Err(ApplyError::ElevationUnavailable);
    }
    // Elevated and still not writable: the install fails and reports why.
    Ok(Preflight::InProcess)
}

/// Checks for and applies updates of one application; one `apply` at a time.
pub struct UpdateSession {
    ctx: UpdateContext,
    client: Mutex<Option<(UpdateConfig, HttpClient)>>,
    latest: Mutex<Option<Release>>,
    busy: AtomicBool,
    launcher: Arc<dyn ElevatedLauncher>,
    replacer: Arc<dyn FileReplacer + Send + Sync>,
}

/// Clears the busy flag of a session when an `apply` ends.
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl UpdateSession {
    /// A session that elevates through PowerShell and replaces files with [`SelfReplacer`].
    pub fn new(ctx: UpdateContext) -> Self {
        Self {
            ctx,
            client: Mutex::new(None),
            latest: Mutex::new(None),
            busy: AtomicBool::new(false),
            launcher: Arc::new(PowerShellLauncher),
            replacer: Arc::new(SelfReplacer::new()),
        }
    }

    /// Uses `launcher` to start the elevated updater.
    #[must_use]
    pub fn with_launcher(mut self, launcher: Arc<dyn ElevatedLauncher>) -> Self {
        self.launcher = launcher;
        self
    }

    /// Uses `replacer` to write files during an in-process install.
    #[must_use]
    pub fn with_replacer(mut self, replacer: Arc<dyn FileReplacer + Send + Sync>) -> Self {
        self.replacer = replacer;
        self
    }

    pub fn context(&self) -> &UpdateContext {
        &self.ctx
    }

    /// The configuration and HTTP client, loaded on first use.
    fn client(&self) -> Result<(UpdateConfig, HttpClient), ApplyError> {
        let mut client = self.client.lock();
        if let Some(loaded) = client.as_ref() {
            return Ok(loaded.clone());
        }
        let config = UpdateConfig::load(&self.ctx.install_dir, &self.ctx.overrides)
            .map_err(ApplyError::Metadata)?;
        let http = HttpClient::new(&config).map_err(ApplyError::Metadata)?;
        *client = Some((config.clone(), http.clone()));
        Ok((config, http))
    }

    /// Finds the latest release and compares it with the running version. The release is
    /// cached: later checks of the session do no network I/O.
    // Mirrors a3sUpdater/service/Service.java:39-46,70-83
    pub async fn check(&self, cancel: &CancellationToken) -> Result<CheckOutcome, ApplyError> {
        let ctx = &self.ctx;
        let target = ctx.target.ok_or(ApplyError::NoTarget)?;
        let cached = self.latest.lock().clone();
        let release = if let Some(release) = cached {
            release
        } else {
            let (config, http) = self.client()?;
            let request = ResolveRequest {
                install_dir: &ctx.install_dir,
                dev: ctx.dev,
                preference: ctx.source,
                target,
            };
            let release = resolve_release(&config, &http, &request, &ctx.diag, cancel)
                .await
                .map_err(|e| match e {
                    UpdateError::Cancelled => ApplyError::Cancelled,
                    other => ApplyError::Metadata(other),
                })?;
            *self.latest.lock() = Some(release.clone());
            release
        };
        let available = is_newer(&release.version, &ctx.current_version);
        ctx.diag.info(&format!(
            "Update comparison: current={}, target={}, available={available}",
            ctx.current_version, release.version
        ));
        let current = ctx.current_version.clone();
        Ok(if available {
            CheckOutcome::Available { release, current }
        } else {
            CheckOutcome::UpToDate {
                latest: release,
                current,
            }
        })
    }

    /// Downloads, verifies and installs `release`.
    ///
    /// Order: [`preflight`] (no network), size, download with SHA-256 verification, a last
    /// cancellation check, then the install: in this process, or through the elevated updater
    /// when the installation needs UAC. An in-process install that fails for lack of
    /// permissions on Windows is retried once through the elevated updater. The downloaded
    /// archive is deleted on every path. Install, elevation wait and relaunch cannot be
    /// cancelled.
    // Mirrors a3sUpdater/service/Service.java:96-195, a3sUpdater/console/Console.java:43-79
    pub async fn apply(
        &self,
        release: &Release,
        on_event: &(dyn Fn(UpdateEvent) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ApplyOutcome, ApplyError> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(ApplyError::Busy);
        }
        let _busy = BusyGuard(&self.busy);
        let ctx = &self.ctx;
        let plan = preflight(ctx, release)?;
        on_event(UpdateEvent::Preparing);
        let (_, http) = self.client()?;
        let total = if release.size > 0 {
            Some(release.size)
        } else {
            http.content_length(&release.download_url, cancel)
                .await
                .map_err(|e| match e {
                    UpdateError::Cancelled => ApplyError::Cancelled,
                    other => ApplyError::SizeUnknown(other),
                })?
        };
        ctx.diag.info(&format!(
            "Preparing update download: file={}, transport=HTTP",
            release.file_name
        ));

        let staging = match &ctx.staging_parent {
            Some(parent) => StagingDir::new_in(parent),
            None => StagingDir::new(),
        }
        .map_err(|e| {
            ApplyError::Download(UpdateError::Io {
                context: "Could not create the update download directory".to_owned(),
                source: e,
            })
        })?;
        ctx.diag.info(&format!(
            "Downloading update: file={}, size={}, transport=HTTP",
            release.file_name,
            total.map_or_else(|| "unknown".to_owned(), |t| t.to_string())
        ));
        let progress = |bytes: u64| on_event(UpdateEvent::Downloading { bytes, total });
        let archive = download_release(
            &http,
            release,
            &staging,
            DownloadOptions::default(),
            &progress,
            cancel,
        )
        .await
        .map_err(|e| match e {
            UpdateError::Cancelled => ApplyError::Cancelled,
            other => ApplyError::Download(other),
        })?;
        ctx.diag.info(&format!(
            "Update download completed: file={}",
            release.file_name
        ));
        // Mirrors a3sUpdater/ui/Updater.java:88
        if cancel.is_cancelled() {
            return Err(ApplyError::Cancelled);
        }

        let report = match plan {
            Preflight::NeedsElevation => None,
            Preflight::InProcess => {
                on_event(UpdateEvent::Installing);
                match self.install_in_process(&archive.path, release).await {
                    Ok(report) => Some(report),
                    Err(err) if err.is_permission_failure() => {
                        self.permission_fallback(err)?;
                        None
                    }
                    Err(err) => return Err(err),
                }
            }
        };
        let elevated = report.is_none();
        if elevated {
            elevated::elevated_install(
                ctx,
                release,
                &archive.path,
                staging.path(),
                Arc::clone(&self.launcher),
                on_event,
            )
            .await?;
        }
        drop(staging);
        on_event(UpdateEvent::Done);
        Ok(ApplyOutcome {
            version: release.version.clone(),
            install_dir: ctx.install_dir.clone(),
            elevated,
            report,
            relaunch: RelaunchPlan::for_context(ctx),
        })
    }

    /// Decides what follows an in-process install that failed for lack of permissions: `Ok`
    /// means "retry through the elevated updater".
    // Mirrors a3sUpdater/console/Console.java:54-76, a3sUpdater/ui/Updater.java:130-155
    fn permission_fallback(&self, err: ApplyError) -> Result<(), ApplyError> {
        let ctx = &self.ctx;
        if ctx.elevated {
            ctx.diag.error(
                "Elevated updater still cannot write to the installation; not retrying UAC.",
                Some(&err),
            );
            Err(err)
        } else if is_windows(ctx.os) {
            ctx.diag
                .info("Install failed with a permission error; requesting UAC.");
            Ok(())
        } else {
            Err(ApplyError::ElevationUnavailable)
        }
    }

    async fn install_in_process(
        &self,
        archive: &Path,
        release: &Release,
    ) -> Result<InstallReport, ApplyError> {
        let ctx = &self.ctx;
        let archive = archive.to_path_buf();
        // `preflight` refused releases without a checksum.
        let sha256 = release.sha256.clone().unwrap_or_default();
        let install_dir = ctx.install_dir.clone();
        let os = ctx.os;
        let replacer = Arc::clone(&self.replacer);
        let diag = ctx.diag.clone();
        let joined = tokio::task::spawn_blocking(move || {
            install_verified_archive(&archive, &sha256, &install_dir, os, &*replacer, &diag)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(err) => Err(ApplyError::Install {
                dir: ctx.install_dir.clone(),
                source: UpdateError::Io {
                    context: "The installation task stopped".to_owned(),
                    source: io::Error::other(err.to_string()),
                },
            }),
        }
    }
}
