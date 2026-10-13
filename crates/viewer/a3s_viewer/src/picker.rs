//! Folder picker through the system dialog (the XDG desktop portal on Linux), without blocking
//! the frame.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// One folder dialog at a time; [`Self::poll`] returns the choice once the user made it.
#[derive(Default)]
pub(crate) struct FolderPicker {
    pending: Option<mpsc::Receiver<Option<PathBuf>>>,
}

impl FolderPicker {
    /// Opens the dialog unless one is open; the window repaints when it closes.
    pub(crate) fn open(&mut self, ctx: &egui::Context, title: &str, start: Option<&Path>) {
        if self.pending.is_some() {
            return;
        }
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
        if let Some(start) = start.filter(|dir| dir.is_dir()) {
            dialog = dialog.set_directory(start);
        }
        // Created here (macOS opens dialogs from the main thread), awaited on a worker.
        let choice = dialog.pick_folder();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("a3s-folder-picker".to_owned())
            .spawn(move || {
                let folder = futures::executor::block_on(choice).map(|h| h.path().to_path_buf());
                sender.send(folder).ok();
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.pending = Some(receiver),
            Err(err) => tracing::warn!("Cannot open the folder dialog: {err}"),
        }
    }

    /// Whether a dialog is open.
    pub(crate) fn is_open(&self) -> bool {
        self.pending.is_some()
    }

    /// The chosen folder, once; `None` while the dialog is open, after a cancel, or when no
    /// dialog was opened.
    pub(crate) fn poll(&mut self) -> Option<PathBuf> {
        let receiver = self.pending.as_ref()?;
        match receiver.try_recv() {
            Ok(folder) => {
                self.pending = None;
                folder
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
        }
    }
}
