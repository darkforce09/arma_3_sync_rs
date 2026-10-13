//! Game: the Arma 3 folder, how the game starts, BattlEye, the in-game name and the launch
//! options of one profile, with the command line they give.

use std::cell::OnceCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use a3s_addons::{AddonRegistry, ScanOptions, scan};
use a3s_foundation::paths::Os;
use a3s_launch::command::{arma3_method, command_line, plan_arma3};
use a3s_launch::executable::{battleye_toggle_path, file_name, is_battleye};
use a3s_launch::{
    LaunchMethod, ModPathStyle, RunParameterInput, SteamCommand, determine_run_parameters,
    find_steam_command,
};
use a3s_platform::detect::ARMA3_X64_EXE;
use a3s_platform::folders::{arma3_documents_dir, game_profile_names};
use a3s_types::constants::MaxMemoryValue;
use a3s_types::local::{Configuration, GameStart, LauncherOptions, Profile};
use a3s_ui::dialog::{Dialog, DialogAction, DialogButton};
use a3s_ui::frames;
use a3s_ui::widgets::{button, form_row, popup_button, toggle};
use a3s_viewer_context::Change;
use egui::{Label, Response, RichText, TextEdit, Ui};

use super::{PaneCx, group_heading, note};
use crate::desktop::Desktop;
use crate::fields::{labelled_row, toggle_row};
use crate::picker::{FilePicker, FolderPicker};

/// A "Customize…" sheet of the Game pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Customize {
    Performance,
    Developer,
}

/// What an addon scan was made from; a change scans again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ScanKey {
    profile: String,
    search_directories: Vec<String>,
    arma3_exe_path: Option<String>,
}

/// The addons of the shown profile, scanned on a named thread for the command-line preview.
#[derive(Default)]
struct Addons {
    key: Option<ScanKey>,
    registry: AddonRegistry,
    pending: Option<mpsc::Receiver<AddonRegistry>>,
}

impl Addons {
    /// The registry of `profile`, scanning again when its folders changed; empty until the
    /// first scan finished.
    fn registry(&mut self, ctx: &egui::Context, profile: &Profile) -> &AddonRegistry {
        if let Some(receiver) = &self.pending
            && let Ok(registry) = receiver.try_recv()
        {
            self.registry = registry;
            self.pending = None;
        }
        let key = ScanKey {
            profile: profile.name.clone(),
            search_directories: profile.addon_search_directories.clone(),
            arma3_exe_path: profile.launcher_options.arma3_exe_path.clone(),
        };
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            let (sender, receiver) = mpsc::channel();
            let profile = profile.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("a3s-addon-scan".to_owned())
                .spawn(move || {
                    sender
                        .send(scan(&profile, &ScanOptions::default()).registry)
                        .ok();
                    ctx.request_repaint();
                });
            match spawned {
                Ok(_) => self.pending = Some(receiver),
                Err(err) => tracing::warn!("Cannot scan the addons: {err}"),
            }
        }
        &self.registry
    }
}

#[derive(Default)]
pub(super) struct GamePane {
    /// The profile shown; the active one until another is chosen.
    profile: Option<String>,
    folder_picker: FolderPicker,
    mission_picker: FilePicker,
    customize: Option<Customize>,

    /// The extra parameters while their field has the focus.
    extra_parameters: Option<String>,

    /// The Arma 3 profile names, read once.
    game_profiles: Option<Vec<String>>,

    /// The Steam client, looked up once.
    steam: OnceCell<Option<SteamCommand>>,
    addons: Addons,
}

