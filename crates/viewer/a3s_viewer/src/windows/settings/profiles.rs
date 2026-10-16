//! Profiles: the list with the current one marked; add, remove (with Undo), rename
//! (double-click), duplicate, and a desktop shortcut that starts the game with a profile.

use std::path::PathBuf;
use std::sync::mpsc;

use a3s_addons::{ScanOptions, scan};
use a3s_foundation::error::UserFacing as _;
use a3s_launch::{ModPathStyle, RunParameterInput, determine_run_parameters};
use a3s_platform::shortcut::{ShortcutSpec, create_desktop_shortcut, strings as text};
use a3s_repo_files::services::duplicate_profile_name;
use a3s_types::constants::DEFAULT_PROFILE_NAME;
use a3s_types::local::{Configuration, Profile};
use a3s_ui::frames;
use a3s_ui::icon::icons;
use a3s_ui::theme;
use a3s_ui::widgets::icon_text_button;
use a3s_viewer_context::{Change, ViewerContext};
use egui::{Key, RichText, TextEdit, Ui};

use super::{PaneCx, note};
use crate::desktop::Desktop;

/// A rename in progress.
struct Rename {
    profile: String,
    draft: String,
    focused: bool,
}

#[derive(Default)]
pub(super) struct ProfilesPane {
    selected: Option<String>,
    rename: Option<Rename>,
    shortcut: Option<mpsc::Receiver<Result<PathBuf, String>>>,
}

impl ProfilesPane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>) {
        self.poll_shortcut(cx);
        let (names, active) = {
            let state = cx.viewer.read();
            (
                state.client.profile_names(),
                state.client.active_profile_name().to_owned(),
            )
        };
        if self
            .selected
            .as_ref()
            .is_none_or(|selected| !names.contains(selected))
        {
            self.selected = Some(active.clone());
        }

        frames::group(ui, |ui| {
            for name in &names {
                self.row(ui, cx, name, *name == active);
            }
        });
        ui.add_space(8.0);
        let selected = self.selected.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            if icon_text_button(ui, icons::PLUS, "Add").clicked() {
                self.add(cx, &names);
            }
            let removable = selected != DEFAULT_PROFILE_NAME;
            if ui
                .add_enabled_ui(removable, |ui| icon_text_button(ui, icons::MINUS, "Remove"))
                .inner
                .on_disabled_hover_text("The Default profile cannot be removed.")
                .clicked()
            {
                remove(cx, &selected, selected == active);
            }
            if icon_text_button(ui, icons::COPY, "Duplicate").clicked() {
                self.duplicate(cx, &selected, &names);
            }
            let busy = self.shortcut.is_some();
            if ui
                .add_enabled_ui(!busy, |ui| {
                    icon_text_button(ui, icons::DESKTOP, "Desktop shortcut")
                })
                .inner
                .clicked()
            {
                self.create_shortcut(ui.ctx(), cx, &selected);
            }
        });
        ui.add_space(4.0);
        note(ui, "Double-click a profile to rename it.");
    }

    fn row(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>, name: &str, active: bool) {
        let secondary = theme::palette(ui.ctx()).text_secondary;
        ui.horizontal(|ui| {
            let mark = if active { icons::CHECK } else { " " };
            ui.label(RichText::new(mark).color(secondary));
            if let Some(rename) = self.rename.as_mut().filter(|rename| rename.profile == name) {
                let response = ui.add(TextEdit::singleline(&mut rename.draft).desired_width(240.0));
                if !rename.focused {
                    response.request_focus();
                    rename.focused = true;
                }
                if ui.input(|input| input.key_pressed(Key::Escape)) {
                    self.rename = None;
                } else if response.lost_focus() {
                    let draft = rename.draft.trim().to_owned();
                    self.rename = None;
                    if draft != name
                        && cx
                            .save(Some(Change::Profile), |client| {
                                client.rename_profile(name, &draft)
                            })
                            .is_some()
                    {
                        self.selected = Some(draft);
                    }
                }
                return;
            }
            let selected = self.selected.as_deref() == Some(name);
            let response = ui.selectable_label(selected, name);
            if response.clicked() {
                self.selected = Some(name.to_owned());
            }
            if response.double_clicked() && name != DEFAULT_PROFILE_NAME {
                self.rename = Some(Rename {
                    profile: name.to_owned(),
                    draft: name.to_owned(),
                    focused: false,
                });
            }
        });
    }

    /// Creates "New profile" (numbered when taken) and starts renaming it.
    fn add(&mut self, cx: &mut PaneCx<'_>, names: &[String]) {
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    "New profile".to_owned()
                } else {
                    format!("New profile {n}")
                }
            })
            .find(|candidate| !names.contains(candidate))
            .unwrap_or_default();
        if cx
            .save(Some(Change::Profile), |client| client.create_profile(&name))
            .is_some()
        {
            self.selected = Some(name.clone());
            self.rename = Some(Rename {
                profile: name.clone(),
                draft: name,
                focused: false,
            });
        }
    }

    fn duplicate(&mut self, cx: &mut PaneCx<'_>, source: &str, names: &[String]) {
        let base = duplicate_profile_name(source).unwrap_or_else(|| format!("{source} copy"));
        let target = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|candidate| !names.contains(candidate))
            .unwrap_or_default();
        if cx
            .save(Some(Change::Profile), |client| {
                client.duplicate_profile(source, &target)
            })
            .is_some()
        {
            self.selected = Some(target);
        }
    }

    /// Writes a desktop shortcut that starts the game with the profile's parameters, on a
    /// named thread (the addon folders are scanned for the `-mod=` parameters).
    fn create_shortcut(&mut self, ctx: &egui::Context, cx: &mut PaneCx<'_>, name: &str) {
        let (profile, configuration) = {
            let state = cx.viewer.read();
            (
                state.client.profile(name).cloned(),
                state.client.configuration().clone(),
            )
        };
        let Some(profile) = profile else {
            return;
        };
        let desktop = cx.desktop.clone();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("a3s-profile-shortcut".to_owned())
            .spawn(move || {
                sender
                    .send(write_shortcut(&desktop, &profile, &configuration))
                    .ok();
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.shortcut = Some(receiver),
            Err(err) => cx.toasts.error(text::failed(&err.to_string())),
        }
    }

    fn poll_shortcut(&mut self, cx: &mut PaneCx<'_>) {
        let Some(receiver) = &self.shortcut else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(path)) => {
                self.shortcut = None;
                cx.toasts
                    .success(format!("Shortcut created: {}", path.display()));
            }
            Ok(Err(message)) => {
                self.shortcut = None;
                cx.toasts.error(message);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.shortcut = None,
        }
    }
}

