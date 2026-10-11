//! Windows UAC elevation: classifying permission failures, probing whether the installation is
//! writable, and starting a program elevated through PowerShell `Start-Process -Verb RunAs`.

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use a3s_foundation::command_line::{encode_powershell_command, quote_powershell, windows_args};
use a3s_foundation::paths::Os;

use crate::DiagnosticLog;

/// Exit code of the PowerShell command when the elevated process could not be started, for
/// example because the UAC prompt was refused (Win32 `ERROR_CANCELLED`).
pub const NOT_STARTED_EXIT_CODE: i32 = 1223;

/// Error texts that mean "permission denied" (English and German Windows).
// Mirrors a3sUpdater/service/ElevationSupport.java:442-448
const PERMISSION_KEYWORDS: [&str; 3] = ["access denied", "permission denied", "zugriff verweigert"];

/// Whether `os` is Windows, the only system where elevation is requested.
// Mirrors a3sUpdater/service/ElevationSupport.java:45-49
pub fn is_windows(os: Os) -> bool {
    os == Os::Windows
}

/// Whether `err` or one of its sources is an [`io::Error`] that means missing permissions.
///
/// An `io::Error` counts when its kind is `PermissionDenied`, or when its message contains
/// `access denied`, `permission denied` or `zugriff verweigert`. On Windows a sharing or lock
/// violation (OS errors 32 and 33) never counts: a locked file is not a permission problem.
/// Other error types in the chain are only traversed, never matched by text.
// Mirrors a3sUpdater/service/ElevationSupport.java:51-60
pub fn is_permission_failure(err: &(dyn Error + 'static)) -> bool {
    let mut current = Some(err);
    while let Some(error) = current {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(io_permission_failure)
        {
            return true;
        }
        current = error.source();
    }
    false
}

fn io_permission_failure(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    if cfg!(windows) && matches!(err.raw_os_error(), Some(32 | 33)) {
        return false;
    }
    let text = err.to_string().to_lowercase();
    PERMISSION_KEYWORDS
        .iter()
        .any(|keyword| text.contains(keyword))
}

/// Whether the current process can create files in `dir`: a probe file
/// (`.a3s-write-probe-*.tmp`) is created and deleted. `false` when `dir` is not a directory.
// Mirrors a3sUpdater/service/ElevationSupport.java:199-217
pub fn can_write_installation(dir: &Path, diag: &DiagnosticLog) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = match tempfile::Builder::new()
        .prefix(".a3s-write-probe-")
        .suffix(".tmp")
        .tempfile_in(dir)
    {
        Ok(probe) => probe,
        Err(err) => {
            diag.info(&format!(
                "Installation write probe failed: path={}, reason={:?}",
                dir.display(),
                err.kind()
            ));
            return false;
        }
    };
    let path = probe.path().to_path_buf();
    if probe.close().is_err() {
        diag.info(&format!(
            "Installation write probe cleanup failed: {}",
            path.display()
        ));
    }
    true
}

/// Whether installing into `dir` needs UAC: only on Windows, only from a process that was not
/// started elevated by the updater, and only for an existing directory the process cannot write.
// Mirrors a3sUpdater/service/ElevationSupport.java:219-225
pub fn requires_elevation(os: Os, elevated: bool, dir: &Path, diag: &DiagnosticLog) -> bool {
    is_windows(os) && !elevated && dir.is_dir() && !can_write_installation(dir, diag)
}

/// A program to start with administrator rights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElevatedLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
}

impl ElevatedLaunch {
    /// The arguments as one Windows command line (MSVC runtime quoting).
    pub fn parameters(&self) -> String {
        windows_args(&self.args)
    }

