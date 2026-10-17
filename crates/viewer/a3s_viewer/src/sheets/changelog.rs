//! The Changelog sheet of a repository: its last builds, newest first, each with its date and
//! the mods it added, updated and removed; the builds after the local revision are marked new.

use a3s_types::Changelog;
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::theme;
use a3s_ui::widgets::button;
use egui::{Label, RichText};

use super::Shown;
use crate::screens::date_text;
use crate::toasts::Toaster;

/// How many builds the sheet lists, like Java's "10 last builds".
const LISTED_BUILDS: usize = 10;

/// The sheet's state.
pub(crate) struct ChangelogSheet {
    repository: String,
    local_revision: i32,

    /// The listed builds, newest first.
    changelogs: Vec<Changelog>,
}

impl ChangelogSheet {
    pub(crate) fn new(
        repository: &str,
        local_revision: i32,
        mut changelogs: Vec<Changelog>,
    ) -> Self {
        changelogs.sort_by_key(|changelog| std::cmp::Reverse(changelog.revision));
        changelogs.truncate(LISTED_BUILDS);
        Self {
            repository: repository.to_owned(),
            local_revision,
            changelogs,
        }
    }

    pub(super) fn show(&self, ctx: &egui::Context, toasts: &mut Toaster) -> Shown {
        let title = format!("Changelog of {}", self.repository);
        let action = Dialog::new("a3s_viewer::sheet::changelog", title)
            .show_ok(false)
            .cancel_label("Close")
            .default_button(Some(DialogButton::Cancel))
            .show(ctx, |ui| {
                let palette = theme::palette(ui.ctx());
                ui.set_width(520.0);
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        for changelog in &self.changelogs {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(heading(changelog)).strong());
                                if changelog.revision > self.local_revision {
                                    ui.label(
                                        RichText::new("not downloaded yet")
                                            .small()
                                            .color(palette.status_info),
                                    );
                                }
                            });
                            for line in lines(changelog) {
                                ui.add(
                                    Label::new(RichText::new(line).color(palette.text_secondary))
                                        .wrap(),
                                );
                            }
                            ui.add_space(8.0);
                        }
                    });
                ui.add_space(4.0);
                if button(ui, "Copy").clicked() {
                    ui.ctx().copy_text(self.text());
                    toasts.info("Changelog copied");
                }
            });
        if action == DialogAction::None {
            Shown::Open
        } else {
            Shown::Closed(None)
        }
    }

    /// The listed builds as plain text.
    fn text(&self) -> String {
        let mut text = String::new();
        for changelog in &self.changelogs {
            text.push_str(&heading(changelog));
            text.push('\n');
            for line in lines(changelog) {
                text.push_str(&line);
                text.push('\n');
            }
            text.push('\n');
        }
        text
    }
}

/// `Revision 3 · Oct 16, 2026`.
fn heading(changelog: &Changelog) -> String {
    let mut heading = format!("Revision {}", changelog.revision);
    if let Some(date) = changelog.build_date.and_then(date_text) {
        heading.push_str(&format!(" · {date}"));
    }
    heading
}

/// One line per non-empty list: `New: @a, @b`.
fn lines(changelog: &Changelog) -> Vec<String> {
    let lists = [
        ("New", &changelog.new_addons),
        ("Updated", &changelog.updated_addons),
        ("Removed", &changelog.deleted_addons),
    ];
    let lines: Vec<String> = lists
        .iter()
        .filter(|(_, addons)| !addons.is_empty())
        .map(|(label, addons)| format!("{label}: {}", addons.join(", ")))
        .collect();
    if lines.is_empty() {
        vec!["No mod changed.".to_owned()]
    } else {
        lines
    }
}
