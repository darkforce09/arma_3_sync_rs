//! The Repository screen: every file and folder of one repository with what a download does to
//! it, each ticked to be part of the next Update, under the header with the Update button, the
//! check and comparison status, the running or paused download, and the download options.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use a3s_foundation::error::UserFacing as _;
use a3s_types::local::Repository;
use a3s_types::{Changelog, Event};
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme::{self, ProgressTone};
use a3s_ui::widgets::{
    CheckState, CheckTree, CheckTreeEvent, CheckTreeModel, button, primary_button,
};
use a3s_viewer_context::sync::{self, NodeStatus, PlanState, Selection, StatusNode, StatusTree};
use a3s_viewer_context::{
    Change, Command, RepositoryRuntime, TaskKey, TaskKind, ViewerContext, repositories,
};
use egui::{Align, Id, Key, Label, Layout, Pos2, RichText, Ui};

use super::{ScreenCx, ago_text, count_text, date_text, size_text};
use crate::picker::FolderPicker;
use crate::sheets::Sheet;
use crate::sheets::add_repository::protocol_input;
use crate::sheets::download_settings::{keep_folder, local_files_options, set_download_folder};
use crate::shell::{Action, Screen};

/// The state of the Repository screen, kept across frames.
pub(crate) struct RepositoryScreen {
    /// The repository the tree and the ticks belong to.
    repository: String,
    tree: CheckTree,

    /// The status tree the ticks were made on; a tree with other content resets them.
    ticks_tree: Option<Arc<StatusTree>>,

    /// Paths left out of the next Update (folders with everything they hold).
    unticked: BTreeSet<String>,
    context_menu: Option<(String, Pos2)>,
    picker: FolderPicker,
    error: Option<String>,
}

impl Default for RepositoryScreen {
    fn default() -> Self {
        Self::for_repository(String::new())
    }
}

/// What the screen shows, copied out of the state so no guard is held while it runs actions.
struct View {
    repository: Repository,
    url: String,
    runtime: RepositoryRuntime,
    plan: Option<PlanState>,

    /// The comparison is missing or out of date.
    stale: bool,
    paused: Option<u64>,
    events: Vec<Event>,
    changelogs: Vec<Changelog>,

    /// Paths being downloaded right now.
    active_files: Vec<String>,
}

impl View {
    fn take(viewer: &ViewerContext, name: &str) -> Option<Self> {
        let state = viewer.read();
        let repository = state.client.repository(name).ok()?.clone();
        let runtime = state.repositories.runtime(name);
        let downloads = &state.sync.downloads;
        let events = runtime
            .metadata
            .events
            .as_ref()
            .map(|events| events.list.clone())
            .unwrap_or_default();
        let changelogs = runtime
            .metadata
            .changelogs
            .as_ref()
            .map(|changelogs| changelogs.list.clone())
            .unwrap_or_default();
        Some(Self {
            url: state
                .client
                .repository_display_url(name)
                .unwrap_or_default(),
            plan: downloads.plans.get(name).cloned(),
            stale: sync::plan_is_stale(&state, name),
            paused: downloads.paused(name),
            events,
            changelogs,
            active_files: downloads
                .active_files
                .get(name)
                .cloned()
                .unwrap_or_default(),
            runtime,
            repository,
        })
    }

    /// The current comparison's status tree, when it succeeded and is not stale.
    fn tree(&self) -> Option<&Arc<StatusTree>> {
        self.plan
            .as_ref()
            .filter(|_| !self.stale)
            .and_then(|plan| plan.result.as_ref().ok())
            .map(|plan| &plan.tree)
    }
}

/// What runs for the repository this frame.
#[derive(Clone, Copy)]
struct Busy {
    checking: bool,
    comparing: bool,
    downloading: bool,
}

/// What the Update button does and says.
enum Primary {
    /// Download the ticked items: "Download" when nothing is on disk yet; no size before the
    /// first comparison.
    Update {
        verb: &'static str,
        size: Option<u64>,
    },
    NothingSelected,
    Resume(u64),
    Updating(Option<u8>),
    Comparing,
    UpToDate,
}

