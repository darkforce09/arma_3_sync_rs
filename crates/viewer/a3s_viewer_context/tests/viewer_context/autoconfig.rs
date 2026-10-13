//! The favourite servers a published repository puts in its auto-config.

use a3s_repo_files::services::ServiceError;
use a3s_types::FavoriteServer;
use a3s_viewer_context::admin;

use crate::support::{Fixture, http_repository};

fn server(name: &str, port: i32) -> FavoriteServer {
    FavoriteServer {
        name: Some(name.to_owned()),
        ip_address: Some("10.0.0.1".to_owned()),
        port,
        ..FavoriteServer::default()
    }
}

/// The included servers by name, in the client's order (sorted by name).
fn included(fixture: &Fixture) -> Vec<String> {
    admin::autoconfig_servers(&fixture.ctx.read().client, "R")
        .into_iter()
        .filter(|entry| entry.included)
        .filter_map(|entry| entry.server.name)
        .collect()
}

#[test]
fn servers_are_included_one_by_one_and_keep_the_others() {
    let fixture = Fixture::new(vec![http_repository("R")]);
    fixture
        .ctx
        .mutate(None, |state| {
            for (name, port) in [("Main", 2302), ("Training", 2402), ("Event", 2502)] {
                state.client.add_favorite_server(server(name, port))?;
            }
            Ok::<(), ServiceError>(())
        })
        .expect("servers");
    assert!(included(&fixture).is_empty());

    let set = |name: &str, on: bool| {
        let index = admin::autoconfig_servers(&fixture.ctx.read().client, "R")
            .iter()
            .position(|entry| entry.server.name.as_deref() == Some(name))
            .expect("server");
        fixture
            .ctx
            .mutate(None, |state| {
                admin::set_autoconfig_server(&mut state.client, "R", index, on)
            })
            .expect("stored");
    };
    set("Training", true);
    set("Event", true);
    assert_eq!(included(&fixture), ["Event", "Training"]);
    let stored: Vec<_> = fixture
        .ctx
        .read()
        .client
        .repository("R")
        .expect("repository")
        .favorite_servers_set_to_autoconfig
        .iter()
        .map(|server| (server.name.clone(), server.repository_name.clone()))
        .collect();
    let tagged = |name: &str| (Some(name.to_owned()), Some("R".to_owned()));
    assert_eq!(stored, [tagged("Event"), tagged("Training")]);

    set("Training", false);
    assert_eq!(included(&fixture), ["Event"]);
}

#[test]
fn a_changed_favourite_server_is_no_longer_included() {
    let fixture = Fixture::new(vec![http_repository("R")]);
    fixture
        .ctx
        .mutate(None, |state| {
            state.client.add_favorite_server(server("Main", 2302))?;
            admin::set_autoconfig_server(&mut state.client, "R", 0, true)?;
            state.client.update_configuration(|configuration| {
                configuration.favorite_servers[0].port = 2402;
                true
            })
        })
        .expect("stored");
    assert!(included(&fixture).is_empty());
}

#[test]
fn an_unknown_repository_or_server_is_refused() {
    let fixture = Fixture::new(vec![http_repository("R")]);
    let result = fixture.ctx.mutate(None, |state| {
        state.client.add_favorite_server(server("Main", 2302))?;
        admin::set_autoconfig_server(&mut state.client, "R", 1, true)
    });
    assert!(matches!(
        result,
        Err(ServiceError::FavoriteServerIndex { index: 1, len: 1 })
    ));
    let result = fixture.ctx.mutate(None, |state| {
        admin::set_autoconfig_server(&mut state.client, "X", 0, true)
    });
    assert!(matches!(result, Err(ServiceError::RepositoryNotFound(_))));
    assert!(admin::autoconfig_servers(&fixture.ctx.read().client, "X").is_empty());
}
