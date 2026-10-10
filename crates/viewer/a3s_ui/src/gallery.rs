//! A gallery of every token, font face, icon, widget, frame and dialog of this crate.
//!
//! Used by `examples/demo.rs` and by the snapshot tests.

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::consts::{JAVA_BLUE, JAVA_RED};
use crate::dialog::{Dialog, DialogAction, DialogButton, ProgressDialog, ProgressDialogAction};
use crate::fonts::{self, FontFace};
use crate::frames;
use crate::icon::{self, IconState, UiIcon};
use crate::theme::{self, ProgressTone, ThemeMode};
use crate::widgets::progress::DIALOG_HEIGHT;
use crate::widgets::{
    CheckItem, CheckList, CheckState, CheckTree, CheckTreeEvent, CheckTreeModel, Progress,
    ProgressBar, button, checked_labels, icon_text_button, primary_button,
};

/// Section headings, in display order.
pub const SECTIONS: [&str; 9] = [
    "Theme",
    "Fonts",
    "Icons",
    "Buttons",
    "Progress",
    "Check tree",
    "Check list",
    "Frames",
    "Dialogs",
];

/// Java size of the gallery icons (18 → 27 pt).
const GALLERY_ICON_SIZE: u32 = 18;

/// Icons per row in the icon grid.
const ICONS_PER_ROW: usize = 6;

struct SampleNode {
    name: &'static str,
    parent: Option<usize>,
    children: Vec<usize>,
    selected: bool,
    color: Option<Color32>,
}

/// The fixed sample tree of the gallery: "@mods" with "@ace" (all selected), "@cba" (mixed),
/// "@tfar" (none selected), a red (missing) leaf and a blue (deleted) leaf.
pub struct SampleTree {
    nodes: Vec<SampleNode>,
}

impl Default for SampleTree {
    fn default() -> Self {
        let mut tree = Self { nodes: Vec::new() };
        let root = tree.add("@mods", None, false, None);
        let ace = tree.add("@ace", Some(root), true, None);
        tree.add("ace_common.pbo", Some(ace), true, None);
        tree.add("ace_medical.pbo", Some(ace), true, None);
        let cba = tree.add("@cba", Some(root), true, None);
        tree.add("cba_main.pbo", Some(cba), true, None);
        tree.add("cba_xeh.pbo", Some(cba), false, None);
        let tfar = tree.add("@tfar", Some(root), false, None);
        tree.add("tfar_core.pbo", Some(tfar), false, None);
        tree.add("tfar_handhelds.pbo", Some(tfar), false, None);
        tree.add("@missing", Some(root), true, Some(JAVA_RED));
        tree.add("@deleted", Some(root), false, Some(JAVA_BLUE));
        tree
    }
}

impl SampleTree {
    fn add(
        &mut self,
        name: &'static str,
        parent: Option<usize>,
        selected: bool,
        color: Option<Color32>,
    ) -> usize {
        let index = self.nodes.len();
        self.nodes.push(SampleNode {
            name,
            parent,
            children: Vec::new(),
            selected,
            color,
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        }
        index
    }

    fn index(&self, key: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.name == key)
    }

    /// Every key, depth first.
    pub fn keys(&self) -> Vec<String> {
        self.nodes.iter().map(|node| node.name.to_owned()).collect()
    }

    /// Whether `key` is selected.
    pub fn is_selected(&self, key: &str) -> bool {
        self.index(key).is_some_and(|i| self.nodes[i].selected)
    }

    /// Flips `key` and sets all its descendants to the new value (demo-only stand-in for
    /// `selectAllDescending` / `deselectAllDescending`).
    pub fn toggle(&mut self, key: &str) {
        let Some(index) = self.index(key) else {
            return;
        };
        let value = !self.nodes[index].selected;
        let mut stack = vec![index];
        while let Some(i) = stack.pop() {
            self.nodes[i].selected = value;
            stack.extend(self.nodes[i].children.iter().copied());
        }
    }
}

impl CheckTreeModel for SampleTree {
    type Key = String;