impl GamePane {
    pub(super) fn show(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>) {
        let (names, active, configuration) = {
            let state = cx.viewer.read();
            (
                state.client.profile_names(),
                state.client.active_profile_name().to_owned(),
                state.client.configuration().clone(),
            )
        };
        let mut name = self
            .profile
            .clone()
            .filter(|name| names.contains(name))
            .unwrap_or(active);
        let Some(mut profile) = cx.viewer.read().client.profile(&name).cloned() else {
            note(ui, "There is no profile to edit.");
            return;
        };
        let before = profile.clone();

        labelled_row(ui, "Profile", |ui| {
            let options: Vec<(String, &str)> = names
                .iter()
                .map(|name| (name.clone(), name.as_str()))
                .collect();
            popup_button(
                ui,
                "a3s_viewer::settings::game_profile",
                &mut name,
                &options,
            )
        });
        if name != profile.name {
            self.profile = Some(name);
            self.extra_parameters = None;
            return;
        }
        self.profile = Some(name.clone());

        if let Some(dir) = self.folder_picker.poll() {
            profile.launcher_options.arma3_exe_path =
                Some(dir.join(ARMA3_X64_EXE).display().to_string());
        }
        if let Some(file) = self.mission_picker.poll() {
            profile.launcher_options.mission_file_path = Some(file.display().to_string());
            profile.launcher_options.mission_file = true;
        }

        group_heading(ui, "Arma 3");
        frames::group(ui, |ui| self.game_rows(ui, cx, &mut profile));
        group_heading(ui, "Options");
        frames::group(ui, |ui| {
            option_rows(ui, &mut profile.launcher_options);
            frames::group_separator(ui);
            form_row(ui, "Performance", |ui| {
                if button(ui, "Customize…").clicked() {
                    self.customize = Some(Customize::Performance);
                }
            });
            frames::group_separator(ui);
            form_row(ui, "Developer", |ui| {
                if button(ui, "Customize…").clicked() {
                    self.customize = Some(Customize::Developer);
                }
            });
        });

        let closed = match self.customize {
            Some(Customize::Performance) => sheet(ui.ctx(), "Performance", |ui| {
                performance_rows(ui, &mut profile.launcher_options);
            }),
            Some(Customize::Developer) => sheet(ui.ctx(), "Developer", |ui| {
                self.developer_rows(ui, &mut profile);
            }),
            None => false,
        };
        if closed {
            self.customize = None;
            if let Some(text) = self.extra_parameters.take() {
                profile.additional_parameters = (!text.trim().is_empty()).then_some(text);
            }
        }

        group_heading(ui, "Command line");
        let steam = self
            .steam
            .get_or_init(|| cx.desktop.env.as_ref().and_then(find_steam_command))
            .clone();
        let registry = self.addons.registry(ui.ctx(), &profile);
        let preview = command_preview(&profile, &configuration, registry, steam);
        frames::group(ui, |ui| {
            if preview.is_empty() {
                note(ui, "No parameters.");
            } else {
                ui.add(
                    Label::new(RichText::new(preview).monospace())
                        .wrap()
                        .selectable(true),
                );
            }
        });

        if profile != before {
            cx.save(Some(Change::Profile), |client| {
                client.update_profile(&name, |stored| {
                    *stored = profile;
                    true
                })
            });
        }
    }

    fn game_rows(&mut self, ui: &mut Ui, cx: &mut PaneCx<'_>, profile: &mut Profile) {
        let options = &mut profile.launcher_options;
        let exe = options
            .arma3_exe_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        let folder = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        form_row(ui, "Arma 3 folder", |ui| {
            if button(ui, "Choose…").clicked() {
                self.folder_picker
                    .open(ui.ctx(), "Arma 3 folder", folder.as_deref());
            }
            let text = folder.map_or_else(|| "Not set".to_owned(), |dir| dir.display().to_string());
            ui.add(Label::new(text).truncate());
        });
        if Os::current() == Os::Linux {
            frames::group_separator(ui);
            labelled_row(ui, "Start through", |ui| {
                popup_button(
                    ui,
                    "a3s_viewer::settings::game_start",
                    &mut options.game_start,
                    &[
                        (GameStart::Automatic, "Automatic"),
                        (GameStart::Steam, "Steam"),
                        (GameStart::Executable, "Executable"),
                    ],
                )
            });
        }
        frames::group_separator(ui);
        let mut on = exe
            .as_deref()
            .is_some_and(|exe| is_battleye(&file_name(exe)));
        let battleye = labelled_row(ui, "BattlEye", |ui| {
            ui.add_enabled_ui(exe.is_some(), |ui| toggle(ui, &mut on))
                .inner
        });
        if battleye.changed()
            && let Some(exe) = &exe
        {
            match battleye_executable(exe) {
                Some(target) => options.arma3_exe_path = Some(target.display().to_string()),
                None => cx
                    .toasts
                    .error("The BattlEye launcher is not in the Arma 3 folder."),
            }
        }
        frames::group_separator(ui);
        let game_profiles = self
            .game_profiles
            .get_or_insert_with(|| game_profiles(cx.desktop));
        labelled_row(ui, "In-game name", |ui| {
            let current = options.game_profile.clone();
            let mut choices: Vec<(Option<String>, &str)> = vec![(None, "Default")];
            choices.extend(
                game_profiles
                    .iter()
                    .map(|name| (Some(name.clone()), name.as_str())),
            );
            if let Some(current) = &current
                && !game_profiles.contains(current)
            {
                choices.push((Some(current.clone()), current.as_str()));
            }
            popup_button(
                ui,
                "a3s_viewer::settings::game_profile_name",
                &mut options.game_profile,
                &choices,
            )
        });
    }

