//! Repository checks: concurrency limits, the periodic cycle with persistence, the manual
//! check and proxy errors.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_foundation::error::ErrorKind;
use a3s_repo_files::A3sFile;
use a3s_repo_files::services::ClientData;
use a3s_sync::{EventError, Phase, RemoteError, RemoteMetadata, SyncEvent};
use a3s_transport::proxy::ProxyError;
use a3s_types::constants::RepositoryStatus;
use a3s_types::local::{Configuration, Proxy, Repository};
use a3s_types::{AutoConfig, FavoriteServer, Protocol, ProtocolType, ServerInfo};
use a3s_viewer_context::repositories::{Command as RepositoryCommand, Event as RepositoryEvent};
use a3s_viewer_context::{
    AppEvent, Change, CheckOutcome, CheckTrigger, Command, RepositoryCheckResult, TaskEvent,
    TaskKind, TaskOutcome, sync,
};
use tokio::sync::Semaphore;

use crate::support::{Fixture, FixtureOptions, Script, http_repository, store};

fn check(names: &[&str]) -> Command {
    Command::Repositories(RepositoryCommand::Check {
        names: names.iter().map(|name| (*name).to_owned()).collect(),
    })
}

fn completed(events: &[AppEvent], trigger: CheckTrigger) -> Vec<RepositoryCheckResult> {
    events
        .iter()
        .find_map(|event| match event {
            AppEvent::Repositories(RepositoryEvent::CheckCompleted {
                trigger: event_trigger,
                results,
            }) if *event_trigger == trigger => Some(results.clone()),
            _ => None,
        })
        .expect("CheckCompleted")
}

fn names(results: &[RepositoryCheckResult]) -> Vec<&str> {
    results.iter().map(|r| r.name.as_str()).collect()
}

fn updated_server_info(revision: i32) -> RemoteMetadata {
    RemoteMetadata {
        server_info: Some(Arc::new(ServerInfo {
            revision,
            repository_content_updated: true,
            hidden_folder_paths: BTreeSet::from(["@x/h".to_owned()]),
            number_of_connections: 3,
            ..ServerInfo::default()
        })),
        ..RemoteMetadata::default()
    }
}

fn repository(name: &str, revision: i32, notify: bool, auto: bool) -> Repository {
    let mut repository = http_repository(name);
    repository.revision = revision;
    repository.notify = notify;
    repository.auto = auto;
    repository
}

