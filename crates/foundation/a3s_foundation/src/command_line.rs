//! Quoting for Windows command lines and PowerShell commands, shared by every crate that starts
//! a process through a Windows shortcut, an autostart entry or a UAC-elevated PowerShell call.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Quotes one argument for a Windows command line so `CommandLineToArgvW` (and the MSVC runtime)
/// splits it back into the same string.
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{0B}', '"']) {
        return arg.to_owned();
    }
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, and the quote itself is escaped.
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(c);
                backslashes = 0;
            }
        }
    }
    // Backslashes before the closing quote are doubled.
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// The arguments of a Windows command line: each quoted with [`quote_windows_arg`], joined by
/// single spaces.
pub fn windows_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

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

/// `script` as PowerShell's `-EncodedCommand` wants it: UTF-16LE, then Base64 with padding.
// Mirrors dao/LauncherDAO.java:444
pub fn encode_powershell_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}
