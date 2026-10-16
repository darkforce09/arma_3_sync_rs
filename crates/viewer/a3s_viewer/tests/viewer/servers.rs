//! The Servers screen: add, edit, join on play, remove and undo, all written at once.

use std::sync::Arc;

use a3s_repo_files::services::{ClientData, LoadOutcome};
use a3s_viewer::{Screen, SheetKind};
use egui_kittest::kittest::Queryable as _;

use crate::support::{Fixture, Start, fixture};

fn reload(fixture: &Fixture) -> ClientData {
    let LoadOutcome { data, .. } = ClientData::load(Arc::clone(&fixture.store));
    data
}

fn servers_screen() -> Fixture {
    let mut fixture = fixture(Start::default());
    fixture.harness.get_by_label("Servers").click();
    fixture.harness.run();
    assert_eq!(*fixture.harness.state().screen(), Screen::Servers);
    fixture
}

fn add_server(fixture: &mut Fixture, name: &str, address: &str) {
    let harness = &mut fixture.harness;
    harness.get_by_label_contains("Add server").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::AddServer));
    harness.get_by_label("Name").focus();
    harness.get_by_label("Name").type_text(name);
    harness.get_by_label("Address").focus();
    harness.get_by_label("Address").type_text(address);
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
}

#[test]
fn servers_are_added_edited_and_joined() {
    let mut fixture = servers_screen();
    add_server(&mut fixture, "Main", "10.0.0.1");
    let servers = reload(&fixture).favorite_servers().to_vec();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name.as_deref(), Some("Main"));
    assert_eq!(servers[0].ip_address.as_deref(), Some("10.0.0.1"));
    assert_eq!(servers[0].port, 2302);

    let harness = &mut fixture.harness;
    harness.get_by_label("Join").click();
    harness.run();
    assert_eq!(
        reload(&fixture).configuration().server_name.as_deref(),
        Some("Main")
    );

    // A double-click edits; the joined server stays joined under its new name. The earlier
    // clicks must be out of the triple-click window first.
    fixture.harness.run_steps(60);
    let harness = &mut fixture.harness;
    harness.get_by_label("Main").click();
    harness.step();
    harness.get_by_label("Main").click();
    harness.run();
    assert_eq!(harness.state().sheet(), Some(SheetKind::EditServer));
    harness.get_by_label("Name").focus();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Name").type_text("Main EU");
    harness.run();
    harness.get_by_label("Save").click();
    harness.run();
    let data = reload(&fixture);
    assert_eq!(data.favorite_servers()[0].name.as_deref(), Some("Main EU"));
    assert_eq!(data.configuration().server_name.as_deref(), Some("Main EU"));

    let harness = &mut fixture.harness;
    harness.get_by_label("Don't join").click();
    harness.run();
    assert_eq!(reload(&fixture).configuration().server_name, None);
}

#[test]
fn a_removed_server_comes_back_with_undo() {
    let mut fixture = servers_screen();
    add_server(&mut fixture, "Practice", "10.0.0.2");
    let harness = &mut fixture.harness;
    harness.get_by_label("Join").click();
    harness.run();
    harness.get_by_label("Practice").click();
    harness.run();
    harness.get_by_label_contains("Remove").click();
    harness.run();
    let data = reload(&fixture);
    assert!(data.favorite_servers().is_empty());
    assert_eq!(data.configuration().server_name, None);

    let harness = &mut fixture.harness;
    harness.get_by_label("Undo").click();
    harness.run();
    let data = reload(&fixture);
    assert_eq!(data.favorite_servers().len(), 1);
    assert_eq!(
        data.configuration().server_name.as_deref(),
        Some("Practice")
    );
}

#[test]
fn the_search_narrows_the_servers() {
    let mut fixture = servers_screen();
    add_server(&mut fixture, "Main", "10.0.0.1");
    add_server(&mut fixture, "Training", "10.0.0.2");
    let harness = &mut fixture.harness;
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    harness.run();
    harness.event(egui::Event::Text("train".to_owned()));
    harness.run();
    assert!(harness.query_by_label("Main").is_none());
    harness.get_by_label("Training");
}