/// Removes `name` and offers Undo, which writes the profile back (and makes it current again
/// when it was).
fn remove(cx: &mut PaneCx<'_>, name: &str, was_active: bool) {
    let Some(profile) = cx.viewer.read().client.profile(name).cloned() else {
        return;
    };
    if cx
        .save(Some(Change::Profile), |client| client.remove_profile(name))
        .is_none()
    {
        return;
    }
    cx.toasts
        .undoable(format!("Removed profile {name}"), move |viewer| {
            restore(viewer, profile, was_active);
        });
}

fn restore(viewer: &ViewerContext, profile: Profile, active: bool) {
    let name = profile.name.clone();
    let restored = viewer.mutate(Some(Change::Profile), |state| {
        state.client.create_profile(&name)?;
        state.client.update_profile(&name, |stored| {
            *stored = profile;
            true
        })?;
        if active {
            state.client.set_active_profile(&name)?;
        }
        Ok::<(), a3s_repo_files::services::ServiceError>(())
    });
    if let Err(err) = restored {
        tracing::warn!("Cannot restore profile {name}: {}", err.user_message());
    }
}

fn write_shortcut(
    desktop: &Desktop,
    profile: &Profile,
    configuration: &Configuration,
) -> Result<PathBuf, String> {
    let env = desktop
        .env
        .as_ref()
        .ok_or_else(|| text::NOT_AVAILABLE.to_owned())?;
    let exe = profile
        .launcher_options
        .arma3_exe_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .ok_or_else(|| text::exe_missing(&profile.name))?;
    let registry = scan(profile, &ScanOptions::default()).registry;
    let args = determine_run_parameters(&RunParameterInput {
        profile: Some(profile),
        configuration,
        registry: &registry,
        mod_path_style: ModPathStyle::native(),
    });
    let spec = ShortcutSpec {
        name: format!("Arma 3 - {}", profile.name),
        working_dir: exe.parent().map(PathBuf::from),
        icon: Some(exe.clone()),
        target: exe,
        args,
        comment: Some(format!("Starts Arma 3 with the {} profile", profile.name)),
    };
    create_desktop_shortcut(env, desktop.folders.as_ref(), &spec)
        .map_err(|err| text::failed(&err.user_message()))
}
