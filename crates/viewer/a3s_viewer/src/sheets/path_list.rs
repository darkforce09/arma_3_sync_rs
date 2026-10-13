//! A small sheet editing one path list of a repository built on this computer: the files the
//! build leaves out, or the folders players keep their own files in. Changes save at once;
//! a removal can be undone.

use std::path::{Path, PathBuf};

use a3s_foundation::error::UserFacing as _;
use a3s_repo_files::services::ServiceError;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::button;
use a3s_viewer_context::{Change, ViewerContext, admin};
use egui::{Align, Label, Layout, RichText};

use super::Shown;
use crate::picker::{FilePicker, FolderPicker};
use crate::toasts::Toaster;

/// Which list the sheet edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathListKind {
    /// Files of the source folder the build leaves out.
    ExcludedFiles,

    /// Folders of the repository where players' extra files are kept (`hidddenFolderPaths`).
    HiddenFolders,
}

impl PathListKind {
    /// The sheet title.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::ExcludedFiles => "Excluded files",
            Self::HiddenFolders => "Hidden folders",
        }
    }

    fn explanation(self) -> &'static str {
        match self {
            Self::ExcludedFiles => "These files of the source folder are not published.",
            Self::HiddenFolders => {
                "Players keep their own files in these folders: updates leave them in place."
            }
        }
    }

    /// The stored paths of `repository`, in order.
    pub(crate) fn paths(self, viewer: &ViewerContext, repository: &str) -> Vec<String> {
        let state = viewer.read();
        let Ok(stored) = state.client.repository(repository) else {
            return Vec::new();
        };
        match self {
            Self::ExcludedFiles => stored.excluded_files_from_build.iter().cloned().collect(),
            Self::HiddenFolders => stored.excluded_folders_from_sync.iter().cloned().collect(),
        }
    }

    fn store(
        self,
        viewer: &ViewerContext,
        repository: &str,
        paths: Vec<String>,
    ) -> Result<(), ServiceError> {
        viewer.mutate(Some(Change::Repository), |state| {
            match self {
                Self::ExcludedFiles => state
                    .client
                    .set_excluded_files_from_build(repository, paths)?,
                Self::HiddenFolders => state
                    .client
                    .set_excluded_folders_from_sync(repository, paths)?,
            }
            // The source folder compares differently now.
            if let Some(publishing) = state.admin.repositories.get_mut(repository) {
                publishing.local = None;
            }
            Ok(())
        })
    }
}

/// The sheet's state.
pub(crate) struct PathList {
    repository: String,
    pub(super) kind: PathListKind,
    file_picker: FilePicker,
    folder_picker: FolderPicker,
    error: Option<String>,
}

impl PathList {
    pub(crate) fn new(repository: &str, kind: PathListKind) -> Self {
        Self {
            repository: repository.to_owned(),
            kind,
            file_picker: FilePicker::default(),
            folder_picker: FolderPicker::default(),
            error: None,
        }
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
    ) -> Shown {
        let source = viewer
            .read()
            .client
            .repository(&self.repository)
            .ok()
            .and_then(|stored| stored.path.clone())
            .map(PathBuf::from);
        if let Some(chosen) = self
            .file_picker
            .poll()
            .or_else(|| self.folder_picker.poll())
        {
            self.add(viewer, source.as_deref(), &chosen);
        }
        let action = Dialog::new("a3s_viewer::sheet::path_list", self.kind.title())
            .show_ok(false)
            .cancel_label("Done")
            .default_button(Some(DialogButton::Cancel))
            .show(ctx, |ui| self.body(ui, viewer, toasts, source.as_deref()));
        if action == DialogAction::None {
            return Shown::Open;
        }
        viewer
            .send(a3s_viewer_context::Command::Admin(
                admin::Command::Refresh {
                    repository: self.repository.clone(),
                    remote: false,
                },
            ))
            .ok();
        Shown::Closed(None)
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
        source: Option<&Path>,
    ) {
        let palette = theme::palette(ui.ctx());
        ui.set_width(440.0);
        ui.label(self.kind.explanation());
        ui.add_space(6.0);
        let paths = self.kind.paths(viewer, &self.repository);
        frames::group(ui, |ui| {
            if paths.is_empty() {
                ui.label(RichText::new("None").color(palette.text_secondary));
            }
            for (index, path) in paths.iter().enumerate() {
                if index > 0 {
                    frames::group_separator(ui);
                }
                ui.horizontal(|ui| {
                    ui.add(Label::new(display_path(source, path)).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(icons::MINUS_CIRCLE)
                            .on_hover_text("Remove")
                            .clicked()
                        {
                            self.remove(viewer, toasts, &paths, path, source);
                        }
                    });
                });
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = match self.kind {
                PathListKind::ExcludedFiles => "Add file…",
                PathListKind::HiddenFolders => "Add folder…",
            };
            if button(ui, label).clicked() {
                let title = self.kind.title();
                match self.kind {
                    PathListKind::ExcludedFiles => {
                        self.file_picker.open(ui.ctx(), title, source);
                    }
                    PathListKind::HiddenFolders => {
                        self.folder_picker.open(ui.ctx(), title, source);
                    }
                }
            }
        });
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(palette.status_danger));
        }
    }

    /// Adds `chosen`, which must be inside the source folder.
    fn add(&mut self, viewer: &ViewerContext, source: Option<&Path>, chosen: &Path) {
        if !source.is_some_and(|source| chosen.starts_with(source) && chosen != source) {
            self.error = Some("Choose something inside the source folder.".to_owned());
            return;
        }
        let mut paths = self.kind.paths(viewer, &self.repository);
        let text = chosen.display().to_string();
        if !paths.contains(&text) {
            paths.push(text);
        }
        self.error = self
            .kind
            .store(viewer, &self.repository, paths)
            .err()
            .map(|err| err.user_message());
    }

    fn remove(
        &mut self,
        viewer: &ViewerContext,
        toasts: &mut Toaster,
        paths: &[String],
        path: &str,
        source: Option<&Path>,
    ) {
        let remaining: Vec<String> = paths.iter().filter(|p| *p != path).cloned().collect();
        match self.kind.store(viewer, &self.repository, remaining) {
            Ok(()) => {
                let kind = self.kind;
                let repository = self.repository.clone();
                let before = paths.to_vec();
                toasts.undoable(
                    format!("Removed {}", display_path(source, path)),
                    move |viewer: &ViewerContext| {
                        if let Err(err) = kind.store(viewer, &repository, before) {
                            tracing::warn!("Cannot undo the removal: {}", err.user_message());
                        }
                    },
                );
            }
            Err(err) => self.error = Some(err.user_message()),
        }
    }
}

/// `path` relative to the source folder when it is inside it.
fn display_path(source: Option<&Path>, path: &str) -> String {
    source
        .and_then(|source| Path::new(path).strip_prefix(source).ok())
        .map_or_else(
            || path.to_owned(),
            |relative| relative.display().to_string(),
        )
}