impl RepositoryScreen {
    fn for_repository(repository: String) -> Self {
        Self {
            tree: CheckTree::new(tree_id(&repository)),
            repository,
            ticks_tree: None,
            unticked: BTreeSet::new(),
            context_menu: None,
            picker: FolderPicker::default(),
            error: None,
        }
    }

    pub(crate) fn show(&mut self, ui: &mut Ui, name: &str, cx: &mut ScreenCx<'_>) {
        if self.repository != name {
            *self = Self::for_repository(name.to_owned());
        }
        let Some(view) = View::take(cx.viewer, name) else {
            super::empty_state(
                ui,
                icons::DATABASE,
                name,
                "This repository no longer exists.",
            );
            return;
        };
        if let Some(folder) = self.picker.poll() {
            self.error = set_download_folder(cx.viewer, name, &folder);
        }
        let downloading = cx
            .viewer
            .is_running(&TaskKey::repository(TaskKind::Download, name));
        let busy = Busy {
            checking: view.runtime.checking
                || cx
                    .viewer
                    .is_running(&TaskKey::new(TaskKind::CheckRepositories))
                    .is_some(),
            comparing: cx
                .viewer
                .is_running(&TaskKey::repository(TaskKind::Compare, name))
                .is_some(),
            downloading: downloading.is_some()
                || cx
                    .viewer
                    .is_running(&TaskKey::repository(TaskKind::AutoUpdate, name))
                    .is_some(),
        };
        compare_when_stale(cx, name, &view, busy);
        self.sync_ticks(view.tree());

        let primary = if let Some(id) = downloading {
            let percent = cx
                .viewer
                .task_snapshot(id)
                .and_then(|snapshot| snapshot.transfer)
                .map(|transfer| transfer.percent());
            Primary::Updating(percent)
        } else if let Some(remaining) = view.paused {
            Primary::Resume(remaining)
        } else {
            match view.tree() {
                Some(tree) if tree.is_up_to_date() => Primary::UpToDate,
                Some(tree) => {
                    let (size, removals) = ticked_work(&tree.roots, &self.unticked);
                    if size == 0 && removals == 0 {
                        Primary::NothingSelected
                    } else {
                        Primary::Update {
                            verb: verb(tree),
                            size: Some(size),
                        }
                    }
                }
                None if busy.comparing => Primary::Comparing,
                None => Primary::Update {
                    verb: "Update",
                    size: None,
                },
            }
        };

        self.header(ui, &view, &primary, busy, cx);
        status_line(ui, &view, busy, cx);
        ui.add_space(6.0);
        super::activity::repository_download(ui, name, cx);
        notices(ui, &view, cx);
        events(ui, &view, cx);
        self.toolbar(ui, &view, busy, cx);
        ui.add_space(4.0);
        self.tree_ui(ui, &view, busy, cx);
        self.context_menu_ui(ui.ctx(), cx);
    }

    /// Keeps the ticks while the status tree has the same content; another tree resets them.
    fn sync_ticks(&mut self, tree: Option<&Arc<StatusTree>>) {
        let same = match (&self.ticks_tree, tree) {
            (Some(kept), Some(tree)) => Arc::ptr_eq(kept, tree) || **kept == **tree,
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.unticked.clear();
        }
        self.ticks_tree = tree.cloned();
    }

    /// What the Update button downloads.
    fn selection(&self) -> Selection {
        if self.unticked.is_empty() {
            Selection::All
        } else {
            Selection::Skip(self.unticked.iter().cloned().collect())
        }
    }

