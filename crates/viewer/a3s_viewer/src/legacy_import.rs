//! The import of the Java client's data, run on a named thread; afterwards the client data is
//! loaded again and every repository is checked.

use std::convert::Infallible;
use std::sync::mpsc;

use a3s_foundation::error::UserFacing as _;
use a3s_foundation::paths::legacy::LegacyJavaData;
use a3s_repo_files::legacy::{self, ImportError, ImportOptions, ImportReport};
use a3s_repo_files::services::{ClientData, LoadOutcome};
use a3s_viewer_context::{Change, Command, ViewerContext, repositories};

/// Where an import stands.
#[derive(Default)]
pub(crate) enum LegacyImport {
    /// Not started.
    #[default]
    NotStarted,

    /// Running on its thread.
    Running(mpsc::Receiver<Result<ImportReport, ImportError>>),

    /// Finished: the summary lines, the warnings, or why it failed.
    Done(Vec<String>),
}

impl LegacyImport {
    /// Starts importing `sources` into the context's store.
    pub(crate) fn start(
        &mut self,
        viewer: &ViewerContext,
        ctx: &egui::Context,
        sources: &[LegacyJavaData],
    ) {
        let store = viewer.read().client.store().clone();
        let sources = sources.to_vec();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("a3s-legacy-import".to_owned())
            .spawn(move || {
                let result = legacy::import(&store, &sources, &ImportOptions::default());
                sender.send(result).ok();
                ctx.request_repaint();
            });
        *self = match spawned {
            Ok(_) => Self::Running(receiver),
            Err(err) => Self::Done(vec![format!("The import could not start: {err}")]),
        };
    }

    /// Whether the import runs.
    pub(crate) fn is_running(&self) -> bool {
        matches!(self, Self::Running(_))
    }

    /// Takes the result once it arrived; after a successful import the client data is loaded
    /// again and every repository is checked.
    pub(crate) fn poll(&mut self, viewer: &ViewerContext) {
        let Self::Running(receiver) = self else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                *self = Self::Done(vec!["The import stopped unexpectedly.".to_owned()]);
                return;
            }
        };
        let lines = match result {
            Ok(report) => {
                let mut lines = report.summary();
                lines.extend(report.warnings.iter().cloned());
                if report.imported_anything() {
                    lines.extend(reload(viewer));
                }
                lines
            }
            Err(err) => vec![err.user_message()],
        };
        *self = Self::Done(lines);
    }
}

/// Loads the client data again into the context and starts a check of every repository;
/// returns the load warnings.
fn reload(viewer: &ViewerContext) -> Vec<String> {
    let store = viewer.read().client.store().clone();
    let LoadOutcome { data, warnings } = ClientData::load(store);
    let names = data.repository_names();
    viewer
        .mutate(Some(Change::Repository), |state| {
            state.client = data;
            Ok::<(), Infallible>(())
        })
        .ok();
    if let Err(err) = viewer.send(Command::Repositories(repositories::Command::Check {
        names,
    })) {
        tracing::warn!("Cannot check the imported repositories: {err}");
    }
    warnings
}