#[test]
fn at_most_four_checks_run_and_never_twice_for_one_repository() {
    let repository_names: Vec<String> = (0..10).map(|i| format!("R{i}")).collect();
    let fixture = Fixture::new(
        repository_names
            .iter()
            .map(|n| http_repository(n))
            .collect(),
    );
    let gate = Arc::new(Semaphore::new(0));
    for name in &repository_names {
        fixture.checker.set(
            name,
            Script {
                gate: Some(Arc::clone(&gate)),
                ..Script::default()
            },
        );
    }

    fixture
        .ctx
        .send(Command::Repositories(RepositoryCommand::Check {
            names: repository_names.clone(),
        }))
        .expect("send");
    std::thread::sleep(Duration::from_millis(50));
    fixture.ctx.start_background();
    std::thread::sleep(Duration::from_millis(50));
    for _ in 0..20 {
        gate.add_permits(1);
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut events = fixture.until_finished(TaskKind::CheckRepositories);
    if !events
        .iter()
        .any(|event| crate::support::finished_kind(event) == Some(TaskKind::PeriodicCheck))
    {
        events.extend(fixture.until_finished(TaskKind::PeriodicCheck));
    }
    assert_eq!(fixture.checker.max_in_flight(), 4);
    assert_eq!(fixture.checker.max_per_repository(), 1);
    assert_eq!(fixture.checker.completed(), 20);
    for trigger in [CheckTrigger::Manual, CheckTrigger::Periodic] {
        let results = completed(&events, trigger);
        assert_eq!(names(&results), repository_names);
        assert!(results.iter().all(|r| r.outcome == CheckOutcome::Ok));
    }
}

#[test]
fn periodic_cycle_applies_persists_and_decides() {
    let fixture = Fixture::new(vec![
        repository("A", 5, true, false),
        repository("B", 5, false, true),
        repository("C", 5, true, false),
        repository("D", 7, true, false),
    ]);
    let favorite = FavoriteServer {
        name: Some("Server".to_owned()),
        ip_address: Some("192.0.2.1".to_owned()),
        port: 2302,
        ..FavoriteServer::default()
    };
    let mut with_autoconfig = updated_server_info(7);
    with_autoconfig.auto_config = Some(Arc::new(AutoConfig {
        repository_name: Some("A".to_owned()),
        protocol: None,
        favorite_servers: vec![favorite],
    }));
    fixture.checker.set(
        "A",
        Script {
            install: Some(with_autoconfig),
            ..Script::default()
        },
    );
    for name in ["B", "D"] {
        fixture.checker.set(
            name,
            Script {
                install: Some(updated_server_info(7)),
                ..Script::default()
            },
        );
    }
    fixture.checker.set(
        "C",
        Script {
            fail: Some(|| RemoteError::MissingMetadata(A3sFile::ServerInfo)),
            ..Script::default()
        },
    );

    fixture.ctx.start_background();
    let events = fixture.until_finished(TaskKind::PeriodicCheck);
    assert_eq!(events.len(), 6, "{events:#?}");
    assert!(
        matches!(&events[0], AppEvent::Task(TaskEvent::Started(info)) if info.key.kind == TaskKind::PeriodicCheck)
    );
    assert!(matches!(&events[1], AppEvent::Changed(Change::Repository)));
    let results = completed(&events[2..3], CheckTrigger::Periodic);
    let c_error = EventError::from_remote(&RemoteError::MissingMetadata(A3sFile::ServerInfo));
    let expected = [
        ("A", CheckOutcome::Ok, RepositoryStatus::Updated),
        ("B", CheckOutcome::Ok, RepositoryStatus::Updated),
        (
            "C",
            CheckOutcome::Failed(c_error.clone()),
            RepositoryStatus::Indeterminated,
        ),
        ("D", CheckOutcome::Ok, RepositoryStatus::Ok),
    ];
    assert_eq!(results.len(), expected.len());
    for (result, (name, outcome, status)) in results.iter().zip(expected) {
        assert_eq!(result.name, name);
        assert_eq!(result.outcome, outcome);
        assert_eq!(result.status, Some(status));
    }
    assert!(matches!(
        &events[3],
        AppEvent::Sync(sync::Event::AutoUpdateRequested { repositories }) if repositories == &["B"]
    ));
    assert!(matches!(
        &events[4],
        AppEvent::Repositories(RepositoryEvent::UpdatesAvailable { repositories }) if repositories == &["A"]
    ));
    assert!(matches!(
        &events[5],
        AppEvent::Task(TaskEvent::Finished {
            outcome: TaskOutcome::Completed,
            ..
        })
    ));

    {
        let state = fixture.ctx.read();
        let c = state.repositories.runtime("C");
        assert_eq!(c.last_check_error, Some(c_error));
        assert_eq!(c.status, RepositoryStatus::Indeterminated);
        assert!(!c.checking);
        assert_eq!(
            state.repositories.runtime("A").status,
            RepositoryStatus::Updated
        );
    }

    let reloaded = ClientData::load(store(fixture.dir.path())).data;
    let a = reloaded.repository("A").expect("A persisted");
    assert!(a.hidden_folder_paths.contains("@x/h"));
    assert_eq!(a.number_of_client_connections, 3);
    assert!(
        reloaded
            .configuration()
            .favorite_servers
            .iter()
            .any(|server| server.repository_name.as_deref() == Some("A"))
    );
}

#[test]
fn manual_check_reports_in_name_order_without_notifications() {
    let fixture = Fixture::new(vec![
        repository("A", 5, true, false),
        repository("B", 5, true, true),
    ]);
    for name in ["A", "B"] {
        fixture.checker.set(
            name,
            Script {
                install: Some(updated_server_info(9)),
                ..Script::default()
            },
        );
    }
    fixture.ctx.send(check(&["B", "A", "zz"])).expect("send");
    let events = fixture.until_finished(TaskKind::CheckRepositories);
    let phases = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                AppEvent::Task(TaskEvent::Sync {
                    event: SyncEvent::Phase(Phase::CheckingRepository),
                    ..
                })
            )
        })
        .count();
    assert_eq!(phases, 2);
    let changed = events
        .iter()
        .position(|event| matches!(event, AppEvent::Changed(Change::Repository)))
        .expect("Changed");
    let check_completed = events
        .iter()
        .position(|event| {
            matches!(
                event,
                AppEvent::Repositories(RepositoryEvent::CheckCompleted { .. })
            )
        })
        .expect("CheckCompleted");
    assert!(changed < check_completed);
    let results = completed(&events, CheckTrigger::Manual);
    assert_eq!(names(&results), ["A", "B"]);
    assert!(
        results
            .iter()
            .all(|r| r.status == Some(RepositoryStatus::Updated))
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        AppEvent::Sync(_) | AppEvent::Repositories(RepositoryEvent::UpdatesAvailable { .. })
    )));
    assert!(matches!(
        events.last(),
        Some(AppEvent::Task(TaskEvent::Finished {
            outcome: TaskOutcome::Completed,
            ..
        }))
    ));
}