    fn header(
        &mut self,
        ui: &mut Ui,
        view: &View,
        primary: &Primary,
        busy: Busy,
        cx: &mut ScreenCx<'_>,
    ) {
        let palette = theme::palette(ui.ctx());
        let name = view.repository.name.as_str();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icons::DATABASE)
                    .size(36.0)
                    .color(palette.accent_primary),
            );
            ui.vertical(|ui| {
                ui.label(RichText::new(name).heading());
                ui.add(
                    Label::new(RichText::new(&view.url).color(palette.text_secondary)).truncate(),
                );
                ui.label(
                    RichText::new(details(view))
                        .small()
                        .color(palette.text_secondary),
                );
                let folder = view
                    .repository
                    .default_download_location
                    .as_deref()
                    .filter(|folder| !folder.is_empty());
                ui.horizontal(|ui| {
                    let text = folder.map_or_else(
                        || "No download folder".to_owned(),
                        |folder| format!("Downloads to {folder}"),
                    );
                    ui.add(
                        Label::new(RichText::new(text).small().color(palette.text_secondary))
                            .truncate(),
                    );
                    let locked = busy.downloading || view.paused.is_some();
                    let change = ui
                        .add_enabled(!locked, egui::Button::new("Change…").small())
                        .on_disabled_hover_text("Not while a download runs or is paused.");
                    if change.clicked() {
                        self.picker.open(
                            ui.ctx(),
                            "Download folder",
                            folder.map(PathBuf::from).as_deref(),
                        );
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(RichText::new(icons::DOTS_THREE_CIRCLE).size(18.0), |ui| {
                    if ui.button("Edit connection…").clicked() {
                        cx.actions.push(Action::EditRepository(name.to_owned()));
                    }
                    if ui.button("Download settings…").clicked() {
                        cx.actions.push(Action::DownloadSettings(name.to_owned()));
                    }
                    if ui.button("Publish from this computer…").clicked() {
                        cx.actions
                            .push(Action::Navigate(Screen::Publishing(name.to_owned())));
                    }
                    ui.separator();
                    if ui.button("Remove repository").clicked() {
                        remove(cx, view);
                    }
                });
                self.primary_action(ui, name, primary, cx);
            });
        });
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(palette.status_danger));
        }
    }

    fn primary_action(&self, ui: &mut Ui, name: &str, primary: &Primary, cx: &mut ScreenCx<'_>) {
        match primary {
            Primary::NothingSelected => {
                ui.add_enabled(false, egui::Button::new("Nothing selected"));
            }
            Primary::Update { verb, size } => {
                let label = match size {
                    Some(size) => format!("{verb} {}", size_text(*size)),
                    None => (*verb).to_owned(),
                };
                if primary_button(ui, &label).clicked() {
                    send(
                        cx,
                        Command::Sync(sync::Command::Download {
                            repository: name.to_owned(),
                            event: None,
                            selection: self.selection(),
                        }),
                    );
                }
            }
            Primary::Resume(remaining) => {
                if primary_button(ui, &format!("Resume {}", size_text(*remaining))).clicked() {
                    send(
                        cx,
                        Command::Sync(sync::Command::Resume {
                            repository: name.to_owned(),
                        }),
                    );
                }
            }
            Primary::Updating(percent) => {
                let label = match percent {
                    Some(percent) => format!("Updating… {percent}%"),
                    None => "Updating…".to_owned(),
                };
                if button(ui, &label).clicked() {
                    cx.actions.push(Action::Navigate(Screen::Activity));
                }
            }
            Primary::Comparing => {
                ui.add_enabled(false, egui::Button::new("Comparing…"));
            }
            Primary::UpToDate => {
                ui.add_enabled(
                    false,
                    egui::Button::new(format!("{} Up to date", icons::CHECK_CIRCLE)),
                );
            }
        }
    }

    /// Select all / none, expand or collapse every folder, the local files options and the
    /// Download settings.
    fn toolbar(&mut self, ui: &mut Ui, view: &View, busy: Busy, cx: &mut ScreenCx<'_>) {
        let tree = view.tree().cloned();
        let locked = busy.downloading || view.paused.is_some();
        ui.horizontal_wrapped(|ui| {
            let selectable = tree.as_ref().is_some_and(|tree| !tree.roots.is_empty()) && !locked;
            ui.add_enabled_ui(selectable, |ui| {
                if ui.small_button("Select all").clicked() {
                    self.unticked.clear();
                }
                if ui.small_button("Select none").clicked()
                    && let Some(tree) = &tree
                {
                    self.unticked = tree.roots.iter().map(|node| node.path.clone()).collect();
                }
            });
            ui.add_enabled_ui(tree.is_some(), |ui| {
                let all_folders: Vec<String> =
                    tree.as_ref().map(|tree| folders(tree)).unwrap_or_default();
                let expanded = tree.as_ref().is_some_and(|tree| {
                    tree.roots
                        .iter()
                        .filter(|node| node.is_folder)
                        .all(|node| self.tree.is_expanded(ui.ctx(), &node.path))
                });
                let label = if expanded && !all_folders.is_empty() {
                    "Collapse all"
                } else {
                    "Expand all"
                };
                if ui.small_button(label).clicked() {
                    if label == "Expand all" {
                        CheckTree::expand_all(
                            ui.ctx(),
                            tree_id(&self.repository),
                            all_folders.iter(),
                        );
                    } else {
                        for folder in &all_folders {
                            CheckTree::set_expanded(
                                ui.ctx(),
                                tree_id(&self.repository),
                                folder,
                                false,
                            );
                        }
                    }
                }
            });
            ui.add_space(8.0);
            if let Some(error) = local_files_options(ui, cx.viewer, &view.repository, !locked) {
                self.error = Some(error);
            }
            ui.add_space(8.0);
            if ui.small_button("Settings…").clicked() {
                cx.actions
                    .push(Action::DownloadSettings(view.repository.name.clone()));
            }
        });
    }

    /// The repository's files and folders with their status; a message until the first
    /// comparison.
    fn tree_ui(&mut self, ui: &mut Ui, view: &View, busy: Busy, cx: &ScreenCx<'_>) {
        let palette = theme::palette(ui.ctx());
        let Some(tree) = view.tree().cloned() else {
            frames::group(ui, |ui| {
                let text = match &view.plan {
                    _ if busy.comparing => "Comparing with your download folder…".to_owned(),
                    Some(PlanState {
                        result: Err(error), ..
                    }) => error.message.clone(),
                    _ => "Not compared with your download folder yet.".to_owned(),
                };
                ui.label(RichText::new(text).color(palette.text_secondary));
            });
            return;
        };
        let locked = busy.downloading || view.paused.is_some();
        let model = TreeView {
            tree: &tree,
            unticked: &self.unticked,
            search: cx.search,
            active: &view.active_files,
        };
        let events = frames::group(ui, |ui| self.tree.show(ui, &model)).inner;
        for event in events {
            match event {
                CheckTreeEvent::Toggled(path) if !locked && !under_removed(&tree, &path) => {
                    let ticked = tick_state(&self.unticked, &path) == CheckState::Unchecked;
                    set_ticked(&tree, &mut self.unticked, &path, ticked);
                }
                CheckTreeEvent::ContextMenu(path, pos)
                    if !path.contains('/')
                        && tree
                            .get(&path)
                            .is_some_and(|node| node.status == NodeStatus::Removed) =>
                {
                    self.context_menu = Some((path, pos));
                }
                _ => {}
            }
        }
    }

    /// "Keep this folder" on a removed top-level folder.
    fn context_menu_ui(&mut self, ctx: &egui::Context, cx: &mut ScreenCx<'_>) {
        let Some((path, pos)) = self.context_menu.clone() else {
            return;
        };
        let mut close = false;
        let area = egui::Area::new(Id::new("a3s_viewer::repository::context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    if ui.button("Keep this folder").clicked() {
                        keep(cx, &self.repository, &path);
                        close = true;
                    }
                });
            });
        let clicked_outside =
            ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
        if close || clicked_outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.context_menu = None;
        }
    }
}

