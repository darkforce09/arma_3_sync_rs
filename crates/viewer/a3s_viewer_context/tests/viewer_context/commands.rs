//! The command bus, the automatic update hook and the synchronization store.

use std::sync::Arc;

use a3s_repo_files::services::ClientData;
use a3s_sync::cleanup::RemovedFiles;
use a3s_sync::pipeline::{StoreChanges, SyncEnd, SyncJob, SyncRunReport, SyncStore};
use a3s_sync::{EventSender, Outcome};
use a3s_transport::CancellationToken;
use a3s_types::constants::RepositoryStatus;
use a3s_viewer_context::repositories::{Command as RepositoryCommand, Event as RepositoryEvent};
use a3s_viewer_context::{
    AppEvent, Change, Command, ContextOptions, Synchronizer, TaskEvent, TaskKey, TaskKind,
    TaskOutcome, ViewerContext, sync,
};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use parking_lot::Mutex;

use crate::support::{EVENT_TIMEOUT, Fixture, FixtureOptions, http_repository, store};

fn auto_update(names: &[&str]) -> Command {
    Command::Sync(sync::Command::AutoUpdate {
        repositories: names.iter().map(|name| (*name).to_owned()).collect(),
    })
}

#[test]
fn auto_update_command_emits_the_request() {
    let fixture = Fixture::new(Vec::new());
    fixture.ctx.send(auto_update(&["R"])).expect("send");
    assert!(matches!(
        fixture.next(),
        AppEvent::Sync(sync::Event::AutoUpdateRequested { repositories }) if repositories == ["R"]
    ));
    assert!(fixture.ctx.running_tasks().is_empty());
}

#[test]
fn commands_are_handled_in_send_order() {
    let fixture = Fixture::new(Vec::new());
    for i in 0..50 {
        fixture
            .ctx
            .send(auto_update(&[&i.to_string()]))
            .expect("send");
        fixture
            .ctx
            .send(Command::Repositories(RepositoryCommand::Check {
                names: Vec::new(),
            }))
            .expect("send");
    }
    for i in 0..50 {
        assert!(matches!(
            fixture.next(),
            AppEvent::Sync(sync::Event::AutoUpdateRequested { repositories }) if repositories == [i.to_string()]
        ));
        assert!(matches!(
            fixture.next(),
            AppEvent::Repositories(RepositoryEvent::CheckCompleted { results, .. }) if results.is_empty()
        ));
    }
}

/// Domains without commands still dispatch: their enums are uninhabited.
#[expect(dead_code, reason = "a compile-time check")]
fn empty_domains_match(command: Command) -> Command {
    match command {
        Command::Admin(command) => match command {},
        Command::Profiles(command) => match command {},
        Command::Launch(command) => match command {},
        Command::Tools(command) => match command {},
        other @ (Command::Repositories(_) | Command::Sync(_)) => other,
    }
}

#[expect(dead_code, reason = "a compile-time check")]
fn empty_events_match(event: AppEvent) -> bool {
    match event {
        AppEvent::Admin(event) => match event {},
        AppEvent::Profiles(event) => match event {},
        AppEvent::Launch(event) => match event {},
        AppEvent::Tools(event) => match event {},
        _ => true,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_context_runs_on_an_external_runtime() {
    let dir = tempfile::tempdir().expect("temp dir");
    let client = ClientData::from_parts(
        store(dir.path()),
        Default::default(),
        Default::default(),
        Vec::new(),
        Vec::new(),
    );
    let ctx = ViewerContext::new(
        client,
        ContextOptions {
            runtime: Some(tokio::runtime::Handle::current()),
            synchronizer: None,
            ..ContextOptions::default()
        },
    )
    .expect("context");
    ctx.send(auto_update(&["R"])).expect("send");
    let event = tokio::task::spawn_blocking(move || {
        let event = ctx.next_event_timeout(EVENT_TIMEOUT);
        drop(ctx);
        event
    })
    .await
    .expect("join");
    assert!(matches!(
        event,
        Some(AppEvent::Sync(sync::Event::AutoUpdateRequested { .. }))
    ));
}

/// Records the job and stores a new revision through the store, like a real synchronization.
#[derive(Default)]
struct FakeSynchronizer {
    jobs: Mutex<Vec<SyncJob>>,
}

impl Synchronizer for FakeSynchronizer {
    fn synchronize(
        &self,
        job: SyncJob,
        store: Arc<dyn SyncStore>,
        _cancel: CancellationToken,
        _events: EventSender,
    ) -> BoxFuture<'static, SyncRunReport> {
        let name = job.check.repository_name.clone();
        self.jobs.lock().push(job);
        async move {
            let stored = store.update(&name, &mut |repository, _| {
                repository.revision = 9;
                StoreChanges {
                    repository: true,
                    configuration: false,
                }
            });
            SyncRunReport {
                end: SyncEnd::Finished(if stored.is_ok() {
                    Outcome::Success
                } else {
                    Outcome::Cancelled
                }),
                status: Some(RepositoryStatus::Ok),
                download: None,
                reconnects: 0,
                userconfig: None,
                remaining_downloads: Some(0),
                report_text: Some("Download finished successfully.".to_owned()),
            }
        }
        .boxed()
    }
}

#[test]
fn an_automatic_update_synchronizes_in_the_background() {
    let synchronizer = Arc::new(FakeSynchronizer::default());
    let fixture = Fixture::with(FixtureOptions {
        repositories: vec![http_repository("R")],
        options: ContextOptions {
            worker_threads: 2,
            synchronizer: Some(synchronizer.clone()),
            ..ContextOptions::default()
        },
        ..FixtureOptions::default()
    });
    fixture.ctx.send(auto_update(&["R"])).expect("send");
    let events = fixture.until_finished(TaskKind::AutoUpdate);
    assert!(matches!(
        &events[0],
        AppEvent::Sync(sync::Event::AutoUpdateRequested { repositories }) if repositories == &["R"]
    ));
    assert!(matches!(
        &events[1],
        AppEvent::Task(TaskEvent::Started(info))
            if info.key == TaskKey::repository(TaskKind::AutoUpdate, "R")
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AppEvent::Changed(Change::AddonFiles)))
    );
    assert!(matches!(
        events.last(),
        Some(AppEvent::Task(TaskEvent::Finished {
            outcome: TaskOutcome::Completed,
            ..
        }))
    ));

    let jobs = synchronizer.jobs.lock();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].check.removed_files, RemovedFiles::Trash);
    let state = fixture.ctx.read();
    assert_eq!(state.client.repository("R").expect("stored").revision, 9);
    let runtime = state.repositories.runtime("R");
    assert_eq!(runtime.status, RepositoryStatus::Ok);
    assert!(runtime.download_report.is_some());
    drop(state);
    let reloaded = ClientData::load(store(fixture.dir.path())).data;
    assert_eq!(reloaded.repository("R").expect("persisted").revision, 9);
}