#[test]
fn manual_check_of_nothing_completes_at_once() {
    let fixture = Fixture::new(vec![http_repository("A")]);
    fixture.ctx.send(check(&[])).expect("send");
    let event = fixture.next();
    assert!(matches!(
        event,
        AppEvent::Repositories(RepositoryEvent::CheckCompleted { trigger: CheckTrigger::Manual, results })
            if results.is_empty()
    ));
    assert!(fixture.quiet().is_empty());
    assert_eq!(fixture.checker.calls(), 0);
}

#[test]
fn a_second_manual_check_is_refused_and_a_cancel_stops_both_checks() {
    let fixture = Fixture::new(vec![http_repository("A"), http_repository("B")]);
    let gate = Arc::new(Semaphore::new(0));
    for name in ["A", "B"] {
        fixture.checker.set(
            name,
            Script {
                install: Some(updated_server_info(9)),
                gate: Some(Arc::clone(&gate)),
                ..Script::default()
            },
        );
    }
    let previous = updated_server_info(1);
    fixture
        .ctx
        .mutate(None, |state| {
            state.repositories.runtime_mut("A").metadata = previous.clone();
            Ok::<_, std::convert::Infallible>(())
        })
        .expect("in memory");

    fixture.ctx.send(check(&["A", "B"])).expect("send");
    fixture.until(|event| {
        matches!(
            event,
            AppEvent::Task(TaskEvent::Sync {
                event: SyncEvent::Phase(_),
                ..
            })
        )
    });
    fixture.ctx.send(check(&["A"])).expect("send");
    let refused = fixture.until(|event| matches!(event, AppEvent::Error { .. }));
    let Some(AppEvent::Error { error, .. }) = refused.last() else {
        unreachable!();
    };
    assert_eq!(error.kind, ErrorKind::InvalidInput);

    let cancelled_at = Instant::now();
    fixture
        .ctx
        .send(Command::Repositories(RepositoryCommand::CancelCheck))
        .expect("send");
    let events = fixture.until_finished(TaskKind::CheckRepositories);
    assert!(cancelled_at.elapsed() < Duration::from_secs(1));
    let results = completed(&events, CheckTrigger::Manual);
    assert_eq!(names(&results), ["A", "B"]);
    assert!(
        results
            .iter()
            .all(|r| r.outcome == CheckOutcome::Cancelled && r.status.is_none())
    );
    assert!(matches!(
        events.last(),
        Some(AppEvent::Task(TaskEvent::Finished {
            outcome: TaskOutcome::Cancelled,
            ..
        }))
    ));
    let state = fixture.ctx.read();
    assert_eq!(state.repositories.runtime("A").metadata, previous);
    assert_eq!(
        state.repositories.runtime("B").metadata,
        RemoteMetadata::default()
    );
}

#[test]
fn an_invalid_proxy_fails_the_check_without_connecting() {
    let configuration = Configuration {
        proxy: Proxy {
            enable_proxy: true,
            proxy_protocol: Some(Protocol::new(
                ProtocolType::Http,
                "proxy.example",
                "x",
                "anonymous",
                "",
                false,
            )),
        },
        ..Configuration::default()
    };
    let fixture = Fixture::with(FixtureOptions {
        repositories: vec![http_repository("A")],
        configuration,
        ..FixtureOptions::default()
    });
    fixture
        .ctx
        .mutate(None, |state| {
            state.repositories.runtime_mut("A").metadata = updated_server_info(1);
            Ok::<_, std::convert::Infallible>(())
        })
        .expect("in memory");

    fixture.ctx.send(check(&["A"])).expect("send");
    let events = fixture.until_finished(TaskKind::CheckRepositories);
    let results = completed(&events, CheckTrigger::Manual);
    let expected = EventError::new(
        ErrorKind::InvalidInput,
        ProxyError::InvalidPort("x".to_owned()).to_string(),
    );
    assert_eq!(results[0].outcome, CheckOutcome::Failed(expected));
    assert_eq!(fixture.checker.calls(), 0);
    assert!(
        fixture
            .ctx
            .read()
            .repositories
            .runtime("A")
            .metadata
            .server_info
            .is_none()
    );
}