    fn developer_rows(&mut self, ui: &mut Ui, profile: &mut Profile) {
        let options = &mut profile.launcher_options;
        frames::group(ui, |ui| {
            toggle_row(ui, "File patching", &mut options.file_patching);
            frames::group_separator(ui);
            toggle_row(ui, "No logs", &mut options.nologs);
            frames::group_separator(ui);
            toggle_row(ui, "Check signatures", &mut options.check_signatures);
            if Os::current() == Os::Windows {
                frames::group_separator(ui);
                toggle_row(
                    ui,
                    "Run as administrator",
                    &mut options.launch_arma3_as_administrator,
                );
            }
            frames::group_separator(ui);
            toggle_row(ui, "Mission file", &mut options.mission_file);
            frames::group_separator(ui);
            form_row(ui, "Mission", |ui| {
                if button(ui, "Choose…").clicked() {
                    let start = options
                        .mission_file_path
                        .as_deref()
                        .map(Path::new)
                        .and_then(Path::parent);
                    self.mission_picker.open(ui.ctx(), "Mission file", start);
                }
                let text = options.mission_file_path.as_deref().unwrap_or("None");
                ui.add(Label::new(text).truncate());
            });
        });
        ui.add_space(8.0);
        ui.label("Extra parameters");
        let stored = profile.additional_parameters.clone().unwrap_or_default();
        let mut text = self.extra_parameters.clone().unwrap_or(stored);
        let response = ui.add(
            TextEdit::singleline(&mut text)
                .hint_text("-par=params.txt")
                .desired_width(f32::INFINITY),
        );
        // Written when the field loses the focus, not on every key.
        if response.has_focus() {
            self.extra_parameters = Some(text);
        } else if self.extra_parameters.take().is_some() {
            profile.additional_parameters = (!text.trim().is_empty()).then_some(text);
        }
    }
}

/// Shows a "Customize…" sheet; whether it was closed.
fn sheet(ctx: &egui::Context, title: &str, add_contents: impl FnOnce(&mut Ui)) -> bool {
    let action = Dialog::new(("a3s_viewer::settings::customize", title), title)
        .show_ok(false)
        .cancel_label("Done")
        .default_button(Some(DialogButton::Cancel))
        .show(ctx, |ui| {
            ui.set_width(380.0);
            add_contents(ui);
        });
    action != DialogAction::None
}

fn option_rows(ui: &mut Ui, options: &mut LauncherOptions) {
    let rows: [(&str, &mut bool); 5] = [
        ("Skip the intro", &mut options.no_splash_screen),
        ("Empty world", &mut options.default_world),
        ("Windowed", &mut options.window_mode),
        ("Keep running in the background", &mut options.no_pause),
        ("Show script errors", &mut options.show_script_errors),
    ];
    for (index, (label, on)) in rows.into_iter().enumerate() {
        if index > 0 {
            frames::group_separator(ui);
        }
        toggle_row(ui, label, on);
    }
}

