//! Rust wrapper around the Java fixture harness (`tools/java_fixture_harness`).
//!
//! Every call runs `bash tools/java_fixture_harness/run.sh <command> …` (or `build.sh`) and
//! interprets the harness exit codes: 0 ok, 1 verification failure, 2 usage error, 3 internal
//! error, 4 JDK or Java sources missing. Only stdout is parsed; stderr is kept for error messages.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Output;

/// The JDK major version the harness requires (`scripts/java_env.sh`).
pub const REQUIRED_JAVA_MAJOR: u32 = 25;

/// Location of the harness relative to the repository root.
pub const HARNESS_DIR: &str = "tools/java_fixture_harness";

/// Number of stderr lines kept in error messages.
const STDERR_TAIL_LINES: usize = 40;

/// Errors of the harness wrapper.
#[derive(Debug, thiserror::Error)]
pub enum HarnessError {
    /// `tools/java_fixture_harness/run.sh` does not exist below the repository root.
    #[error("tools/java_fixture_harness/run.sh not found below {0}")]
    NotFound(PathBuf),

    /// The harness exited with code 4: no JDK 25 or no Java sources.
    #[error(
        "JDK 25 or the Java sources are not available: {0} (source scripts/java_env.sh or set A3S_JAVA_HOME / A3S_JAVA_SRC)"
    )]
    NoJdk(String),

    /// `bash` could not be started.
    #[error("bash is required to run the Java harness")]
    NoBash,

    /// The harness only runs on Unix hosts.
    #[error("the Java harness is only supported on Unix hosts")]
    Unsupported,

    /// The harness rejected its arguments (exit code 2).
    #[error("harness usage error: {0}")]
    Usage(String),

    /// The harness failed (exit code 3, any other code, or a signal).
    #[error("harness command `{command}` failed with exit code {code}:\n{stderr_tail}")]
    Command {
        /// The harness command line, for the message.
        command: String,

        /// The exit code, `-1` when the process was killed by a signal.
        code: i32,

        /// The last lines of stderr.
        stderr_tail: String,
    },

    /// stdout did not follow the harness protocol.
    #[error("unexpected harness output: {0}")]
    Protocol(String),

    /// I/O error while starting the harness or handling its files.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A part of the generated fixture tree (`generate --only`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// `java_descriptors.txt`.
    Descriptors,

    /// `java_local/`.
    Local,

    /// `java_built/`.
    Built,

    /// `java_presets/`.
    Presets,

    /// `java_roundtrip/`.
    Roundtrip,
}

impl Part {
    /// Every part, in generation order.
    pub const ALL: [Self; 5] = [
        Self::Descriptors,
        Self::Local,
        Self::Built,
        Self::Presets,
        Self::Roundtrip,
    ];

    /// The harness argument for this part.
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Descriptors => "descriptors",
            Self::Local => "local",
            Self::Built => "built",
            Self::Presets => "presets",
            Self::Roundtrip => "roundtrip",
        }
    }

    /// Parses a harness argument (`"local"`, …).
    pub fn from_arg(arg: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|part| part.as_arg() == arg)
    }
}

/// How object graphs are dumped as JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DumpMode {
    /// Every object has an `id`, repeat visits are `{"ref":N}` (shared instances visible).
    Identity,

    /// No identities; sets and maps sorted. Equal exactly when the graphs are value-equal.
    Semantic,
}

/// One `OK` or `FAIL` line of a verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyLine {
    /// The verified file.
    pub path: PathBuf,

    /// Root class or detail for `OK`, the Java error (newlines as ` | `) for `FAIL`.
    pub detail: String,
}

/// Parsed output of `verify` and `selftest`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Files read successfully.
    pub ok: Vec<VerifyLine>,

    /// Files Java rejected.
    pub failed: Vec<VerifyLine>,

    /// Files the harness does not know how to read.
    pub skipped: Vec<PathBuf>,
}

