//! The Events segment: the repository's events (list on the left: add, duplicate, remove with
//! Undo) and the selected one (name, description, and which mods are required or optional).
//! Every edit is saved to the source folder at once.

#![allow(
    clippy::result_large_err,
    reason = "the events errors carry transport errors; they are returned once per edit"
)]

use std::collections::BTreeMap;

use a3s_admin::EventSelection;
use a3s_admin::events::{EventNameError, validate_event_name};
use a3s_foundation::error::UserFacing as _;
use a3s_types::Event;
use a3s_ui::frames;
use a3s_ui::theme;
use a3s_ui::widgets::{button, segmented};
use a3s_viewer_context::admin::{edit_events, restore_events};
use a3s_viewer_context::{Change, ViewerContext};
use egui::{Align, Layout, RichText, TextEdit, Ui, vec2};

use super::View;
use crate::screens::{ScreenCx, section_title};
use crate::sheets::add_repository::text_row;

/// Width of the event list.
const LIST_WIDTH: f32 = 220.0;

/// The selected event and the text fields being typed into.
#[derive(Default)]
pub(super) struct EventsState {
    selected: Option<String>,

    /// The event the fields below were filled from.
    fields_of: Option<String>,
    name: String,
    description: String,
    name_error: Option<EventNameError>,
}

pub(super) fn show(ui: &mut Ui, view: &View, state: &mut EventsState, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    let Some(doc) = &view.publishing.events else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Reading the events…");
        });
        return;
    };
    let events = doc.events();
    if !state
        .selected
        .as_ref()
        .is_some_and(|selected| events.iter().any(|event| event.name == *selected))
    {
        state.selected = events.first().map(|event| event.name.clone());
    }

    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            vec2(LIST_WIDTH, ui.available_height()),
            Layout::top_down(Align::Min),
            |ui| list(ui, view, &events, state, cx),
        );
        ui.add_space(16.0);
        ui.vertical(|ui| {
            match events
                .iter()
                .find(|event| Some(&event.name) == state.selected.as_ref())
            {
                Some(event) => detail(ui, view, event, state, cx),
                None => {
                    ui.label(
                        RichText::new("Add an event to give players a preset of mods.")
                            .color(palette.text_secondary),
                    );
                }
            }
        });
    });
    if view.publishing.events_changed {
        ui.add_space(12.0);
        ui.label(
            RichText::new("Saved in the source folder; uploaded with the next publish.")
                .small()
                .color(palette.text_secondary),
        );
    }
}

