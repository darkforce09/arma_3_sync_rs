//! Requests to the main window from other threads: the tray menu and later instances.

use std::sync::mpsc;

/// What the tray or another instance asks the main window to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRequest {
    /// Show the window and bring it to the front.
    Show,

    /// Hide the window (it stays in the tray).
    Hide,

    /// Check every repository now.
    CheckNow,

    /// Quit the application, as the menu's Quit does.
    Quit,
}

/// Sends [`WindowRequest`]s to the window and wakes it, even while it is hidden: eframe then
/// calls [`crate::Shell::logic`], which drains the requests.
#[derive(Clone)]
pub struct RequestSender {
    sender: mpsc::Sender<WindowRequest>,
    ctx: egui::Context,
}

impl RequestSender {
    /// Queues `request` and requests a repaint; a closed window ignores it.
    pub fn send(&self, request: WindowRequest) {
        // The receiver is gone only once the window closed.
        self.sender.send(request).ok();
        self.ctx.request_repaint();
    }
}

/// A request channel waking `ctx`; the receiver goes into [`crate::ShellSetup`].
pub fn channel(ctx: &egui::Context) -> (RequestSender, mpsc::Receiver<WindowRequest>) {
    let (sender, receiver) = mpsc::channel();
    (
        RequestSender {
            sender,
            ctx: ctx.clone(),
        },
        receiver,
    )
}