fn tree_id(repository: &str) -> Id {
    Id::new(("a3s_viewer::repository::tree", repository))
}

/// `Revision 3 · built Oct 16, 2026 · 14 mods`.
fn details(view: &View) -> String {
    let metadata = &view.runtime.metadata;
    let revision = metadata
        .server_revision()
        .unwrap_or(view.repository.revision);
    let mut details = format!("Revision {revision}");
    if let Some(date) = metadata
        .server_info
        .as_ref()
        .and_then(|info| info.build_date)
        .and_then(date_text)
    {
        details.push_str(&format!(" · built {date}"));
    }
    if let Some(mods) = repositories::addon_count(metadata) {
        details.push_str(&format!(" · {}", count_text(mods, "mod", "mods")));
    }
    details
}

/// "Download" when nothing of the repository is on disk yet, else "Update".
fn verb(tree: &StatusTree) -> &'static str {
    if tree
        .roots
        .iter()
        .all(|node| node.status == NodeStatus::NotDownloaded)
    {
        "Download"
    } else {
        "Update"
    }
}

/// The check, comparison or result line under the header, with Check now and Changelog.
fn status_line(ui: &mut Ui, view: &View, busy: Busy, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let name = view.repository.name.as_str();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if busy.checking || busy.comparing {
            ui.add(egui::Spinner::new().size(14.0));
            let text = if busy.checking {
                "Checking the repository…"
            } else {
                "Comparing with your download folder…"
            };
            ui.label(text);
        } else if let Some(tree) = view.tree() {
            let (icon, color) = if tree.is_up_to_date() {
                (icons::CHECK_CIRCLE, palette.status_success)
            } else {
                (icons::ARROW_CIRCLE_DOWN, palette.status_info)
            };
            ui.label(RichText::new(icon).color(color));
            let mut text = String::new();
            if let Some(checked) = view.runtime.last_checked {
                text.push_str(&format!(
                    "Checked {} · ",
                    ago_text(checked, std::time::SystemTime::now())
                ));
            }
            text.push_str(&summary(tree));
            ui.add(Label::new(text).truncate());
        } else {
            ui.label(
                RichText::new("Not compared with your download folder yet.")
                    .color(palette.text_secondary),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !view.changelogs.is_empty() && button(ui, "Changelog").clicked() {
                cx.sheets.push(Sheet::changelog(
                    name,
                    view.repository.revision,
                    view.changelogs.clone(),
                ));
            }
            let idle = !(busy.checking || busy.comparing || busy.downloading);
            let check = ui
                .add_enabled(
                    idle,
                    egui::Button::new(format!("{} Check now", icons::ARROWS_CLOCKWISE)),
                )
                .on_hover_text("Check the repository and compare it with your download folder");
            if check.clicked() {
                send(
                    cx,
                    Command::Repositories(repositories::Command::Check {
                        names: vec![name.to_owned()],
                    }),
                );
            }
        });
    });
}