    fn roots(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|node| node.parent.is_none())
            .map(|node| node.name.to_owned())
            .collect()
    }

    fn children(&self, key: &String) -> Vec<String> {
        self.index(key)
            .map(|i| {
                self.nodes[i]
                    .children
                    .iter()
                    .map(|&c| self.nodes[c].name.to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_leaf(&self, key: &String) -> bool {
        self.index(key)
            .is_none_or(|i| self.nodes[i].children.is_empty())
    }

    fn label(&self, key: &String) -> String {
        key.clone()
    }

    fn check_state(&self, key: &String) -> CheckState {
        let Some(i) = self.index(key) else {
            return CheckState::Unchecked;
        };
        let node = &self.nodes[i];
        CheckState::for_node(
            node.selected,
            self.is_leaf(key),
            node.parent.is_some(),
            node.children.iter().map(|&c| self.nodes[c].selected),
        )
    }

    fn label_color(&self, key: &String) -> Option<Color32> {
        self.index(key).and_then(|i| self.nodes[i].color)
    }
}

/// The gallery state.
pub struct Gallery {
    /// Freeze the indeterminate progress animation at t = 0 (deterministic snapshots).
    pub freeze_animations: bool,
    sample: SampleTree,
    tree: CheckTree,
    last_tree_event: String,
    list: CheckList,
    items: Vec<CheckItem>,
    modal_open: bool,
    window_open: bool,
    progress_dialog: Option<ProgressDialog>,
    last_action: String,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            freeze_animations: false,
            sample: SampleTree::default(),
            tree: CheckTree::new("a3s_ui_gallery_tree").default_expanded(true),
            last_tree_event: String::new(),
            list: CheckList::new("a3s_ui_gallery_list"),
            items: vec![
                CheckItem::new("Profile A", false),
                CheckItem::new("Profile B", true),
                CheckItem::new("Profile C", false),
            ],
            modal_open: false,
            window_open: false,
            progress_dialog: None,
            last_action: "none".to_owned(),
        }
    }
}

impl Gallery {
    /// Shows every section.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.theme_ui(ui);
        self.fonts_ui(ui);
        self.icons_ui(ui);
        self.buttons_ui(ui);
        self.progress_ui(ui);
        self.check_tree_ui(ui);
        self.check_list_ui(ui);
        self.frames_ui(ui);
        self.dialogs_ui(ui);
    }

    fn heading(ui: &mut Ui, text: &str) {
        ui.add_space(8.0);
        ui.heading(text);
        ui.add_space(4.0);
    }

    /// Theme toggle, the 21 palette swatches and the metrics.
    pub fn theme_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[0]);
        let def = theme::current(ui.ctx());
        // Java shows the moon in light mode (switch to dark) and the sun in dark mode.
        let icon = match def.mode() {
            ThemeMode::Light => UiIcon::Moon,
            ThemeMode::Dark => UiIcon::Sun,
        };
        if icon_text_button(ui, icon, 16, "Toggle theme").clicked() {
            theme::toggle(ui.ctx());
        }
        ui.label(format!("Current theme: {}", def.id));

        egui::Grid::new("a3s_ui_gallery_swatches")
            .striped(false)
            .num_columns(6)
            .spacing(vec2(12.0, 4.0))
            .show(ui, |ui| {
                for (i, (name, color)) in def.palette.named_tokens().into_iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(2),
                            color,
                            Stroke::new(1.0, def.palette.border),
                            StrokeKind::Inside,
                        );
                        ui.label(name);
                    });
                    if i % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
        let m = &def.metrics;
        ui.label(format!(
            "Spacing xs {} / sm {} / md {} / lg {} / xl {} / xxl {}   Radius sm {} / md {} / lg {}   Focus {}",
            m.spacing_xs,
            m.spacing_sm,
            m.spacing_md,
            m.spacing_lg,
            m.spacing_xl,
            m.spacing_xxl,
            m.radius_sm,
            m.radius_md,
            m.radius_lg,
            m.focus_width
        ));
    }

    /// One line per Geist face at body size.
    pub fn fonts_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[1]);
        for face in FontFace::ALL {
            ui.label(
                RichText::new(format!(
                    "{}: The quick brown fox jumps over the lazy dog 0123456789",
                    face.name()
                ))
                .font(fonts::font_id_in(ui.ctx(), face, fonts::BODY_SIZE)),
            );
        }
    }

    /// All 42 icons in Normal, Disabled and Selected state, labelled with the file stem.
    pub fn icons_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[2]);
        let ctx = ui.ctx().clone();
        let selection_bg = ui.visuals().selection.bg_fill;
        egui::Grid::new("a3s_ui_gallery_icons")
            .striped(false)
            .num_columns(ICONS_PER_ROW)
            .spacing(vec2(16.0, 6.0))
            .show(ui, |ui| {
                for (i, icon) in UiIcon::ALL.into_iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for state in
                                [IconState::Normal, IconState::Disabled, IconState::Selected]
                            {
                                let image = icon::icon_image(&ctx, icon, GALLERY_ICON_SIZE, state);
                                let image = if state == IconState::Selected {
                                    image.bg_fill(selection_bg)
                                } else {
                                    image
                                };
                                ui.add(image.alt_text(format!("{} {state:?}", icon.file_name())));
                            }
                        });
                        ui.label(icon.file_name());
                    });
                    if i % ICONS_PER_ROW == ICONS_PER_ROW - 1 {
                        ui.end_row();
                    }
                }
            });
    }

    /// Normal, primary, disabled and icon + text buttons.
    pub fn buttons_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[3]);
        ui.horizontal(|ui| {
            button(ui, "Normal");
            primary_button(ui, "Primary");
            ui.add_enabled_ui(false, |ui| button(ui, "Disabled"));
            icon_text_button(ui, UiIcon::Refresh, 16, "Refresh");
            ui.add_enabled_ui(false, |ui| {
                icon_text_button(ui, UiIcon::Download, 16, "Download")
            });
        });
    }

    /// Four tones at 0 / 33 / 67 / 100 %, an indeterminate bar and an 18 px bar.
    pub fn progress_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[4]);
        egui::Grid::new("a3s_ui_gallery_progress")
            .striped(false)
            .num_columns(5)
            .spacing(vec2(12.0, 8.0))
            .show(ui, |ui| {
                for tone in ProgressTone::ALL {
                    ui.label(format!("{tone:?}"));
                    // 0 %, 33 %, 67 %, 100 %.
                    for (value, max) in [(0, 3), (1, 3), (2, 3), (3, 3)] {
                        ui.add(
                            ProgressBar::new(Progress::of(value, max))
                                .tone(tone)
                                .desired_width(160.0),
                        );
                    }
                    ui.end_row();
                }
            });
        let mut indeterminate = ProgressBar::new(Progress::Indeterminate {
            text: Some("Connecting...".to_owned()),
        })
        .desired_width(400.0);
        if self.freeze_animations {
            indeterminate = indeterminate.animation_time(0.0);
        }
        ui.add(indeterminate);
        ui.add(
            ProgressBar::new(Progress::of(50, 100))
                .height(DIALOG_HEIGHT)
                .desired_width(250.0),
        );
    }

    /// The sample check tree; toggling flips a node and its descendants.
    pub fn check_tree_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[5]);
        let events = self.tree.show(ui, &self.sample);
        for event in events {
            if let CheckTreeEvent::Toggled(key) = &event {
                self.sample.toggle(key);
            }
            self.last_tree_event = format!("{event:?}");
        }
        if !self.last_tree_event.is_empty() {
            ui.label(format!("Last tree event: {}", self.last_tree_event));
        }
    }

    /// The sample check list.
    pub fn check_list_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[6]);
        ui.allocate_ui(vec2(240.0, 0.0), |ui| {
            self.list.show(ui, &mut self.items);
        });
        ui.label(format!(
            "Checked: {}",
            checked_labels(&self.items).join(", ")
        ));
    }

    /// Section, panel, toolbar, status bar and pane divider.
    pub fn frames_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[7]);
        frames::section(ui, "Section", |ui| {
            ui.label("Content of a titled section.");
        });
        ui.add_space(8.0);
        frames::panel(ui, |ui| {
            ui.label("Content of a bordered panel.");
        });
        ui.add_space(8.0);
        frames::toolbar(ui, |ui| {
            ui.horizontal(|ui| {
                icon_text_button(ui, UiIcon::Add, 16, "Add");
                icon_text_button(ui, UiIcon::Edit, 16, "Edit");
                icon_text_button(ui, UiIcon::Delete, 16, "Delete");
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            frames::pane_divider(ui, |ui| {
                ui.set_min_size(vec2(160.0, 40.0));
                ui.label("Left pane");
            });
            ui.label("Right pane");
        });
        ui.add_space(8.0);
        frames::status_bar(ui, |ui| {
            ui.label("Status bar");
        });
    }

    /// Buttons opening a modal dialog, a non-modal dialog and a progress dialog.
    pub fn dialogs_ui(&mut self, ui: &mut Ui) {
        Self::heading(ui, SECTIONS[8]);
        ui.horizontal(|ui| {
            if button(ui, "Open modal dialog").clicked() {
                self.modal_open = true;
            }
            if button(ui, "Open dialog").clicked() {
                self.window_open = true;
            }
            if button(ui, "Open progress dialog").clicked() {
                let mut dialog =
                    ProgressDialog::new("a3s_ui_gallery_progress", "Downloading files...");
                dialog.progress = Progress::of(50, 100);
                self.progress_dialog = Some(dialog);
            }
        });
        ui.label(format!("Last action: {}", self.last_action));

        let ctx = ui.ctx().clone();
        if self.modal_open {
            let action = Dialog::new("a3s_ui_gallery_modal", "Modal dialog")
                .default_button(Some(DialogButton::Ok))
                .show(&ctx, |ui| {
                    ui.label("A modal dialog blocks the rest of the UI.");
                });
            self.record(action, |gallery| gallery.modal_open = false);
        }
        if self.window_open {
            let action = Dialog::new("a3s_ui_gallery_window", "Dialog")
                .modal(false)
                .show(&ctx, |ui| {
                    ui.label("A non-modal dialog.");
                });
            self.record(action, |gallery| gallery.window_open = false);
        }
        if let Some(dialog) = &mut self.progress_dialog
            && dialog.show(&ctx) == ProgressDialogAction::Cancel
        {
            self.progress_dialog = None;
            self.last_action = "Cancel (progress)".to_owned();
        }
    }

    fn record(&mut self, action: DialogAction, close: impl FnOnce(&mut Self)) {
        if action != DialogAction::None {
            self.last_action = format!("{action:?}");
            close(self);
        }
    }
}
