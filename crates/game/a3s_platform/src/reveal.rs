//! Showing a file in the system file manager.

use std::io;
use std::path::Path;
use std::process::{Child, Command};

/// Opens the file manager on `path`.
///
/// The file is selected in Explorer (Windows) and Finder (macOS); elsewhere its folder opens
/// (`xdg-open`). Returns once the file manager was started; a named thread waits for the
/// launcher so it leaves no zombie process.
pub fn show_in_folder(path: &Path) -> io::Result<()> {
    let child = launcher(path).spawn()?;
    std::thread::Builder::new()
        .name("a3s-reveal".to_owned())
        .spawn(move || wait(child))?;
    Ok(())
}

fn launcher(path: &Path) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("explorer");
        command.arg(format!("/select,{}", path.display()));
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg("-R").arg(path);
        command
    } else {
        let folder = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let mut command = Command::new("xdg-open");
        command.arg(folder);
        command
    }
}

fn wait(mut child: Child) {
    if let Err(err) = child.wait() {
        tracing::debug!("file manager launcher: {err}");
    }
}
