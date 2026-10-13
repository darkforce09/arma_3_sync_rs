//! `mutate` and its change events, runtime defaults, and the repository wrappers.

use std::sync::Arc;

use a3s_repo_files::services::ServiceError;
use a3s_sync::RemoteMetadata;
use a3s_types::ServerInfo;
use a3s_types::constants::RepositoryStatus;
use a3s_viewer_context::{
    AppEvent, Change, ContextOptions, RepositoryRuntime, TaskError, TaskEvent, TaskKey, TaskKind,
    TaskOutcome,
};

use crate::support::{Fixture, FixtureOptions, http_repository};

#[test]
fn mutate_emits_its_change_only_on_success() {
    let fixture = Fixture::new(Vec::new());
    fixture
        .ctx
        .mutate(Some(Change::Profile), |state| {
            state.client.create_profile("P")
        })
        .expect("new profile");
    assert!(matches!(fixture.next(), AppEvent::Changed(Change::Profile)));

    let again = fixture.ctx.mutate(Some(Change::Profile), |state| {
        state.client.create_profile("P")
    });
    assert!(matches!(again, Err(ServiceError::ProfileExists(_))));
    fixture
        .ctx
        .mutate(None, |state| state.client.create_profile("Q"))
        .expect("new profile");
    assert!(fixture.quiet().is_empty());
}

#[test]
fn flags_come_from_the_options() {
    let fixture = Fixture::with(FixtureOptions {
        options: ContextOptions {
            dev_mode: true,
            synchronizer: None,
            ..ContextOptions::default()
        },
        ..FixtureOptions::default()
    });
    let flags = fixture.ctx.read().flags;
    assert!(flags.dev_mode);
    assert!(!flags.run_mode);
}

#[test]
fn a_missing_runtime_entry_reads_as_the_defaults() {
    let fixture = Fixture::new(Vec::new());
    let runtime = fixture.ctx.read().repositories.runtime("missing");
    assert_eq!(runtime.status, RepositoryStatus::Indeterminated);
    assert_eq!(runtime.last_index_file_transferred, 0);
    assert_eq!(runtime.download_report, None);
    assert_eq!(runtime.metadata, RemoteMetadata::default());
    assert_eq!(runtime, RepositoryRuntime::default());
}

#[test]
fn rename_and_remove_keep_the_runtime_state_consistent() {
    let fixture = Fixture::new(vec![http_repository("A")]);
    fixture
        .ctx
        .mutate(None, |state| {
            let runtime = state.repositories.runtime_mut("A");
            runtime.status = RepositoryStatus::Updated;
            runtime.metadata.server_info = Some(Arc::new(ServerInfo::default()));
            Ok::<_, ServiceError>(())
        })
        .expect("in memory");

    fixture.ctx.rename_repository("A", "B").expect("rename");
    assert!(matches!(
        fixture.next(),
        AppEvent::Changed(Change::Repository)
    ));
    {
        let state = fixture.ctx.read();
        assert!(!state.repositories.runtime.contains_key("A"));
        assert_eq!(
            state.repositories.runtime("B").status,
            RepositoryStatus::Updated
        );
    }

    let task = fixture
        .ctx
        .cx()
        .spawn_task(
            TaskKey::repository(TaskKind::Download, "B"),
            false,
            |task| async move {
                task.cancel.cancelled().await;
                Err(TaskError::Cancelled)
            },
        )
        .expect("spawn");
    assert!(matches!(
        fixture.next(),
        AppEvent::Task(TaskEvent::Started(_))
    ));

    fixture.ctx.remove_repository("B").expect("remove");
    // The cancelled task may finish before or after the removal is announced.
    let finished = |event: &AppEvent| matches!(event, AppEvent::Task(TaskEvent::Finished { .. }));
    let mut events = fixture.until(|event| matches!(event, AppEvent::Changed(Change::Repository)));
    if !events.iter().any(finished) {
        events.extend(fixture.until(finished));
    }
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::Task(TaskEvent::Finished { info, outcome: TaskOutcome::Cancelled }) if info.id == task
    )));
    let state = fixture.ctx.read();
    assert!(state.repositories.runtime.is_empty());
    assert!(state.client.repository("B").is_err());
}