fn list(
    ui: &mut Ui,
    view: &View,
    events: &[Event],
    state: &mut EventsState,
    cx: &mut ScreenCx<'_>,
) {
    let palette = theme::palette(ui.ctx());
    frames::group(ui, |ui| {
        ui.set_min_width(ui.available_width());
        if events.is_empty() {
            ui.label(RichText::new("No events yet.").color(palette.text_secondary));
        }
        for event in events {
            let selected = state.selected.as_ref() == Some(&event.name);
            if ui
                .add_sized(
                    vec2(ui.available_width(), 24.0),
                    egui::Button::selectable(selected, &event.name),
                )
                .clicked()
            {
                state.selected = Some(event.name.clone());
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if button(ui, "Add").on_hover_text("Add an event").clicked() {
            let name = unused_name(events);
            let added = edit(cx, view.name(), |doc| {
                doc.add_event(&name, "", &EventSelection::default());
            });
            if added.is_some() {
                state.selected = Some(name);
            }
        }
        let Some(selected) = state.selected.clone() else {
            return;
        };
        if button(ui, "Duplicate").clicked()
            && edit(cx, view.name(), |doc| doc.duplicate_event(&selected)) == Some(true)
        {
            state.selected = Some(format!("{selected} - duplicate"));
        }
        if button(ui, "Remove").clicked() {
            remove(view, &selected, state, cx);
        }
    });
}

/// "New event", or "New event 2", 3… when taken.
fn unused_name(events: &[Event]) -> String {
    let taken = |name: &str| events.iter().any(|event| event.name == name);
    let mut name = "New event".to_owned();
    let mut number = 2;
    while taken(&name) {
        name = format!("New event {number}");
        number += 1;
    }
    name
}

/// Removes the event, with an Undo toast that puts the events back as they were.
fn remove(view: &View, name: &str, state: &mut EventsState, cx: &mut ScreenCx<'_>) {
    let before = view.publishing.events.clone().unwrap_or_default();
    if edit(cx, view.name(), |doc| doc.remove_event(name)) != Some(true) {
        return;
    }
    state.selected = None;
    let repository = view.name().to_owned();
    cx.toasts
        .undoable(format!("Removed {name}"), move |viewer: &ViewerContext| {
            let restored = viewer.mutate(Some(Change::Repository), |state| {
                restore_events(state, &repository, before)
            });
            if let Err(err) = restored {
                tracing::warn!("Cannot undo the event removal: {}", err.user_message());
            }
        });
}

fn detail(ui: &mut Ui, view: &View, event: &Event, state: &mut EventsState, cx: &mut ScreenCx<'_>) {
    let palette = theme::palette(ui.ctx());
    if state.fields_of.as_ref() != Some(&event.name) {
        state.fields_of = Some(event.name.clone());
        state.name.clone_from(&event.name);
        state.description = event.description.clone().unwrap_or_default();
        state.name_error = None;
    }
    frames::group(ui, |ui| {
        if text_row(ui, "Name", &mut state.name, "", false) {
            rename(view, event, state, cx);
        }
        frames::group_separator(ui);
        let label = ui.label("Description");
        let changed = ui
            .add(
                TextEdit::multiline(&mut state.description)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id)
            .changed();
        if changed {
            let description = state.description.clone();
            edit(cx, view.name(), |doc| {
                doc.rename_event(&event.name, &event.name, &description);
            });
        }
    });
    if let Some(error) = state.name_error {
        let text = match error {
            EventNameError::Empty => "Enter a name.",
            EventNameError::Duplicate => "Another event has this name.",
        };
        ui.label(RichText::new(text).color(palette.status_danger));
    }

    section_title(ui, "Mods");
    let mods = mod_names(view, event);
    let mut addons = event.addon_names.clone();
    let mut changed = false;
    frames::group(ui, |ui| {
        if mods.is_empty() {
            ui.label(
                RichText::new("The source folder has no mods yet.").color(palette.text_secondary),
            );
        }
        for (index, name) in mods.iter().enumerate() {
            if index > 0 {
                frames::group_separator(ui);
            }
            ui.horizontal(|ui| {
                let mut included = addons.contains_key(name);
                if ui.checkbox(&mut included, name.as_str()).changed() {
                    changed = true;
                    if included {
                        addons.insert(name.clone(), false);
                    } else {
                        addons.remove(name);
                    }
                }
                if let Some(optional) = addons.get_mut(name) {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut segment = usize::from(*optional);
                        if segmented(ui, &mut segment, &["Required", "Optional"]).changed() {
                            *optional = segment == 1;
                            changed = true;
                        }
                    });
                }
            });
        }
    });
    if changed {
        let selection = selection(&mods, &addons, event);
        edit(cx, view.name(), |doc| {
            doc.save_event(&event.name, &selection)
        });
    }
}

/// Renames the event once the typed name is valid.
fn rename(view: &View, event: &Event, state: &mut EventsState, cx: &mut ScreenCx<'_>) {
    let Some(doc) = &view.publishing.events else {
        return;
    };
    match validate_event_name(doc, &state.name, Some(&event.name)) {
        Ok(name) => {
            state.name_error = None;
            if name == event.name {
                return;
            }
            let description = state.description.clone();
            if edit(cx, view.name(), |doc| {
                doc.rename_event(&event.name, &name, &description)
            })
            .is_some()
            {
                state.selected = Some(name.clone());
                state.fields_of = Some(name);
            }
        }
        Err(error) => state.name_error = Some(error),
    }
}

/// The repository's mods, then the event's mods the source folder no longer has.
fn mod_names(view: &View, event: &Event) -> Vec<String> {
    let mut mods: Vec<String> = view
        .publishing
        .local
        .as_ref()
        .and_then(|local| local.as_ref().ok())
        .map(|local| local.mods.clone())
        .unwrap_or_default();
    for name in event.addon_names.keys() {
        if !mods.contains(name) {
            mods.push(name.clone());
        }
    }
    mods
}

/// The selection saved into the event: the checked mods in list order; DLCs and userconfig
/// folders unchanged.
fn selection(mods: &[String], addons: &BTreeMap<String, bool>, event: &Event) -> EventSelection {
    EventSelection {
        addon_names: mods
            .iter()
            .filter_map(|name| addons.get(name).map(|optional| (name.clone(), *optional)))
            .collect(),
        dlc_names: event
            .dlc_names
            .iter()
            .map(|(name, optional)| (name.clone(), *optional))
            .collect(),
        userconfig_folder_names: event
            .userconfig_folder_names
            .iter()
            .map(|(name, optional)| (name.clone(), *optional))
            .collect(),
    }
}

/// Changes the events and saves them; a failure is a toast and `None`.
fn edit<R>(
    cx: &mut ScreenCx<'_>,
    repository: &str,
    f: impl FnOnce(&mut a3s_admin::EventsDoc) -> R,
) -> Option<R> {
    let edited = cx.viewer.mutate(Some(Change::Repository), |state| {
        edit_events(state, repository, f)
    });
    match edited {
        Ok(result) => Some(result),
        Err(err) => {
            cx.toasts.error(err.user_message());
            None
        }
    }
}