fn performance_rows(ui: &mut Ui, options: &mut LauncherOptions) {
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let cpu_labels: Vec<String> = (1..=cpus).map(|n| n.to_string()).collect();
    let mut cpu_choices: Vec<(i32, &str)> = vec![(0, "Automatic")];
    cpu_choices.extend(
        cpu_labels
            .iter()
            .enumerate()
            .map(|(index, label)| (i32::try_from(index + 1).unwrap_or(i32::MAX), label.as_str())),
    );
    let memory: Vec<String> = MaxMemoryValue::ALL
        .iter()
        .map(|value| value.megabytes().to_string())
        .collect();
    frames::group(ui, |ui| {
        labelled_row(ui, "Processor cores", |ui| {
            popup_button(
                ui,
                "a3s_viewer::settings::cpu_count",
                &mut options.cpu_count_selection,
                &cpu_choices,
            )
        });
        frames::group_separator(ui);
        labelled_row(ui, "Extra threads", |ui| {
            text_choice(
                ui,
                "a3s_viewer::settings::ex_threads",
                &mut options.ex_threads_selection,
                &["0", "1", "3", "5", "7"],
            )
        });
        frames::group_separator(ui);
        labelled_row(ui, "Maximum memory (MB)", |ui| {
            let memory: Vec<&str> = memory.iter().map(String::as_str).collect();
            text_choice(
                ui,
                "a3s_viewer::settings::max_memory",
                &mut options.max_memory_selection,
                &memory,
            )
        });
        frames::group_separator(ui);
        labelled_row(ui, "Memory allocator", |ui| {
            text_choice(
                ui,
                "a3s_viewer::settings::malloc",
                &mut options.malloc_selection,
                &["system"],
            )
        });
        frames::group_separator(ui);
        toggle_row(ui, "Huge pages", &mut options.huge_pages);
        frames::group_separator(ui);
        toggle_row(ui, "Hyper-threading", &mut options.enable_ht);
    });
}

/// A pop-up of "Default" (no value) and `values`; a stored value outside them stays listed.
fn text_choice(ui: &mut Ui, id: &str, value: &mut Option<String>, values: &[&str]) -> Response {
    let mut choices: Vec<(Option<String>, &str)> = vec![(None, "Default")];
    choices.extend(values.iter().map(|v| (Some((*v).to_owned()), *v)));
    let current = value.clone();
    if let Some(current) = &current
        && !values.contains(&current.as_str())
    {
        choices.push((Some(current.clone()), current.as_str()));
    }
    popup_button(ui, id, value, &choices)
}

/// The executable the BattlEye toggle switches to, when it exists.
fn battleye_executable(exe: &Path) -> Option<PathBuf> {
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)").is_some();
    battleye_toggle_path(exe, Os::current(), program_files_x86).filter(|target| target.is_file())
}

fn game_profiles(desktop: &Desktop) -> Vec<String> {
    desktop
        .env
        .as_ref()
        .and_then(|env| {
            arma3_documents_dir(env, desktop.folders.as_ref(), desktop.registry.as_ref())
        })
        .map(|dir| game_profile_names(&dir))
        .unwrap_or_default()
}

/// The command the game would start with, for display: the planned command when the
/// executable is set, the parameters alone otherwise.
pub(super) fn command_preview(
    profile: &Profile,
    configuration: &Configuration,
    registry: &AddonRegistry,
    steam: Option<SteamCommand>,
) -> String {
    let os = Os::current();
    let options = &profile.launcher_options;
    let exe = options
        .arma3_exe_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(Path::new);
    let method = exe.and_then(|exe| arma3_method(os, exe, options, options.game_start.into()).ok());
    let mod_path_style = if method == Some(LaunchMethod::Steam) {
        ModPathStyle::WineZDrive
    } else {
        ModPathStyle::native()
    };
    let params = determine_run_parameters(&RunParameterInput {
        profile: Some(profile),
        configuration,
        registry,
        mod_path_style,
    });
    // Shown with a plain `steam` when no Steam client was found.
    let steam = steam.unwrap_or_else(|| SteamCommand {
        prefix: vec![OsString::from("steam")],
    });
    match (exe, method) {
        (Some(exe), Some(method)) => plan_arma3(os, exe, &params, method, Some(&steam))
            .map_or_else(
                |_| command_line(&params),
                |plan| command_line(&plan.shown_command),
            ),
        _ => command_line(&params),
    }
}