/// `1 to update (2 files, 237.05 KB) · 14 up to date`, or that everything is up to date.
fn summary(tree: &StatusTree) -> String {
    if tree.is_up_to_date() {
        return "everything is up to date".to_owned();
    }
    let totals = tree.totals();
    let (mut update_size, mut update_files, mut new_size) = (0_u64, 0_usize, 0_u64);
    for node in &tree.roots {
        match node.status {
            NodeStatus::Update => {
                update_size = update_size.saturating_add(node.download_size);
                update_files += node.files_to_download;
            }
            NodeStatus::NotDownloaded => new_size = new_size.saturating_add(node.download_size),
            NodeStatus::UpToDate | NodeStatus::Removed => {}
        }
    }
    let mut parts = Vec::new();
    if totals.to_update > 0 {
        let mut part = format!("{} to update", totals.to_update);
        if update_files > 0 {
            part.push_str(&format!(
                " ({}, {})",
                count_text(update_files, "file", "files"),
                size_text(update_size)
            ));
        }
        parts.push(part);
    }
    if totals.not_downloaded > 0 {
        parts.push(format!(
            "{} not downloaded ({})",
            totals.not_downloaded,
            size_text(new_size)
        ));
    }
    if totals.removed > 0 {
        parts.push(format!("{} to remove", totals.removed));
    }
    if totals.up_to_date > 0 {
        parts.push(format!("{} up to date", totals.up_to_date));
    }
    parts.join(" · ")
}

/// Compares the repository with its folder once its metadata is known and the last comparison
/// is missing or out of date.
fn compare_when_stale(cx: &ScreenCx<'_>, name: &str, view: &View, busy: Busy) {
    let checked = view.runtime.metadata.server_info.is_some() && !busy.checking;
    if busy.comparing || busy.downloading || !checked || view.paused.is_some() || !view.stale {
        return;
    }
    if let Err(err) = sync::compare(cx.viewer.cx(), name) {
        tracing::warn!("Cannot compare repository {name}: {err}");
    }
}

