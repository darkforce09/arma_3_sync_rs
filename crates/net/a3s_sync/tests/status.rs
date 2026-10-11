//! Repository status, metadata merge, revision handling and check-cycle decisions.

use std::sync::Arc;

use a3s_sync::RemoteMetadata;
use a3s_sync::status::{
    CheckCycleDecision, CheckedRepository, NotificationTracker, decide_check_cycle,
    determine_sync_status, mark_failed, mark_synced, merge_favorite_servers, update_repository,
};
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Configuration, Repository};
use a3s_types::{
    AutoConfig, Changelog, Changelogs, FavoriteServer, Protocol, ProtocolType, ServerInfo,
};

fn server(revision: i32, content_updated: bool) -> ServerInfo {
    ServerInfo {
        revision,
        repository_content_updated: content_updated,
        ..Default::default()
    }
}

fn changelogs(entries: &[(i32, bool)]) -> Changelogs {
    Changelogs {
        list: entries
            .iter()
            .map(|&(revision, content_updated)| Changelog {
                revision,
                content_updated,
                ..Default::default()
            })
            .collect(),
    }
}

fn repo() -> Repository {
    Repository::new(
        "WOG",
        Protocol::new(ProtocolType::Ftp, "host/path", "21", "anonymous", "", false),
    )
}

fn fav(name: Option<&str>, ip: Option<&str>, port: i32) -> FavoriteServer {
    FavoriteServer {
        name: name.map(str::to_owned),
        ip_address: ip.map(str::to_owned),
        port,
        ..Default::default()
    }
}

#[test]
fn determine_sync_status_table() {
    use RepositoryStatus::{Indeterminated, Ok, Updated};

    let s10 = server(10, false);
    let logs = changelogs(&[(8, true), (9, false), (10, false)]);
    type Row<'a> = (
        &'a str,
        i32,
        Option<&'a ServerInfo>,
        Option<&'a Changelogs>,
        RepositoryStatus,
    );
    let content_updated = server(10, true);
    let rows: [Row<'_>; 11] = [
        ("no server info", 10, None, Some(&logs), Indeterminated),
        ("equal", 10, Some(&s10), None, Ok),
        ("local ahead", 11, Some(&s10), Some(&logs), Indeterminated),
        ("content updated", 5, Some(&content_updated), None, Updated),
        ("no changelogs", 9, Some(&s10), None, Indeterminated),
        (
            "local not in changelogs",
            7,
            Some(&s10),
            Some(&logs),
            Indeterminated,
        ),
        (
            "a later true",
            8,
            Some(&s10),
            Some(&changelogs(&[(8, false), (9, false), (10, true)])),
            Updated,
        ),
        ("all later false", 8, Some(&s10), Some(&logs), Ok),
        (
            "duplicate revision, last wins",
            8,
            Some(&s10),
            Some(&changelogs(&[
                (8, false),
                (9, true),
                (9, false),
                (10, false),
            ])),
            Ok,
        ),
        (
            "earlier true ignored",
            9,
            Some(&s10),
            Some(&changelogs(&[(8, true), (9, true), (10, false)])),
            Ok,
        ),
        (
            "local -1",
            -1,
            Some(&s10),
            Some(&changelogs(&[(-1, false), (10, true)])),
            Updated,
        ),
    ];
    for (label, local, server_info, logs, expected) in rows {
        assert_eq!(
            determine_sync_status(local, server_info, logs),
            expected,
            "{label}"
        );
    }
    // -1 is usually missing from the changelogs.
    assert_eq!(
        determine_sync_status(-1, Some(&s10), Some(&logs)),
        Indeterminated
    );
}

#[test]
fn update_repository_server_info() {
    let mut r = repo();
    r.hidden_folder_paths.insert("@local".to_owned());
    r.revision = 10;
    let mut c = Configuration::default();
    let mut info = server(10, false);
    info.hidden_folder_paths.insert("@server".to_owned());
    info.number_of_connections = 3;
    let metadata = RemoteMetadata {
        server_info: Some(Arc::new(info.clone())),
        ..Default::default()
    };

    let update = update_repository(&mut r, &mut c, &metadata);
    assert_eq!(update.status, RepositoryStatus::Ok);
    assert!(update.repository_changed);
    assert!(!update.configuration_changed);
    assert_eq!(
        r.hidden_folder_paths.iter().collect::<Vec<_>>(),
        ["@local", "@server"]
    );
    assert_eq!(r.number_of_client_connections, 3);

    // Nothing new: no change. A non-zero client count is kept.
    let update = update_repository(&mut r, &mut c, &metadata);
    assert!(!update.repository_changed);
    r.number_of_client_connections = 7;
    let update = update_repository(&mut r, &mut c, &metadata);
    assert!(!update.repository_changed);
    assert_eq!(r.number_of_client_connections, 7);

    // No server info: no change, indeterminate.
    let mut r = repo();
    let update = update_repository(&mut r, &mut c, &RemoteMetadata::default());
    assert_eq!(
        update,
        a3s_sync::status::RepositoryUpdate {
            status: RepositoryStatus::Indeterminated,
            repository_changed: false,
            configuration_changed: false,
        }
    );
    assert_eq!(r.number_of_client_connections, 0);
}

