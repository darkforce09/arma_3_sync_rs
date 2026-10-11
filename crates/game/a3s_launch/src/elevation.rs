//! Starting the game elevated (UAC) on Windows through PowerShell's `Start-Process -Verb RunAs`,
//! without elevating Arma3Sync itself.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub use a3s_platform::shortcut::quote_windows_arg;

use crate::command::CommandSpec;

/// A PowerShell single-quoted string literal. Every single-quote character PowerShell accepts
/// (`'` and the curly `‘ ’ ‚ ‛`) is doubled, so no argument can end the literal.
// Mirrors dao/LauncherDAO.java:451-453
pub fn quote_powershell(s: &str) -> String {
    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}'..='\u{201B}') {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

/// The PowerShell script that starts `exe` elevated, waits for it and exits with its exit code
/// (`1` if starting fails, for example when the UAC prompt is refused).
///
/// Each argument is quoted for the Windows command line before it becomes a PowerShell literal,
/// because `Start-Process` joins `-ArgumentList` with spaces without quoting. Without arguments
/// `-ArgumentList` is left out, since it rejects an empty array.
// Mirrors dao/LauncherDAO.java:427-442
pub fn elevation_script(exe: &str, working_dir: Option<&str>, args: &[String]) -> String {
    let arguments = args
        .iter()
        .map(|arg| quote_powershell(&quote_windows_arg(arg)))
        .collect::<Vec<_>>()
        .join(",");
    let mut script = format!(
        "$ErrorActionPreference='Stop'; try {{ $arguments=@({arguments}); $process=Start-Process -FilePath {}",
        quote_powershell(exe)
    );
    if let Some(dir) = working_dir {
        script.push_str(" -WorkingDirectory ");
        script.push_str(&quote_powershell(dir));
    }
    if !args.is_empty() {
        script.push_str(" -ArgumentList $arguments");
    }
    script.push_str(" -Verb RunAs -Wait -PassThru; exit $process.ExitCode } catch { exit 1 }");
    script
}

/// `script` as PowerShell's `-EncodedCommand` wants it: UTF-16LE, then Base64 with padding.
// Mirrors dao/LauncherDAO.java:444
pub fn encode_powershell_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

/// The hidden PowerShell command that runs [`elevation_script`].
// Mirrors dao/LauncherDAO.java:445-449
pub fn elevated_command(exe: &str, working_dir: Option<&str>, args: &[String]) -> CommandSpec {
    let encoded = encode_powershell_command(&elevation_script(exe, working_dir, args));
    CommandSpec {
        program: "powershell.exe".into(),
        args: [
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
        ]
        .into_iter()
        .map(Into::into)
        .chain(std::iter::once(encoded.into()))
        .collect(),
        current_dir: None,
        hide_window: true,
        discard_output: true,
    }
}