/// A failed check and a failed comparison; failed files of the last download are on its result
/// row.
fn notices(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let name = view.repository.name.as_str();
    if let Some(error) = &view.runtime.last_check_error
        && !view.runtime.checking
    {
        ui.add_space(8.0);
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARNING).color(palette.status_danger));
                ui.add(
                    Label::new(format!("Cannot check the repository: {}", error.message)).wrap(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Edit connection…").clicked() {
                        cx.actions.push(Action::EditRepository(name.to_owned()));
                    }
                    if button(ui, "Check again").clicked() {
                        send(
                            cx,
                            Command::Repositories(repositories::Command::Check {
                                names: vec![name.to_owned()],
                            }),
                        );
                    }
                });
            });
        });
    }
    if let Some(PlanState {
        result: Err(error), ..
    }) = &view.plan
    {
        ui.add_space(8.0);
        frames::group(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::WARNING).color(palette.status_warning));
                ui.add(Label::new(&error.message).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Edit connection…").clicked() {
                        cx.actions.push(Action::EditRepository(name.to_owned()));
                    }
                });
            });
        });
    }
}

/// One row per event of the repository, with Download and Use preset.
fn events(ui: &mut Ui, view: &View, cx: &mut ScreenCx<'_>) {
    if view.events.is_empty() {
        return;
    }
    let palette = theme::palette(ui.ctx());
    ui.add_space(8.0);
    frames::group(ui, |ui| {
        for (index, event) in view.events.iter().enumerate() {
            if index > 0 {
                frames::group_separator(ui);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(icons::CALENDAR_BLANK).color(palette.accent_primary));
                ui.label(RichText::new(&event.name).strong());
                ui.label(
                    RichText::new(count_text(event.addon_names.len(), "mod", "mods"))
                        .small()
                        .color(palette.text_secondary),
                );
                if let Some(description) = event.description.as_deref().filter(|d| !d.is_empty()) {
                    ui.add(
                        Label::new(RichText::new(description).color(palette.text_secondary))
                            .truncate(),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Use preset")
                        .on_hover_text("Enable exactly the mods of this event")
                        .clicked()
                    {
                        super::mods::use_event_preset(
                            cx.viewer,
                            cx.toasts,
                            &view.repository.name,
                            event,
                        );
                    }
                    if button(ui, "Download")
                        .on_hover_text("Download only the mods of this event")
                        .clicked()
                    {
                        send(
                            cx,
                            Command::Sync(sync::Command::Download {
                                repository: view.repository.name.clone(),
                                event: Some(event.name.clone()),
                                selection: Selection::All,
                            }),
                        );
                    }
                });
            });
        }
    });
}

/// The status tree as the check tree shows it.
struct TreeView<'a> {
    tree: &'a StatusTree,
    unticked: &'a BTreeSet<String>,
    search: &'a str,
    active: &'a [String],
}

impl TreeView<'_> {
    fn node(&self, path: &str) -> Option<&StatusNode> {
        self.tree.get(path)
    }

    /// Whether `path` is being downloaded, or holds a file that is.
    fn is_active(&self, path: &str) -> bool {
        self.active
            .iter()
            .any(|active| active == path || is_below(active, path))
    }
}