#[test]
fn update_repository_merges_and_sorts_favorites() {
    let mut r = repo();
    let mut c = Configuration::default();
    let metadata = RemoteMetadata {
        auto_config: Some(Arc::new(AutoConfig {
            repository_name: Some("WOG".to_owned()),
            protocol: None,
            favorite_servers: vec![
                fav(Some("zeta"), Some("1.1.1.1"), 1),
                fav(Some("Alpha"), Some("2.2.2.2"), 2),
            ],
        })),
        ..Default::default()
    };
    let update = update_repository(&mut r, &mut c, &metadata);
    assert!(update.configuration_changed);
    assert!(!update.repository_changed);
    let names: Vec<_> = c
        .favorite_servers
        .iter()
        .map(|s| s.name.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(names, ["Alpha", "zeta"]);

    let update = update_repository(&mut r, &mut c, &metadata);
    assert!(!update.configuration_changed);
}

#[test]
fn merge_matches_against_the_sorted_list() {
    // Two local servers share ip and port; Java matches the first one in *sorted* order.
    let mut c = Configuration {
        favorite_servers: vec![
            fav(Some("zulu"), Some("1.1.1.1"), 1),
            fav(Some("Alpha"), Some("1.1.1.1"), 1),
        ],
        ..Default::default()
    };
    let mut remote = fav(None, Some("1.1.1.1"), 1);
    remote.password = Some("pw".to_owned());
    let metadata = RemoteMetadata {
        auto_config: Some(Arc::new(AutoConfig {
            repository_name: None,
            protocol: None,
            favorite_servers: vec![remote],
        })),
        ..Default::default()
    };
    let update = update_repository(&mut repo(), &mut c, &metadata);
    assert!(update.configuration_changed);
    assert_eq!(c.favorite_servers[0].name.as_deref(), Some("Alpha"));
    assert_eq!(c.favorite_servers[0].password.as_deref(), Some("pw"));
    assert_eq!(c.favorite_servers[1].password, None);
}

#[test]
fn merge_into_empty_list() {
    let mut local = Vec::new();
    let mut remote = fav(Some(""), Some("1.2.3.4"), 2302);
    remote.password = Some("pw".to_owned());
    remote.modset_name = Some("event".to_owned());
    remote.selected = true;
    remote.repository_name = Some("other".to_owned());
    let named = fav(Some("Main"), Some("5.6.7.8"), 2402);
    let nameless = fav(None, Some("9.9.9.9"), 1);

    assert!(merge_favorite_servers(
        &mut local,
        &[remote, named, nameless],
        "WOG"
    ));
    assert_eq!(
        local[0],
        FavoriteServer {
            name: Some("WOG".to_owned()),
            ip_address: Some("1.2.3.4".to_owned()),
            port: 2302,
            password: Some("pw".to_owned()),
            selected: false,
            modset_name: Some("event".to_owned()),
            repository_name: Some("WOG".to_owned()),
        }
    );
    assert_eq!(local[1].name.as_deref(), Some("Main"));
    assert_eq!(local[2].name.as_deref(), Some("WOG"));
    assert!(!merge_favorite_servers(&mut Vec::new(), &[], "WOG"));
}

#[test]
fn merge_matches_by_ip_and_port() {
    let mut existing = fav(Some(""), Some("1.2.3.4"), 2302);
    existing.password = Some("old".to_owned());
    existing.modset_name = Some("keep".to_owned());
    existing.repository_name = Some("old repo".to_owned());
    let other_port = fav(Some("Other"), Some("1.2.3.4"), 9999);
    let mut local = vec![existing, other_port.clone()];

    let mut remote = fav(Some("Remote name"), Some("1.2.3.4"), 2302);
    remote.password = None;
    remote.modset_name = Some(String::new());
    assert!(merge_favorite_servers(&mut local, &[remote], "WOG"));
    assert_eq!(local.len(), 2);
    assert_eq!(
        local[0],
        FavoriteServer {
            // Empty local name defaults to the repository; remote name ignored.
            name: Some("WOG".to_owned()),
            ip_address: Some("1.2.3.4".to_owned()),
            port: 2302,
            // Overwritten even with null.
            password: None,
            selected: false,
            // Kept: the remote modset is empty.
            modset_name: Some("keep".to_owned()),
            repository_name: Some("WOG".to_owned()),
        }
    );
    assert_eq!(local[1], other_port);

    // A non-empty remote modset replaces the local one; the local name is kept.
    let mut remote = fav(None, Some("1.2.3.4"), 9999);
    remote.modset_name = Some("new".to_owned());
    assert!(merge_favorite_servers(&mut local, &[remote.clone()], "WOG"));
    assert_eq!(local[1].name.as_deref(), Some("Other"));
    assert_eq!(local[1].modset_name.as_deref(), Some("new"));
    assert!(!merge_favorite_servers(&mut local, &[remote], "WOG"));
}

#[test]
fn merge_appends_unmatched_and_duplicates() {
    let mut local = vec![fav(Some("A"), Some("1.1.1.1"), 1)];
    let new = fav(Some("New"), Some("2.2.2.2"), 2);
    assert!(merge_favorite_servers(
        &mut local,
        &[new.clone(), new],
        "WOG"
    ));
    assert_eq!(local.len(), 3, "duplicates are appended twice");
    assert_eq!(local[1], local[2]);
    assert_eq!(local[1].repository_name.as_deref(), Some("WOG"));
}

#[test]
fn merge_null_ip_matches_null_ip() {
    let mut local = vec![fav(Some("A"), None, 5)];
    let mut remote = fav(Some("B"), None, 5);
    remote.password = Some("pw".to_owned());
    assert!(merge_favorite_servers(&mut local, &[remote], "WOG"));
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].password.as_deref(), Some("pw"));
}

