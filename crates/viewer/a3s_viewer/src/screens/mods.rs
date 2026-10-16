//! The Mods screen: the active profile's addon groups and the addons on this computer with
//! their status, the load order, the event presets and Arma 3 Launcher presets dropped on it.

mod edit;
mod load_order;
mod model;

use std::path::PathBuf;
use std::sync::mpsc;

use a3s_addons::add_groups_from_repository;
use a3s_foundation::error::UserFacing as _;
use a3s_foundation::format;
use a3s_tools::preset::{ImportResult, import_preset};
use a3s_types::Event;
use a3s_types::local::{Addon, Profile, TreeNode};
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::{CheckTree, CheckTreeEvent, segmented};
use a3s_viewer_context::launch::launch_order;
use a3s_viewer_context::{Change, ViewerContext, profiles};
use egui::{Align, Key, Layout, Pos2, RichText, Ui};

use self::model::{ModNode, ModsModel, TreeView};
use super::ScreenCx;
use crate::picker::FolderPicker;
use crate::sheets::Sheet;
use crate::toasts::Toaster;

/// The segments of the screen.
const SEGMENTS: [&str; 3] = ["All", "Enabled", "Load order"];

/// The segment showing the load order.
const LOAD_ORDER: usize = 2;

/// The state of the Mods screen, kept across frames.
pub(crate) struct ModsScreen {
    segment: usize,
    tree: CheckTree,
    model: Option<ModsModel>,
    picker: FolderPicker,
    context_menu: Option<(ModNode, Pos2)>,

    /// The group being renamed and the text typed so far.
    rename: Option<(String, String)>,

    /// The preset being read and compared on a worker thread.
    preset: Option<mpsc::Receiver<Result<ImportResult, String>>>,
}

impl Default for ModsScreen {
    fn default() -> Self {
        Self {
            segment: 0,
            tree: CheckTree::new("a3s_viewer::mods::tree").default_expanded(true),
            model: None,
            picker: FolderPicker::default(),
            context_menu: None,
            rename: None,
            preset: None,
        }
    }
}