impl CheckTreeModel for TreeView<'_> {
    type Key = String;

    fn roots(&self) -> Vec<String> {
        self.tree
            .roots
            .iter()
            .filter(|node| super::matches_search(self.search, &node.name))
            .map(|node| node.path.clone())
            .collect()
    }

    fn children(&self, key: &String) -> Vec<String> {
        self.node(key)
            .map(|node| {
                node.children
                    .iter()
                    .map(|child| child.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_leaf(&self, key: &String) -> bool {
        self.node(key).is_some_and(|node| !node.is_folder)
    }

    fn label(&self, key: &String) -> String {
        match self.node(key) {
            Some(node) if node.hidden => format!("{} (kept)", node.name),
            Some(node) => node.name.clone(),
            None => key.clone(),
        }
    }

    fn check_state(&self, key: &String) -> CheckState {
        tick_state(self.unticked, key)
    }

    fn pill(&self, key: &String) -> Option<(String, ProgressTone)> {
        let node = self.node(key)?;
        if node.status != NodeStatus::UpToDate && self.is_active(key) {
            return Some((
                format!("{} Downloading", icons::ARROW_CIRCLE_DOWN),
                ProgressTone::Info,
            ));
        }
        Some(match node.status {
            NodeStatus::UpToDate => (
                format!("{} Up to date", icons::CHECK_CIRCLE),
                ProgressTone::Success,
            ),
            NodeStatus::Update => (
                format!("{} Update", icons::ARROW_CIRCLE_DOWN),
                ProgressTone::Info,
            ),
            NodeStatus::NotDownloaded => (
                format!("{} Not downloaded", icons::CLOUD_ARROW_DOWN),
                ProgressTone::Warning,
            ),
            NodeStatus::Removed => (format!("{} Removed", icons::TRASH), ProgressTone::Danger),
        })
    }

    fn detail(&self, key: &String) -> Option<String> {
        let node = self.node(key)?;
        match node.status {
            NodeStatus::UpToDate => None,
            NodeStatus::Removed => Some("to the Trash".to_owned()),
            NodeStatus::Update | NodeStatus::NotDownloaded => {
                if node.is_folder && node.files_to_download > 0 {
                    Some(format!(
                        "{} · {}",
                        count_text(node.files_to_download, "file", "files"),
                        size_text(node.download_size)
                    ))
                } else if node.download_size > 0 {
                    Some(size_text(node.download_size))
                } else {
                    None
                }
            }
        }
    }

    fn tooltip(&self, key: &String) -> Option<String> {
        let node = self.node(key)?;
        if node.status == NodeStatus::Removed && !key.contains('/') {
            Some(
                "No longer in the repository: Update moves it to the Trash. Untick it to keep it \
                 this time, or right-click to keep it for good."
                    .to_owned(),
            )
        } else {
            Some(key.clone())
        }
    }
}

/// Unchecked when `path` or a folder above it is unticked, indeterminate when something below
/// it is, else checked.
fn tick_state(unticked: &BTreeSet<String>, path: &str) -> CheckState {
    if unticked.contains(path) || ancestors(path).any(|ancestor| unticked.contains(ancestor)) {
        CheckState::Unchecked
    } else if unticked.iter().any(|other| is_below(other, path)) {
        CheckState::Indeterminate
    } else {
        CheckState::Checked
    }
}

/// Ticks or unticks `path` with everything below it. An unticked folder above a ticked path
/// becomes its other children; a folder whose children are all unticked is unticked as a
/// whole, so a download never creates it empty.
fn set_ticked(tree: &StatusTree, unticked: &mut BTreeSet<String>, path: &str, ticked: bool) {
    unticked.retain(|other| !is_below(other, path));
    if ticked {
        unticked.remove(path);
        let ancestor = ancestors(path)
            .find(|ancestor| unticked.contains(*ancestor))
            .map(str::to_owned);
        if let Some(ancestor) = ancestor {
            unticked.remove(&ancestor);
            let mut current = tree.get(&ancestor);
            while let Some(node) = current {
                current = None;
                for child in &node.children {
                    if child.path == path {
                        continue;
                    }
                    if is_below(path, &child.path) {
                        current = Some(child);
                    } else {
                        unticked.insert(child.path.clone());
                    }
                }
            }
        }
    } else {
        unticked.insert(path.to_owned());
    }
    let parents: Vec<&str> = ancestors(path).collect();
    for parent in parents.into_iter().rev() {
        let Some(node) = tree.get(parent) else {
            continue;
        };
        if !node.children.is_empty()
            && node
                .children
                .iter()
                .all(|child| unticked.contains(&child.path))
        {
            for child in &node.children {
                unticked.remove(&child.path);
            }
            unticked.insert(parent.to_owned());
        }
    }
}

/// The bytes to download and the files to remove of the ticked part of `nodes`.
fn ticked_work(nodes: &[StatusNode], unticked: &BTreeSet<String>) -> (u64, usize) {
    let mut size = 0_u64;
    let mut removals = 0_usize;
    for node in nodes {
        if unticked.contains(&node.path) {
            continue;
        }
        if node.is_folder && node.status != NodeStatus::Removed && !node.children.is_empty() {
            let (folder_size, folder_removals) = ticked_work(&node.children, unticked);
            size = size.saturating_add(folder_size);
            removals += folder_removals;
        } else {
            size = size.saturating_add(node.download_size);
            removals += node.files_to_remove;
        }
    }
    (size, removals)
}

/// Every folder path of `tree`.
fn folders(tree: &StatusTree) -> Vec<String> {
    fn walk(nodes: &[StatusNode], out: &mut Vec<String>) {
        for node in nodes.iter().filter(|node| node.is_folder) {
            out.push(node.path.clone());
            walk(&node.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&tree.roots, &mut out);
    out
}

/// Whether a folder above `path` is removed: its content goes with it.
fn under_removed(tree: &StatusTree, path: &str) -> bool {
    ancestors(path).any(|ancestor| {
        tree.get(ancestor)
            .is_some_and(|node| node.status == NodeStatus::Removed)
    })
}

/// The folders above `path`, outermost first: `a`, `a/b` for `a/b/c`.
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(|(index, _)| &path[..index])
}

/// Whether `path` is strictly below `folder`.
fn is_below(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Keeps the folder `folder` of `repository` when updating (its hidden folders), with Undo; the
/// comparison runs again without it.
// Mirrors ui/repository/DownloadPanel.java:749-765
fn keep(cx: &mut ScreenCx<'_>, repository: &str, folder: &str) {
    let kept = cx.viewer.mutate(Some(Change::Repository), |state| {
        let added = keep_folder(&mut state.client, repository, folder)?;
        sync::forget(state, repository);
        Ok::<bool, a3s_repo_files::services::ServiceError>(added)
    });
    match kept {
        Ok(_) => {
            let (repository, folder) = (repository.to_owned(), folder.to_owned());
            cx.toasts.undoable(
                format!("Keeping {folder} when updating"),
                move |viewer: &ViewerContext| {
                    let undone = viewer.mutate(Some(Change::Repository), |state| {
                        state.client.update_repository(&repository, |stored| {
                            Ok(stored.hidden_folder_paths.remove(&folder))
                        })?;
                        sync::forget(state, &repository);
                        Ok::<(), a3s_repo_files::services::ServiceError>(())
                    });
                    if let Err(err) = undone {
                        tracing::warn!("Undo failed: {}", err.user_message());
                    }
                },
            );
        }
        Err(err) => cx.toasts.error(err.user_message()),
    }
}

/// Removes the repository, with an Undo toast that stores it again.
fn remove(cx: &mut ScreenCx<'_>, view: &View) {
    let name = view.repository.name.clone();
    match cx.viewer.remove_repository(&name) {
        Ok(_) => {
            cx.viewer
                .mutate(None, |state| {
                    sync::forget(state, &name);
                    Ok::<(), Infallible>(())
                })
                .ok();
            let repository = view.repository.clone();
            cx.toasts
                .undoable(format!("Removed {name}"), move |viewer: &ViewerContext| {
                    restore(viewer, repository);
                });
            cx.actions.push(Action::Navigate(Screen::Mods));
        }
        Err(err) => cx.toasts.error(err.user_message()),
    }
}

/// Stores a removed repository again, with all its settings, and checks it.
fn restore(viewer: &ViewerContext, repository: Repository) {
    let name = repository.name.clone();
    let restored = viewer.mutate(Some(Change::Repository), |state| {
        state
            .client
            .create_repository(&name, &protocol_input(&repository.protocol))?;
        state.client.update_repository(&name, |stored| {
            *stored = repository;
            Ok(true)
        })
    });
    match restored {
        Ok(_) => {
            let check = Command::Repositories(repositories::Command::Check { names: vec![name] });
            if let Err(err) = viewer.send(check) {
                tracing::warn!("Cannot check the restored repository: {err}");
            }
        }
        Err(err) => tracing::warn!("Cannot restore repository {name}: {}", err.user_message()),
    }
}

fn send(cx: &mut ScreenCx<'_>, command: Command) {
    if let Err(err) = cx.viewer.send(command) {
        cx.toasts.error(err.user_message());
    }
}
