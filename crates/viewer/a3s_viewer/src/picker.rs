//! Folder and file pickers through the system dialog (the XDG desktop portal on Linux), without
//! blocking the frame.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The choice of one open dialog, awaited on a named thread.
#[derive(Default)]
struct PendingChoice {
    pending: Option<mpsc::Receiver<Option<PathBuf>>>,
}

impl PendingChoice {
    /// Awaits `choice` on a thread; the window repaints when the dialog closes.
    fn start(
        &mut self,
        ctx: &egui::Context,
        choice: impl Future<Output = Option<rfd::FileHandle>> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("a3s-file-dialog".to_owned())
            .spawn(move || {
                let path = futures::executor::block_on(choice).map(|h| h.path().to_path_buf());
                sender.send(path).ok();
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.pending = Some(receiver),
            Err(err) => tracing::warn!("Cannot open the file dialog: {err}"),
        }
    }

    fn poll(&mut self) -> Option<PathBuf> {
        let receiver = self.pending.as_ref()?;
        match receiver.try_recv() {
            Ok(path) => {
                self.pending = None;
                path
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
        }
    }
}

fn dialog(title: &str, start: Option<&Path>) -> rfd::AsyncFileDialog {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
    if let Some(start) = start.filter(|dir| dir.is_dir()) {
        dialog = dialog.set_directory(start);
    }
    dialog
}

/// One folder dialog at a time; [`Self::poll`] returns the choice once the user made it.
#[derive(Default)]
pub(crate) struct FolderPicker {
    choice: PendingChoice,
}

impl FolderPicker {
    /// Opens the dialog unless one is open; the window repaints when it closes.
    pub(crate) fn open(&mut self, ctx: &egui::Context, title: &str, start: Option<&Path>) {
        if self.is_open() {
            return;
        }
        // Created here (macOS opens dialogs from the main thread), awaited on a worker.
        let choice = dialog(title, start).pick_folder();
        self.choice.start(ctx, choice);
    }

    /// Whether a dialog is open.
    pub(crate) fn is_open(&self) -> bool {
        self.choice.pending.is_some()
    }

    /// The chosen folder, once; `None` while the dialog is open, after a cancel, or when no
    /// dialog was opened.
    pub(crate) fn poll(&mut self) -> Option<PathBuf> {
        self.choice.poll()
    }
}

/// One file dialog at a time; [`Self::poll`] returns the choice once the user made it.
#[derive(Default)]
pub(crate) struct FilePicker {
    choice: PendingChoice,
}

impl FilePicker {
    /// Opens the dialog for an existing file unless one is open; the window repaints when it
    /// closes.
    pub(crate) fn open(&mut self, ctx: &egui::Context, title: &str, start: Option<&Path>) {
        if self.is_open() {
            return;
        }
        let choice = dialog(title, start).pick_file();
        self.choice.start(ctx, choice);
    }

    /// Whether a dialog is open.
    pub(crate) fn is_open(&self) -> bool {
        self.choice.pending.is_some()
    }

    /// The chosen file, once; `None` while the dialog is open, after a cancel, or when no
    /// dialog was opened.
    pub(crate) fn poll(&mut self) -> Option<PathBuf> {
        self.choice.poll()
    }
}
