//! Showing a file or folder in the system file manager.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use a3s_foundation::paths::{Os, PlatformEnv};

use crate::Error;

/// Shows `path` in the system file manager: a folder is opened, a file is selected in its
/// folder.
///
/// Explorer (Windows) and Finder (macOS) select the file. Elsewhere the file manager is asked to
/// select it through the `org.freedesktop.FileManager1` D-Bus service (`dbus-send`); when that
/// cannot start or fails, the file's folder opens through `xdg-open`. Returns once a launcher
/// started; the `a3s-reveal` thread reaps it (and runs the fallback), so it leaves no zombie.
pub fn show_in_folder(env: &PlatformEnv, path: &Path) -> crate::Result<()> {
    let path = std::path::absolute(path).map_err(|source| Error::io(path, source))?;
    let mut attempts = launchers(env.os(), &path)?;
    let child = spawn_next(&mut attempts, &path)?;
    std::thread::Builder::new()
        .name("a3s-reveal".to_owned())
        .spawn(move || reap(child, attempts, &path))
        .map(drop)
        .map_err(|source| Error::Io { path: None, source })
}

/// The commands to try in order; each one runs only when the one before it failed.
fn launchers(os: Os, path: &Path) -> crate::Result<VecDeque<Command>> {
    let file = !path.is_dir();
    let mut attempts = VecDeque::new();
    match os {
        Os::Windows => {
            let mut command = Command::new("explorer");
            if file {
                command.arg("/select,");
            }
            command.arg(path);
            attempts.push_back(command);
        }
        Os::MacOs => {
            let mut command = Command::new("open");
            if file {
                command.arg("-R");
            }
            command.arg(path);
            attempts.push_back(command);
        }
        Os::Linux | Os::OtherUnix => {
            let folder = if file {
                path.parent().unwrap_or(path)
            } else {
                path
            };
            if file {
                attempts.push_back(show_items(path));
            }
            let mut open = Command::new("xdg-open");
            open.arg(folder);
            attempts.push_back(open);
        }
        Os::Other => return Err(Error::Unsupported),
    }
    for command in &mut attempts {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    Ok(attempts)
}

/// `org.freedesktop.FileManager1.ShowItems` for `path`; `--print-reply` makes the exit status
/// tell whether a file manager answered.
fn show_items(path: &Path) -> Command {
    let mut item = OsString::from("array:string:");
    item.push(file_uri(path));
    let mut command = Command::new("dbus-send");
    command
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(item)
        .arg("string:");
    command
}

/// The `file://` URI of the absolute `path`, percent-encoded so commas (the `dbus-send` array
/// separator) and other reserved bytes stay inside the item.
fn file_uri(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push('%');
            uri.push(char::from(HEX[usize::from(byte >> 4)]));
            uri.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    uri
}

/// Starts the first command of `attempts` that starts; the error is the last one's.
fn spawn_next(attempts: &mut VecDeque<Command>, path: &Path) -> crate::Result<Child> {
    let mut last = Error::Unsupported;
    while let Some(mut command) = attempts.pop_front() {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(source) => last = Error::io(path, source),
        }
    }
    Err(last)
}

/// Waits for the launcher; when it failed and a fallback is left, runs that.
fn reap(mut child: Child, mut attempts: VecDeque<Command>, path: &Path) {
    loop {
        let succeeded = match child.wait() {
            Ok(status) => status.success(),
            Err(err) => {
                tracing::debug!("file manager launcher: {err}");
                false
            }
        };
        if succeeded || attempts.is_empty() {
            return;
        }
        match spawn_next(&mut attempts, path) {
            Ok(next) => child = next,
            Err(err) => {
                tracing::debug!("file manager fallback: {err}");
                return;
            }
        }
    }
}
