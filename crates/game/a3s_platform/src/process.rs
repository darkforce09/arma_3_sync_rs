//! "Is this program running?" probes: `tasklist` on Windows, `/proc` on Linux (which also sees
//! Wine and Proton processes).

use std::fs;
use std::io;
use std::path::PathBuf;

/// Arma 3 client executables.
// Mirrors service/LaunchService.java:94-127 and constant/GameExecutables.java:5-6
pub const ARMA3_EXES: [&str; 2] = ["arma3.exe", "arma3_x64.exe"];

/// Arma 3 dedicated server executables.
pub const ARMA3_SERVER_EXES: [&str; 2] = ["arma3server.exe", "arma3server_x64.exe"];

/// The Steam client executable.
// Mirrors service/LaunchService.java:470
pub const STEAM_EXE: &str = "steam.exe";

/// `TeamSpeak` 3 client executables.
pub const TS3_EXES: [&str; 2] = ["ts3client_win32.exe", "ts3client_win64.exe"];

/// Length at which Linux truncates a process's `comm` name.
const COMM_MAX_BYTES: usize = 15;

/// The running processes at one moment, by image name (compared ASCII case-insensitively).
#[derive(Clone, Debug, Default)]
pub struct ProcessSnapshot {
    /// Full image names, lowercase.
    names: Vec<String>,

    /// Linux `comm` names, lowercase; possibly cut at 15 bytes.
    comms: Vec<String>,
}

impl ProcessSnapshot {
    /// A snapshot of full image names.
    pub fn from_names<I: IntoIterator<Item = String>>(names: I) -> Self {
        Self {
            names: names
                .into_iter()
                .map(|name| name.to_ascii_lowercase())
                .collect(),
            comms: Vec::new(),
        }
    }

    /// Whether a process with image name `name` runs. A Linux `comm` name of exactly 15 bytes also
    /// matches a longer `name` that starts with it.
    pub fn is_running(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.names.contains(&name)
            || self.comms.iter().any(|comm| {
                *comm == name
                    || (comm.len() == COMM_MAX_BYTES
                        && name.as_bytes().starts_with(comm.as_bytes()))
            })
    }

    /// Whether any of `names` runs.
    pub fn is_any_running(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.is_running(name))
    }
}

/// Takes process snapshots.
pub trait ProcessProbe: Send + Sync {
    /// The running processes.
    fn snapshot(&self) -> io::Result<ProcessSnapshot>;
}

/// Whether `name` runs; `false` if the probe fails (Java swallowed the error).
// Mirrors dao/LauncherDAO.java:28-54
pub fn is_running(probe: &dyn ProcessProbe, name: &str) -> bool {
    is_any_running(probe, &[name])
}

/// Whether any of `names` runs, from one snapshot; `false` if the probe fails.
pub fn is_any_running(probe: &dyn ProcessProbe, names: &[&str]) -> bool {
    match probe.snapshot() {
        Ok(snapshot) => snapshot.is_any_running(names),
        Err(err) => {
            tracing::debug!("cannot list running processes: {err}");
            false
        }
    }
}

/// Whether the Arma 3 client runs.
pub fn is_arma3_running(probe: &dyn ProcessProbe) -> bool {
    is_any_running(probe, &ARMA3_EXES)
}

/// Whether an Arma 3 dedicated server runs.
pub fn is_arma3_server_running(probe: &dyn ProcessProbe) -> bool {
    is_any_running(probe, &ARMA3_SERVER_EXES)
}

/// Whether the Steam client runs.
pub fn is_steam_running(probe: &dyn ProcessProbe) -> bool {
    is_running(probe, STEAM_EXE)
}

/// Whether the `TeamSpeak` 3 client runs.
pub fn is_ts3_running(probe: &dyn ProcessProbe) -> bool {
    is_any_running(probe, &TS3_EXES)
}

/// The image names (first column) of `tasklist /FO CSV /NH` output.
pub fn parse_tasklist_csv(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(first_csv_field)
        .collect()
}

/// The first field of a CSV line, unquoted (`""` inside quotes is one `"`).
fn first_csv_field(line: &str) -> String {
    let Some(quoted) = line.strip_prefix('"') else {
        return line.split(',').next().unwrap_or_default().to_owned();
    };
    let mut field = String::new();
    let mut chars = quoted.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
            } else {
                break;
            }
        }
        field.push(c);
    }
    field
}

/// The probe of this OS: `tasklist` on Windows, `/proc` on Linux; unsupported elsewhere.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProcessProbe;

impl ProcessProbe for SystemProcessProbe {
    fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        #[cfg(windows)]
        {
            tasklist_snapshot()
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            ProcFsProbe::default().snapshot()
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
        {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

/// Runs `%SystemRoot%\System32\tasklist.exe /FO CSV /NH` without a console window. CSV output
/// keeps image names whole (the table output cuts them at 25 chars).
#[cfg(windows)]
fn tasklist_snapshot() -> io::Result<ProcessSnapshot> {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let windows_dir = ["SystemRoot", "windir"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|dir| !dir.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let output = std::process::Command::new(windows_dir.join("System32").join("tasklist.exe"))
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "tasklist exited with {}",
            output.status
        )));
    }
    Ok(ProcessSnapshot::from_names(parse_tasklist_csv(
        &String::from_utf8_lossy(&output.stdout),
    )))
}

/// A Linux `/proc` scan.
///
/// Each process contributes its `comm` name and the last component of its `argv[0]` (split on
/// `/` and `\`, so Wine paths like `Z:\…\arma3_x64.exe` work). Processes that exit or cannot be
/// read during the scan are skipped.
#[derive(Clone, Debug)]
pub struct ProcFsProbe {
    root: PathBuf,
}

impl ProcFsProbe {
    /// A probe of the `proc` file system mounted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Default for ProcFsProbe {
    fn default() -> Self {
        Self::new("/proc")
    }
}

impl ProcessProbe for ProcFsProbe {
    fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        let mut snapshot = ProcessSnapshot::default();
        for entry in fs::read_dir(&self.root)? {
            let Ok(entry) = entry else { continue };
            let file_name = entry.file_name();
            let is_pid = file_name
                .to_str()
                .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()));
            if !is_pid {
                continue;
            }
            let dir = entry.path();
            if let Ok(comm) = fs::read(dir.join("comm")) {
                let comm = String::from_utf8_lossy(&comm);
                let comm = comm.strip_suffix('\n').unwrap_or(&comm);
                if !comm.is_empty() {
                    snapshot.comms.push(comm.to_ascii_lowercase());
                }
            }
            if let Ok(cmdline) = fs::read(dir.join("cmdline")) {
                let argv0 = cmdline.split(|&b| b == 0).next().unwrap_or_default();
                let argv0 = String::from_utf8_lossy(argv0);
                let base = argv0.rsplit(['/', '\\']).next().unwrap_or_default();
                if !base.is_empty() {
                    snapshot.names.push(base.to_ascii_lowercase());
                }
            }
        }
        Ok(snapshot)
    }
}

/// A probe with a fixed answer, for tests: `None` fails every snapshot.
#[derive(Clone, Debug, Default)]
pub struct FixedProcessProbe(pub Option<Vec<String>>);

impl ProcessProbe for FixedProcessProbe {
    fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        self.0
            .clone()
            .map(ProcessSnapshot::from_names)
            .ok_or_else(|| io::Error::other("process listing failed"))
    }
}