    /// The PowerShell command that starts the program elevated and hidden, waits for it and
    /// exits with its exit code; a failed start (UAC refused) prints the reason to stderr and
    /// exits with [`NOT_STARTED_EXIT_CODE`].
    // Mirrors a3sUpdater/service/ElevationSupport.java:335-355,413-415
    pub fn powershell_command(&self) -> String {
        let parameters = self.parameters();
        // `-ArgumentList` rejects an empty string.
        let argument_list = if parameters.is_empty() {
            String::new()
        } else {
            format!(" -ArgumentList {}", quote_powershell(&parameters))
        };
        format!(
            "$ErrorActionPreference='Stop'; try {{ $p=Start-Process -FilePath {}{argument_list} \
             -WorkingDirectory {} -WindowStyle Hidden -Verb RunAs -PassThru }} catch {{ \
             [Console]::Error.WriteLine($_.Exception.Message); exit {NOT_STARTED_EXIT_CODE} }}; \
             $null=$p.Handle; $p.WaitForExit(); exit $p.ExitCode",
            quote_powershell(&self.program.to_string_lossy()),
            quote_powershell(&self.working_dir.to_string_lossy()),
        )
    }

    /// [`Self::powershell_command`] for `powershell.exe -EncodedCommand`.
    pub fn encoded_command(&self) -> String {
        encode_powershell_command(&self.powershell_command())
    }
}

/// How an elevated launch ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchExit {
    /// The elevated program ran and exited with this code.
    Exited(i32),

    /// The elevated program was never started (UAC refused, or no elevation on this system).
    NotStarted { detail: String },

    /// The elevated program did not finish within the timeout; it may still be running.
    TimedOut,
}

/// Starts a program elevated and waits for it.
pub trait ElevatedLauncher: Send + Sync {
    /// Runs `launch` elevated and waits at most `timeout` for it to exit.
    fn run(&self, launch: &ElevatedLaunch, timeout: Duration) -> io::Result<LaunchExit>;
}

/// Elevates through `powershell.exe -EncodedCommand` with [`ElevatedLaunch::powershell_command`]
/// (Windows only; elsewhere every launch is [`LaunchExit::NotStarted`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct PowerShellLauncher;

impl ElevatedLauncher for PowerShellLauncher {
    #[cfg(windows)]
    fn run(&self, launch: &ElevatedLaunch, timeout: Duration) -> io::Result<LaunchExit> {
        powershell::run(launch, timeout)
    }

    #[cfg(not(windows))]
    fn run(&self, _launch: &ElevatedLaunch, _timeout: Duration) -> io::Result<LaunchExit> {
        Ok(LaunchExit::NotStarted {
            detail: "UAC elevation is only available on Windows.".to_owned(),
        })
    }
}

#[cfg(windows)]
mod powershell {
    use std::io::{self, Read as _};
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::{ElevatedLaunch, LaunchExit, NOT_STARTED_EXIT_CODE};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    const MAX_DETAIL_BYTES: u64 = 4096;

    // Mirrors a3sUpdater/service/ElevationSupport.java:139-170
    pub(super) fn run(launch: &ElevatedLaunch, timeout: Duration) -> io::Result<LaunchExit> {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
            ])
            .arg(launch.encoded_command())
            .current_dir(&launch.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        // Drained on its own thread so a chatty stderr can never block PowerShell.
        let stderr = child.stderr.take();
        let reader = std::thread::Builder::new()
            .name("a3s-uac-stderr".to_owned())
            .spawn(move || {
                let mut detail = Vec::new();
                if let Some(mut stderr) = stderr {
                    let _ignored: io::Result<usize> = (&mut stderr)
                        .take(MAX_DETAIL_BYTES)
                        .read_to_end(&mut detail);
                    let _ignored: io::Result<u64> = io::copy(&mut stderr, &mut io::sink());
                }
                detail
            })?;

        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                // The elevated program is not a child of this PowerShell process and keeps
                // running.
                let _ignored: io::Result<()> = child.kill();
                let _ignored: io::Result<std::process::ExitStatus> = child.wait();
                break None;
            }
            std::thread::sleep(POLL_INTERVAL);
        };
        let detail = reader.join().unwrap_or_default();
        let Some(status) = status else {
            return Ok(LaunchExit::TimedOut);
        };
        Ok(match status.code().unwrap_or(-1) {
            NOT_STARTED_EXIT_CODE => LaunchExit::NotStarted {
                detail: String::from_utf8_lossy(&detail).trim().to_owned(),
            },
            code => LaunchExit::Exited(code),
        })
    }
}
