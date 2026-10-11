//! Permission-failure classification, the write probe, the elevation rule and the PowerShell
//! command that starts the elevated updater.

use std::io;
use std::path::PathBuf;

use a3s_updater::apply::ELEVATED_SUBCOMMAND;
use a3s_updater::elevation::{ElevatedLaunch, can_write_installation, is_permission_failure};
use a3s_updater::{ApplyError, DiagnosticLog, UpdateError};
use base64::Engine as _;

/// An error with a message whose source is an I/O error.
#[derive(Debug)]
struct Wrapped(io::Error);

impl std::fmt::Display for Wrapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("install failed")
    }
}

impl std::error::Error for Wrapped {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn permission_failures_are_found_in_the_cause_chain() {
    let nested = ApplyError::Install {
        dir: PathBuf::from("installation"),
        source: UpdateError::Stage {
            stage: "installation copy",
            source: Box::new(UpdateError::Io {
                context: "Could not install arma3sync".to_owned(),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            }),
        },
    };
    assert!(is_permission_failure(&nested));
    assert!(nested.is_permission_failure());
    assert!(is_permission_failure(&Wrapped(io::Error::from(
        io::ErrorKind::PermissionDenied
    ))));
    assert!(is_permission_failure(&io::Error::other(
        "Zugriff verweigert"
    )));
    assert!(is_permission_failure(&io::Error::other(
        "Permission denied"
    )));
    assert!(is_permission_failure(&io::Error::other("Access denied")));
}

#[test]
fn other_failures_are_not_permission_failures() {
    let not = |err: &(dyn std::error::Error + 'static)| !is_permission_failure(err);
    assert!(not(&io::Error::other(
        "The process cannot access the file because it is being used by another process."
    )));
    assert!(not(&io::Error::new(
        io::ErrorKind::NotFound,
        "extracted/runtime/legal/java.base"
    )));
    assert!(not(&io::Error::from(io::ErrorKind::ReadOnlyFilesystem)));
    assert!(not(&UpdateError::MissingSource(PathBuf::from(
        "extracted/runtime/legal/java.base"
    ))));
    assert!(not(&UpdateError::Transport("permission denied".to_owned())));
    assert!(not(&ApplyError::ElevationUnavailable));
}

#[cfg(windows)]
#[test]
fn windows_lock_violations_are_not_permission_failures() {
    assert!(!is_permission_failure(&io::Error::from_raw_os_error(32)));
    assert!(!is_permission_failure(&io::Error::from_raw_os_error(33)));
    assert!(is_permission_failure(&io::Error::from_raw_os_error(5)));
}

#[test]
fn write_probe_leaves_nothing_behind() {
    let temp = tempfile::tempdir().expect("tempdir");
    let diag = DiagnosticLog::disabled();
    assert!(can_write_installation(temp.path(), &diag));
    assert_eq!(std::fs::read_dir(temp.path()).expect("list").count(), 0);

    assert!(!can_write_installation(&temp.path().join("missing"), &diag));
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").expect("write");
    assert!(!can_write_installation(&file, &diag));
}

#[cfg(unix)]
#[test]
fn write_probe_and_elevation_rule_on_a_read_only_directory() {
    use a3s_foundation::paths::Os;
    use a3s_updater::elevation::requires_elevation;

    let temp = tempfile::tempdir().expect("tempdir");
    let diag = DiagnosticLog::disabled();
    let writable = temp.path().join("writable");
    let read_only = temp.path().join("read-only");
    std::fs::create_dir(&writable).expect("mkdir");
    std::fs::create_dir(&read_only).expect("mkdir");
    let Some(_guard) = crate::apply_fixture::ReadOnly::new(&read_only) else {
        eprintln!("skipped: running as root");
        return;
    };
    assert!(!can_write_installation(&read_only, &diag));

    assert!(!requires_elevation(Os::Linux, false, &read_only, &diag));
    assert!(!requires_elevation(Os::Windows, false, &writable, &diag));
    assert!(!requires_elevation(Os::Windows, true, &read_only, &diag));
    assert!(!requires_elevation(
        Os::Windows,
        false,
        &temp.path().join("missing"),
        &diag
    ));
    assert!(requires_elevation(Os::Windows, false, &read_only, &diag));
}

/// Splits a command line by the `CommandLineToArgvW` rules.
fn split_windows_args(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut args = Vec::new();
    let mut index = 0;
    loop {
        while index < chars.len() && matches!(chars[index], ' ' | '\t') {
            index += 1;
        }
        if index == chars.len() {
            return args;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while index < chars.len() {
            match chars[index] {
                '\\' => {
                    let start = index;
                    while index < chars.len() && chars[index] == '\\' {
                        index += 1;
                    }
                    let count = index - start;
                    if chars.get(index) == Some(&'"') {
                        arg.extend(std::iter::repeat_n('\\', count / 2));
                        if count % 2 == 1 {
                            arg.push('"');
                            index += 1;
                        }
                    } else {
                        arg.extend(std::iter::repeat_n('\\', count));
                    }
                }
                '"' => {
                    if quoted && chars.get(index + 1) == Some(&'"') {
                        arg.push('"');
                        index += 2;
                    } else {
                        quoted = !quoted;
                        index += 1;
                    }
                }
                ' ' | '\t' if !quoted => break,
                c => {
                    arg.push(c);
                    index += 1;
                }
            }
        }
        args.push(arg);
    }
}

fn decode_utf16le_base64(encoded: &str) -> String {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("base64");
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).expect("utf-16")
}

#[test]
fn powershell_command_starts_the_elevated_updater_and_waits() {
    let args: Vec<String> = [
        ELEVATED_SUBCOMMAND,
        "--archive",
        r"C:\Users\Daniel\AppData\Local\Temp\arma3sync-update-x\Arma3Sync 2026.zip",
        "--marker",
        r"C:\Temp\with space\",
        "--result",
        r"C:\Users\O'Brien\Temp\elevated.result",
        "--version",
        r#"quote"inside"#,
    ]
    .iter()
    .map(|&s| s.to_owned())
    .collect();
    let launch = ElevatedLaunch {
        program: PathBuf::from(r"C:\Program Files (x86)\ArmA3Sync\arma3sync.exe"),
        args: args.clone(),
        working_dir: PathBuf::from(r"C:\Program Files (x86)\ArmA3Sync"),
    };

    let command = launch.powershell_command();
    for part in [
        "Start-Process",
        "-Verb RunAs",
        "-WindowStyle Hidden",
        r"-FilePath 'C:\Program Files (x86)\ArmA3Sync\arma3sync.exe'",
        r"-WorkingDirectory 'C:\Program Files (x86)\ArmA3Sync'",
        "O''Brien",
        "WaitForExit()",
        "exit 1223",
        "exit $p.ExitCode",
    ] {
        assert!(command.contains(part), "{part} missing from {command}");
    }
    assert_eq!(decode_utf16le_base64(&launch.encoded_command()), command);
    assert_eq!(split_windows_args(&launch.parameters()), args);
}