impl VerifyReport {
    /// True when no file failed.
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }

    /// Parses the tab-separated `OK`/`FAIL`/`SKIP` lines and the final `SUMMARY` line.
    pub fn parse(stdout: &str) -> Result<Self, HarnessError> {
        let mut report = Self::default();
        let mut summary = None;
        for line in stdout.lines() {
            if summary.is_some() {
                return Err(HarnessError::Protocol(format!(
                    "line after SUMMARY: {line}"
                )));
            }
            let mut fields = line.splitn(3, '\t');
            let tag = fields.next().unwrap_or_default();
            match tag {
                "OK" | "FAIL" => {
                    let (Some(path), Some(detail)) = (fields.next(), fields.next()) else {
                        return Err(HarnessError::Protocol(format!(
                            "malformed {tag} line: {line}"
                        )));
                    };
                    let entry = VerifyLine {
                        path: PathBuf::from(path),
                        detail: detail.to_owned(),
                    };
                    if tag == "OK" {
                        report.ok.push(entry);
                    } else {
                        report.failed.push(entry);
                    }
                }
                "SKIP" => {
                    let Some(path) = fields.next() else {
                        return Err(HarnessError::Protocol(format!(
                            "malformed SKIP line: {line}"
                        )));
                    };
                    report.skipped.push(PathBuf::from(path));
                }
                "SUMMARY" => summary = Some(parse_summary(line)?),
                _ => return Err(HarnessError::Protocol(format!("unknown line: {line}"))),
            }
        }
        let Some((ok, fail, skip)) = summary else {
            return Err(HarnessError::Protocol("missing SUMMARY line".to_owned()));
        };
        if (ok, fail, skip) != (report.ok.len(), report.failed.len(), report.skipped.len()) {
            return Err(HarnessError::Protocol(format!(
                "SUMMARY ok={ok} fail={fail} skip={skip} does not match the {} OK, {} FAIL and {} SKIP lines",
                report.ok.len(),
                report.failed.len(),
                report.skipped.len()
            )));
        }
        Ok(report)
    }
}

fn parse_summary(line: &str) -> Result<(usize, usize, usize), HarnessError> {
    let bad = || HarnessError::Protocol(format!("malformed SUMMARY line: {line}"));
    let mut counts = [0_usize; 3];
    let mut fields = line.split('\t').skip(1);
    for (count, key) in counts.iter_mut().zip(["ok=", "fail=", "skip="]) {
        let field = fields.next().ok_or_else(bad)?;
        *count = field
            .strip_prefix(key)
            .and_then(|n| n.parse().ok())
            .ok_or_else(bad)?;
    }
    if fields.next().is_some() {
        return Err(bad());
    }
    Ok(counts.into())
}