impl ModsScreen {
    /// Shows the screen.
    pub(crate) fn show(&mut self, ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        {
            let state = cx.viewer.read();
            match &mut self.model {
                Some(model) => {
                    model.refresh(&state);
                }
                None => self.model = ModsModel::take(&state),
            }
        }
        self.poll_background(cx);
        self.take_dropped_preset(ui.ctx(), cx);
        if self.model.is_none() {
            super::empty_state(
                ui,
                icons::PUZZLE_PIECE,
                "No profile",
                "Create a profile in Settings to manage mods.",
            );
            return;
        }

        egui::Panel::bottom("a3s_viewer::mods::footer")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 6)))
            .show(ui, |ui| self.footer(ui, cx));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    segmented(ui, &mut self.segment, &SEGMENTS);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        Self::event_preset_menu(ui, cx);
                    });
                });
                ui.add_space(8.0);
                if self.segment == LOAD_ORDER {
                    self.load_order(ui, cx);
                } else {
                    self.tree_ui(ui, cx);
                }
            });
        self.context_menu_ui(ui.ctx(), cx);
        self.rename_ui(ui.ctx(), cx);
        paint_drop_hint(ui);
    }

    fn model(&self) -> &ModsModel {
        self.model
            .as_ref()
            .expect("the model exists while the screen shows")
    }

    fn tree_ui(&mut self, ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        let Some(model) = &self.model else {
            return;
        };
        let view = TreeView::new(model, cx.search, self.segment == 1);
        let events = self.tree.show(ui, &view);
        for event in events {
            match event {
                CheckTreeEvent::Toggled(node) => self.toggle(cx, &node),
                CheckTreeEvent::RowDoubleClicked(node) => self.start_rename(&node),
                CheckTreeEvent::ContextMenu(node, pos) => self.context_menu = Some((node, pos)),
                CheckTreeEvent::Key(node, Key::Delete) => self.remove(cx, &node),
                _ => {}
            }
        }
    }

    fn load_order(&self, ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        let order = enabled_order(cx.viewer);
        let rows = load_order::rows(&order, self.model().scan.as_deref());
        if let Some(new_order) = load_order::show(ui, &rows, cx.search) {
            let before = edit_profile(cx.viewer, cx.toasts, Change::AddonPriority, |profile| {
                edit::reorder_enabled(&mut profile.addon_names_by_priority, &new_order)
            });
            if let Some(before) = before {
                cx.toasts.undoable(
                    "Changed the load order",
                    undo_profile(before, Change::AddonPriority),
                );
            }
        }
    }

    fn toggle(&self, cx: &mut ScreenCx<'_>, node: &ModNode) {
        let model = self.model();
        match node {
            ModNode::Group(path) => {
                let path = path.clone();
                edit_profile(cx.viewer, cx.toasts, Change::AddonSelection, |profile| {
                    edit::toggle(&mut profile.tree, &path)
                });
            }
            ModNode::Computer | ModNode::Folder(_) | ModNode::Local(_) => {
                let enable = !model.is_checked(node);
                let keys = model.local_keys(node);
                edit_profile(cx.viewer, cx.toasts, Change::AddonSelection, |profile| {
                    let mut changed = false;
                    for key in &keys {
                        changed |= edit::set_loose(&mut profile.tree, key, enable);
                    }
                    changed
                });
            }
        }
    }

    /// The row the tree has selected, if it is shown.
    fn selected_node(&self) -> Option<ModNode> {
        let model = self.model.as_ref()?;
        let view = TreeView::new(model, "", false);
        let mut pending = a3s_ui::widgets::CheckTreeModel::roots(&view);
        while let Some(node) = pending.pop() {
            if self.tree.is_selected(&node) {
                return Some(node);
            }
            pending.extend(a3s_ui::widgets::CheckTreeModel::children(&view, &node));
        }
        None
    }

    /// Whether `node` can be removed with "−", Delete or its menu.
    fn removable(&self, node: &ModNode) -> bool {
        let model = self.model();
        match node {
            ModNode::Group(path) if path.len() == 1 => true,
            ModNode::Group(_) => model
                .top_group(node)
                .is_some_and(|group| group.modset_type.is_none()),
            ModNode::Folder(Some(_)) => true,
            ModNode::Computer | ModNode::Folder(None) | ModNode::Local(_) => false,
        }
    }

    /// Removes a group, an addon of a user group or a search folder; Undo puts it back.
    fn remove(&self, cx: &mut ScreenCx<'_>, node: &ModNode) {
        if !self.removable(node) {
            return;
        }
        let label = self.model().label(node);
        let (text, change) = match node {
            ModNode::Folder(_) => (format!("Removed the folder {label}"), Change::AddonFiles),
            ModNode::Group(path) if path.len() == 1 => {
                (format!("Removed the group {label}"), Change::Group)
            }
            _ => (format!("Removed {label}"), Change::AddonSelection),
        };
        let node = node.clone();
        let folders = self.model().folders().to_vec();
        let before = edit_profile(cx.viewer, cx.toasts, change, |profile| match &node {
            ModNode::Group(path) => remove_at(&mut profile.tree, path),
            ModNode::Folder(Some(index)) => {
                let Some(folder) = folders.get(*index) else {
                    return false;
                };
                let position = profile
                    .addon_search_directories
                    .iter()
                    .position(|d| d == folder);
                position.is_some_and(|position| {
                    profile.addon_search_directories.remove(position);
                    true
                })
            }
            _ => false,
        });
        if let Some(before) = before {
            cx.toasts.undoable(text, undo_profile(before, change));
        }
    }

    fn start_rename(&mut self, node: &ModNode) {
        let ModNode::Group(path) = node else {
            return;
        };
        if path.len() != 1 {
            return;
        }
        if let Some(group) = self.model().top_group(node)
            && group.modset_type.is_none()
        {
            self.rename = Some((group.name.clone(), group.name.clone()));
        }
    }

    fn footer(&mut self, ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        let palette = theme::palette(ui.ctx());
        ui.horizontal(|ui| {
            ui.menu_button(RichText::new(icons::PLUS).size(15.0), |ui| {
                if ui.button("New group").clicked() {
                    let mut name = None;
                    edit_profile(cx.viewer, cx.toasts, Change::Group, |profile| {
                        name = Some(edit::add_group(&mut profile.tree));
                        true
                    });
                    if let Some(name) = name {
                        self.rename = Some((name.clone(), name));
                    }
                }
                if ui.button("Add mods folder…").clicked() {
                    let start = cx
                        .viewer
                        .read()
                        .client
                        .last_added_addon_search_directory()
                        .map(PathBuf::from);
                    self.picker
                        .open(ui.ctx(), "Choose a mods folder", start.as_deref());
                }
                Self::repository_groups_menu(ui, cx);
            })
            .response
            .on_hover_text("Add a group or a mods folder");
            let selected = self.selected_node().filter(|node| self.removable(node));
            let remove = ui
                .add_enabled(
                    selected.is_some(),
                    egui::Button::new(RichText::new(icons::MINUS).size(15.0)),
                )
                .on_hover_text("Remove the selected group or folder");
            if remove.clicked()
                && let Some(node) = selected
            {
                self.remove(cx, &node);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let order = enabled_order(cx.viewer);
                let size: u64 = self
                    .model()
                    .scan
                    .as_ref()
                    .map_or(0, |scan| order.iter().map(|key| scan.size(key)).sum());
                let scanning = cx
                    .viewer
                    .is_running(&a3s_viewer_context::TaskKey::new(
                        a3s_viewer_context::TaskKind::AddonScan,
                    ))
                    .is_some();
                let mut text = format!("{} enabled · {}", order.len(), format::size(size as i64));
                if scanning {
                    text.push_str(" · scanning…");
                }
                ui.label(RichText::new(text).color(palette.text_secondary));
            });
        });
    }

    fn repository_groups_menu(ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        let repositories: Vec<String> = {
            let state = cx.viewer.read();
            state
                .client
                .repositories()
                .filter(|r| state.repositories.runtime(&r.name).metadata.sync.is_some())
                .map(|r| r.name.clone())
                .collect()
        };
        if repositories.is_empty() {
            return;
        }
        ui.menu_button("Group from repository", |ui| {
            for name in repositories {
                if ui.button(&name).clicked() {
                    let sync = cx
                        .viewer
                        .read()
                        .repositories
                        .runtime(&name)
                        .metadata
                        .sync
                        .clone();
                    edit_profile(cx.viewer, cx.toasts, Change::Group, |profile| {
                        add_groups_from_repository(
                            &mut profile.tree,
                            &[(name.as_str(), sync.as_deref())],
                            false,
                        );
                        true
                    });
                }
            }
        });
    }

    fn event_preset_menu(ui: &mut Ui, cx: &mut ScreenCx<'_>) {
        let events: Vec<(String, Event)> = {
            let state = cx.viewer.read();
            state
                .client
                .repositories()
                .flat_map(|repository| {
                    state
                        .repositories
                        .runtime(&repository.name)
                        .metadata
                        .events_list()
                        .into_iter()
                        .map(|event| (repository.name.clone(), event.clone()))
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        ui.add_enabled_ui(!events.is_empty(), |ui| {
            ui.menu_button(format!("{} Event preset", icons::CALENDAR_DOTS), |ui| {
                for (repository, event) in &events {
                    if ui
                        .button(format!("{} ({repository})", event.name))
                        .clicked()
                    {
                        use_event_preset(cx.viewer, cx.toasts, repository, event);
                    }
                }
            })
            .response
            .on_hover_text("Enable exactly the mods of an event");
        });
    }

    fn context_menu_ui(&mut self, ctx: &egui::Context, cx: &mut ScreenCx<'_>) {
        let Some((node, pos)) = self.context_menu.clone() else {
            return;
        };
        let mut close = false;
        let area = egui::Area::new(egui::Id::new("a3s_viewer::mods::context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(160.0);
                    close |= self.context_menu_items(ui, cx, &node);
                });
            });
        let clicked_outside =
            ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
        if close || clicked_outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.context_menu = None;
        }
    }

    /// The items of the context menu of `node`; returns whether one was chosen.
    fn context_menu_items(&mut self, ui: &mut Ui, cx: &mut ScreenCx<'_>, node: &ModNode) -> bool {
        let mut chosen = false;
        if let ModNode::Local(key) = node {
            for group in self.model().user_groups() {
                if ui.button(format!("Add to {group}")).clicked() {
                    edit_profile(cx.viewer, cx.toasts, Change::AddonSelection, |profile| {
                        edit::add_to_group(&mut profile.tree, &group, key)
                    });
                    chosen = true;
                }
            }
        }
        if let ModNode::Group(path) = node
            && path.len() == 1
            && self
                .model()
                .top_group(node)
                .is_some_and(|group| group.modset_type.is_none())
            && ui.button("Rename…").clicked()
        {
            self.start_rename(node);
            chosen = true;
        }
        if self.removable(node) && ui.button("Remove").clicked() {
            self.remove(cx, node);
            chosen = true;
        }
        chosen
    }

    fn rename_ui(&mut self, ctx: &egui::Context, cx: &mut ScreenCx<'_>) {
        let Some((old, text)) = &mut self.rename else {
            return;
        };
        let mut done = None;
        egui::Modal::new(egui::Id::new("a3s_viewer::mods::rename")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(RichText::new("Rename group").strong());
            ui.add_space(6.0);
            let field = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if a3s_ui::widgets::primary_button(ui, "Rename").clicked() || enter {
                    done = Some(true);
                }
                if a3s_ui::widgets::button(ui, "Cancel").clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                let (old, new) = (old.clone(), text.clone());
                edit_profile(cx.viewer, cx.toasts, Change::Group, |profile| {
                    edit::rename_group(&mut profile.tree, &old, &new)
                });
                self.rename = None;
            }
            Some(false) => self.rename = None,
            None => {}
        }
    }

    /// Takes the chosen mods folder and the compared preset once they are ready.
    fn poll_background(&mut self, cx: &mut ScreenCx<'_>) {
        if let Some(folder) = self.picker.poll() {
            let folder = folder.display().to_string();
            let added = cx.viewer.mutate(Some(Change::AddonFiles), |state| {
                if state.client.addon_search_directories().contains(&folder) {
                    return Ok(false);
                }
                state.client.add_addon_search_directory(folder.clone())?;
                state
                    .client
                    .set_last_added_addon_search_directory(Some(folder.clone()))?;
                Ok::<_, a3s_repo_files::services::ServiceError>(true)
            });
            if let Err(err) = added {
                cx.toasts.error(err.user_message());
            }
        }
        if let Some(receiver) = &self.preset {
            match receiver.try_recv() {
                Ok(Ok(result)) => {
                    self.preset = None;
                    cx.sheets.push(Sheet::LauncherPreset(Box::new(result)));
                }
                Ok(Err(message)) => {
                    self.preset = None;
                    cx.toasts.error(message);
                }
                Err(mpsc::TryRecvError::Disconnected) => self.preset = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Reads a dropped `.html` preset on a worker thread (it reads every mod's `meta.cpp`).
    fn take_dropped_preset(&mut self, ctx: &egui::Context, cx: &mut ScreenCx<'_>) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        let Some(path) = dropped.into_iter().find(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
        }) else {
            return;
        };
        let addons: Vec<Addon> = self
            .model
            .as_ref()
            .and_then(|model| model.scan.as_ref())
            .map(|scan| scan.registry.addons().cloned().collect())
            .unwrap_or_default();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("a3s-preset-import".to_owned())
            .spawn(move || {
                let result = import_preset(&path, addons.iter()).map_err(|err| err.to_string());
                sender.send(result).ok();
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.preset = Some(receiver),
            Err(err) => cx.toasts.error(format!("Cannot read the preset: {err}")),
        }
    }
}

/// Makes `event` of `repository` the selection of the active profile
/// ([`profiles::use_event_preset`]), with an Undo toast that restores the profile. Used by the
/// Mods and Repository screens.
pub(crate) fn use_event_preset(
    viewer: &ViewerContext,
    toasts: &mut Toaster,
    repository: &str,
    event: &Event,
) {
    let before = viewer.read().client.active_profile().cloned();
    let applied = viewer.mutate(Some(Change::AddonSelection), |state| {
        profiles::use_event_preset(state, repository, event)
    });
    match applied {
        Ok(true) => {
            if let Some(before) = before {
                toasts.undoable(
                    format!("Using the mods of {}", event.name),
                    undo_profile(before, Change::AddonSelection),
                );
            }
        }
        Ok(false) => toasts.info(format!("The mods of {} are already in use", event.name)),
        Err(err) => toasts.error(err.user_message()),
    }
}

/// The enabled addons of the active profile in load order.
fn enabled_order(viewer: &ViewerContext) -> Vec<String> {
    viewer
        .read()
        .client
        .active_profile()
        .map(launch_order)
        .unwrap_or_default()
}

/// Runs `edit` on the active profile and saves it when it reports a change; returns the profile
/// as it was before. A failed save shows a toast.
fn edit_profile(
    viewer: &ViewerContext,
    toasts: &mut Toaster,
    change: Change,
    edit: impl FnOnce(&mut Profile) -> bool,
) -> Option<Profile> {
    let mut before = None;
    let edited = viewer.mutate(Some(change), |state| {
        state.client.update_active_profile(|profile| {
            let previous = profile.clone();
            let changed = edit(profile);
            if changed {
                before = Some(previous);
            }
            changed
        })
    });
    match edited {
        Ok(true) => before,
        Ok(false) => None,
        Err(err) => {
            toasts.error(err.user_message());
            None
        }
    }
}

/// Puts `before` back as the active profile, unless another profile became active.
pub(crate) fn undo_profile(
    before: Profile,
    change: Change,
) -> impl FnOnce(&ViewerContext) + Send + 'static {
    move |viewer| {
        let restored = viewer.mutate(Some(change), |state| {
            state.client.update_active_profile(|profile| {
                if profile.name != before.name {
                    return false;
                }
                *profile = before;
                true
            })
        });
        if let Err(err) = restored {
            tracing::warn!("Undo failed: {}", err.user_message());
        }
    }
}

/// Removes the node at `path`.
fn remove_at(root: &mut a3s_types::local::TreeDirectory, path: &[usize]) -> bool {
    let Some((last, parent)) = path.split_last() else {
        return false;
    };
    if parent.is_empty() {
        return root.remove_child(*last).is_some();
    }
    let mut directory = root;
    for &index in parent {
        match directory.children.get_mut(index) {
            Some(TreeNode::Directory(child)) => directory = child,
            _ => return false,
        }
    }
    let removed = directory.remove_child(*last).is_some();
    directory.selected = edit::all_leaves_selected(directory);
    removed
}

/// While files are dragged over the window, says what dropping a preset does.
fn paint_drop_hint(ui: &Ui) {
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    if !hovering {
        return;
    }
    let palette = theme::palette(ui.ctx());
    let rect = ui.max_rect();
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("a3s_viewer::mods::drop_hint"),
    ));
    painter.rect_filled(rect, 8.0, palette.window_background.gamma_multiply(0.92));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!(
            "{} Drop an Arma 3 Launcher preset to compare it",
            icons::FILE_HTML
        ),
        egui::TextStyle::Heading.resolve(ui.style()),
        palette.text_primary,
    );
}