#[test]
fn mark_synced_and_failed() {
    let mut r = repo();
    r.revision = 3;
    assert_eq!(
        mark_synced(&mut r, &RemoteMetadata::default()),
        RepositoryStatus::Ok
    );
    assert_eq!(r.revision, 3);

    let metadata = RemoteMetadata {
        server_info: Some(Arc::new(server(55, false))),
        ..Default::default()
    };
    assert_eq!(mark_synced(&mut r, &metadata), RepositoryStatus::Ok);
    assert_eq!(r.revision, 55);

    assert_eq!(mark_failed(&mut r), RepositoryStatus::Error);
    assert_eq!(r.revision, -1);
}

fn checked(
    name: &str,
    notify: bool,
    auto: bool,
    status: RepositoryStatus,
    rev: i32,
) -> CheckedRepository {
    CheckedRepository {
        name: name.to_owned(),
        notify,
        auto,
        status,
        server_revision: Some(rev),
    }
}

#[test]
fn decide_check_cycle_cases() {
    let mut tracker = NotificationTracker::new();
    let repos = [
        checked("notify", true, false, RepositoryStatus::Updated, 5),
        checked("auto", true, true, RepositoryStatus::Updated, 7),
        checked("ok", true, false, RepositoryStatus::Ok, 1),
    ];
    let decision = decide_check_cycle(&repos, &mut tracker);
    assert_eq!(
        decision,
        CheckCycleDecision {
            updated: vec!["notify".to_owned(), "auto".to_owned()],
            auto_update: vec!["auto".to_owned()],
            notify: vec!["notify".to_owned()],
        }
    );
    assert_eq!(
        decision.console_message().as_deref(),
        Some("The following repositories have been updated: \nnotify\nauto")
    );

    // Same revisions again: nothing new to notify.
    let decision = decide_check_cycle(&repos, &mut tracker);
    assert!(decision.notify.is_empty());
    assert_eq!(decision.updated.len(), 2);

    // The repository was synchronized (Ok), then updated again at the same revision.
    let mut repos2 = repos.clone();
    repos2[0].status = RepositoryStatus::Ok;
    let decision = decide_check_cycle(&repos2, &mut tracker);
    assert!(decision.notify.is_empty());
    assert_eq!(decision.updated, ["auto"]);
    let decision = decide_check_cycle(&repos, &mut tracker);
    assert_eq!(decision.notify, ["notify"]);

    let none = decide_check_cycle(
        &[checked("ok", true, false, RepositoryStatus::Ok, 1)],
        &mut tracker,
    );
    assert_eq!(none, CheckCycleDecision::default());
    assert_eq!(none.console_message(), None);
}