/// Handle on the Java fixture harness of one repository checkout.
#[derive(Clone, Debug)]
pub struct Harness {
    repo_root: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl Harness {
    /// Finds the repository root: `$A3S_REPO_ROOT`, else three levels above this crate.
    pub fn locate() -> Result<Self, HarnessError> {
        let root = match std::env::var_os("A3S_REPO_ROOT") {
            Some(root) => PathBuf::from(root),
            None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."),
        };
        let root = root
            .canonicalize()
            .map_err(|_err| HarnessError::NotFound(root))?;
        Self::with_repo_root(root)
    }

    /// Uses an explicit repository root, which must contain `tools/java_fixture_harness/run.sh`.
    pub fn with_repo_root(root: impl Into<PathBuf>) -> Result<Self, HarnessError> {
        let repo_root = root.into();
        if !repo_root.join(HARNESS_DIR).join("run.sh").is_file() {
            return Err(HarnessError::NotFound(repo_root));
        }
        Ok(Self {
            repo_root,
            env: Vec::new(),
        })
    }

    /// Adds an environment variable for the harness process only (e.g. `A3S_JAVA_HOME`).
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The repository root this harness belongs to.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// The harness directory (`<root>/tools/java_fixture_harness`).
    pub fn harness_dir(&self) -> PathBuf {
        self.repo_root.join(HARNESS_DIR)
    }

    /// Compiles the harness if its inputs changed (`build.sh`).
    pub fn build(&self) -> Result<(), HarnessError> {
        self.build_output().map(drop)
    }

    /// Like [`Self::build`], returning the script's stdout (`up to date` when nothing changed).
    pub fn build_output(&self) -> Result<String, HarnessError> {
        let output = self.run_script("build.sh", &[])?;
        check_status("build.sh", &output, false)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Deletes `build/` and compiles from scratch (`build.sh --clean`).
    pub fn clean_build(&self) -> Result<String, HarnessError> {
        let output = self.run_script("build.sh", &[OsString::from("--clean")])?;
        check_status("build.sh --clean", &output, false)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Generates the fixture parts into `out`, which must be empty or missing. An empty `parts`
    /// means every part.
    pub fn generate(&self, out: &Path, parts: &[Part]) -> Result<(), HarnessError> {
        let parts = if parts.is_empty() {
            &Part::ALL[..]
        } else {
            parts
        };
        let only = parts
            .iter()
            .map(|part| part.as_arg())
            .collect::<Vec<_>>()
            .join(",");
        let args = [
            OsString::from("generate"),
            OsString::from("--out"),
            out.as_os_str().to_owned(),
            OsString::from("--only"),
            OsString::from(only),
        ];
        self.run_harness(&args, false).map(drop)
    }

    /// Reads files with Arma3Sync's own readers (`verify`). Java rejections are reported in the
    /// returned [`VerifyReport`], not as an error.
    pub fn verify(
        &self,
        paths: &[PathBuf],
        dump_dir: Option<&Path>,
        mode: DumpMode,
    ) -> Result<VerifyReport, HarnessError> {
        let mut args = vec![OsString::from("verify")];
        if let Some(dir) = dump_dir {
            args.push(OsString::from("--dump-dir"));
            args.push(dir.as_os_str().to_owned());
        }
        if mode == DumpMode::Semantic {
            args.push(OsString::from("--semantic"));
        }
        args.extend(paths.iter().map(|path| path.as_os_str().to_owned()));
        let stdout = self.run_harness(&args, true)?;
        VerifyReport::parse(&stdout)
    }

    /// Canonical JSON dump of one object file (`dump`).
    pub fn dump(&self, file: &Path, mode: DumpMode) -> Result<String, HarnessError> {
        let mut args = vec![OsString::from("dump")];
        if mode == DumpMode::Semantic {
            args.push(OsString::from("--semantic"));
        }
        args.push(file.as_os_str().to_owned());
        self.run_harness(&args, false)
    }

    /// Java read of `input` followed by the upload serialization into `output` (`roundtrip`).
    pub fn roundtrip(&self, input: &Path, output: &Path) -> Result<(), HarnessError> {
        let args = [
            OsString::from("roundtrip"),
            input.as_os_str().to_owned(),
            output.as_os_str().to_owned(),
        ];
        let stdout = self.run_harness(&args, false)?;
        if !stdout.starts_with("OK\t") {
            return Err(HarnessError::Protocol(stdout));
        }
        Ok(())
    }

    /// The harness self-test (port of `A3SFilesAccessorTest`).
    pub fn selftest(&self) -> Result<VerifyReport, HarnessError> {
        let stdout = self.run_harness(&[OsString::from("selftest")], true)?;
        VerifyReport::parse(&stdout)
    }

    /// Runs `run.sh <args>`; exit code 1 is accepted when `failures_allowed`.
    fn run_harness(
        &self,
        args: &[OsString],
        failures_allowed: bool,
    ) -> Result<String, HarnessError> {
        let output = self.run_script("run.sh", args)?;
        let command = describe("run.sh", args);
        check_status(&command, &output, failures_allowed)?;
        String::from_utf8(output.stdout)
            .map_err(|_err| HarnessError::Protocol(format!("`{command}` printed invalid UTF-8")))
    }

    #[cfg(unix)]
    fn run_script(&self, script: &str, args: &[OsString]) -> Result<Output, HarnessError> {
        let mut command = std::process::Command::new("bash");
        command
            .arg(self.harness_dir().join(script))
            .args(args)
            .stdin(std::process::Stdio::null());
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command.output().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                HarnessError::NoBash
            } else {
                HarnessError::Io(err)
            }
        })
    }

    #[cfg(not(unix))]
    #[expect(
        clippy::unused_self,
        reason = "same signature as the Unix implementation"
    )]
    fn run_script(&self, _script: &str, _args: &[OsString]) -> Result<Output, HarnessError> {
        Err(HarnessError::Unsupported)
    }
}

fn describe(script: &str, args: &[OsString]) -> String {
    std::iter::once(OsStr::new(script))
        .chain(args.iter().map(OsString::as_os_str))
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Maps the harness exit code to an error (B47).
fn check_status(
    command: &str,
    output: &Output,
    failures_allowed: bool,
) -> Result<(), HarnessError> {
    let code = output.status.code();
    match code {
        Some(0) => Ok(()),
        Some(1) if failures_allowed => Ok(()),
        Some(2) => Err(HarnessError::Usage(stderr_tail(&output.stderr))),
        Some(4) => Err(HarnessError::NoJdk(stderr_tail(&output.stderr))),
        _ => Err(HarnessError::Command {
            command: command.to_owned(),
            code: code.unwrap_or(-1),
            stderr_tail: stderr_tail(&output.stderr),
        }),
    }
}

fn stderr_tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(STDERR_TAIL_LINES);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::{HarnessError, Part, VerifyReport};

    #[test]
    fn a3_report_parsing_maps_lines() {
        let stdout = "OK\t/r/.a3s/sync\tfr.soe.a3s.domain.repository.SyncTreeDirectory\n\
                      FAIL\t/r/.a3s/serverinfo\tFailed to read file: serverinfo | The file seems to be corrupted.\n\
                      SKIP\t/r/readme.txt\n\
                      SUMMARY\tok=1\tfail=1\tskip=1\n";
        let report = VerifyReport::parse(stdout).unwrap();
        assert_eq!(report.ok.len(), 1);
        assert_eq!(report.ok[0].path.to_str(), Some("/r/.a3s/sync"));
        assert_eq!(
            report.ok[0].detail,
            "fr.soe.a3s.domain.repository.SyncTreeDirectory"
        );
        assert_eq!(
            report.failed[0].detail,
            "Failed to read file: serverinfo | The file seems to be corrupted."
        );
        assert_eq!(report.skipped[0].to_str(), Some("/r/readme.txt"));
        assert!(!report.is_success());
    }

    #[test]
    fn a3_report_parsing_rejects_bad_output() {
        let missing_summary = "OK\t/a\tX\n";
        assert!(matches!(
            VerifyReport::parse(missing_summary),
            Err(HarnessError::Protocol(_))
        ));
        let unknown_tag = "MAYBE\t/a\tX\nSUMMARY\tok=0\tfail=0\tskip=0\n";
        assert!(matches!(
            VerifyReport::parse(unknown_tag),
            Err(HarnessError::Protocol(_))
        ));
        let wrong_counts = "OK\t/a\tX\nSUMMARY\tok=2\tfail=0\tskip=0\n";
        assert!(matches!(
            VerifyReport::parse(wrong_counts),
            Err(HarnessError::Protocol(_))
        ));
        let empty = "SUMMARY\tok=0\tfail=0\tskip=0\n";
        assert!(VerifyReport::parse(empty).unwrap().is_success());
    }

    #[cfg(unix)]
    #[test]
    fn a3_exit_codes_map_to_results() {
        use std::os::unix::process::ExitStatusExt as _;
        use std::process::{ExitStatus, Output};

        let output = |raw: i32| Output {
            status: ExitStatus::from_raw(raw),
            stdout: Vec::new(),
            stderr: b"line 1\nset A3S_JAVA_HOME\n".to_vec(),
        };
        let exit = |code: i32| output(code << 8);
        assert!(super::check_status("verify", &exit(0), false).is_ok());
        // Exit 1 is a report with failures for verify/selftest, an error elsewhere.
        assert!(super::check_status("verify", &exit(1), true).is_ok());
        assert!(matches!(
            super::check_status("dump", &exit(1), false),
            Err(HarnessError::Command { code: 1, .. })
        ));
        assert!(matches!(
            super::check_status("generate", &exit(2), false),
            Err(HarnessError::Usage(_))
        ));
        match super::check_status("build.sh", &exit(4), false) {
            Err(HarnessError::NoJdk(tail)) => assert!(tail.contains("A3S_JAVA_HOME")),
            other => panic!("expected NoJdk, got {other:?}"),
        }
        assert!(matches!(
            super::check_status("generate", &exit(3), false),
            Err(HarnessError::Command { code: 3, .. })
        ));
        // Killed by SIGKILL.
        assert!(matches!(
            super::check_status("generate", &output(9), false),
            Err(HarnessError::Command { code: -1, .. })
        ));
    }

    #[test]
    fn a3_stderr_tail_keeps_the_last_lines() {
        let stderr: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let tail = super::stderr_tail(stderr.as_bytes());
        assert_eq!(tail.lines().count(), 40);
        assert!(tail.starts_with("line 60"));
        assert!(tail.ends_with("line 99"));
    }

    #[test]
    fn a3_part_arguments_round_trip() {
        for part in Part::ALL {
            assert_eq!(Part::from_arg(part.as_arg()), Some(part));
        }
        assert_eq!(Part::from_arg("everything"), None);
    }
}
