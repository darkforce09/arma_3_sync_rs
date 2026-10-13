//! Shared fixtures: a client data store in a temporary folder, a scripted remote checker, a
//! manual clock, and event helpers.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use a3s_repo_files::local::{LocalDirs, LocalStore};
use a3s_repo_files::services::ClientData;
use a3s_sync::{Phase, RemoteError, RemoteMetadata, SyncEvent};
use a3s_types::constants::CheckRepositoriesFrequency;
use a3s_types::local::{Configuration, Preferences, Repository};
use a3s_types::{Protocol, ProtocolType};
use a3s_viewer_context::{
    AppEvent, ContextOptions, ManualClock, RemoteCheck, RemoteChecker, TaskEvent, TaskKind,
    TaskOutcome, ViewerContext,
};
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tempfile::TempDir;
use tokio::sync::Semaphore;

/// How long a test waits for an expected event.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a test waits to conclude that no event comes.
pub const QUIET: Duration = Duration::from_millis(300);

pub fn start_time() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

pub fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

pub fn store(root: &Path) -> Arc<LocalStore> {
    Arc::new(LocalStore::with_dirs(LocalDirs {
        config: root.join("config"),
        profiles: root.join("config/profiles"),
        repositories: root.join("data/repositories"),
        sha1_cache: root.join("data/sha1_cache"),
    }))
}

pub fn http_repository(name: &str) -> Repository {
    Repository::new(
        name,
        Protocol::new(
            ProtocolType::Http,
            "127.0.0.1/repo",
            "80",
            "anonymous",
            "",
            true,
        ),
    )
}

/// What the fake checker does for one repository.
#[derive(Clone, Default)]
pub struct Script {
    /// Installed into the metadata before succeeding or failing.
    pub install: Option<RemoteMetadata>,

    /// Fails with this error, clearing the server info.
    pub fail: Option<fn() -> RemoteError>,

    /// Each check takes one permit before it ends.
    pub gate: Option<Arc<Semaphore>>,
}

/// A scripted [`RemoteChecker`] that records concurrency.
#[derive(Default)]
pub struct FakeChecker {
    scripts: Mutex<BTreeMap<String, Script>>,
    in_flight: Mutex<(usize, BTreeMap<String, usize>)>,
    max_in_flight: AtomicUsize,
    max_per_repository: AtomicUsize,
    calls: AtomicUsize,
    completed: AtomicUsize,
}

impl FakeChecker {
    pub fn set(&self, name: &str, script: Script) {
        self.scripts.lock().insert(name.to_owned(), script);
    }

    pub fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    pub fn max_per_repository(&self) -> usize {
        self.max_per_repository.load(Ordering::SeqCst)
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn completed(&self) -> usize {
        self.completed.load(Ordering::SeqCst)
    }

    fn enter(&self, name: &str) -> InFlight<'_> {
        let mut in_flight = self.in_flight.lock();
        in_flight.0 += 1;
        let per_repository = in_flight.1.entry(name.to_owned()).or_default();
        *per_repository += 1;
        let per_repository = *per_repository;
        self.max_in_flight.fetch_max(in_flight.0, Ordering::SeqCst);
        self.max_per_repository
            .fetch_max(per_repository, Ordering::SeqCst);
        InFlight {
            checker: self,
            name: name.to_owned(),
        }
    }
}

struct InFlight<'a> {
    checker: &'a FakeChecker,
    name: String,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut in_flight = self.checker.in_flight.lock();
        in_flight.0 -= 1;
        if let Some(count) = in_flight.1.get_mut(&self.name) {
            *count -= 1;
        }
    }
}

impl RemoteChecker for FakeChecker {
    fn check<'a>(&'a self, request: RemoteCheck<'a>) -> BoxFuture<'a, Result<(), RemoteError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let script = self
                .scripts
                .lock()
                .get(request.repository_name)
                .cloned()
                .unwrap_or_default();
            let _in_flight = self.enter(request.repository_name);
            request
                .events
                .emit(SyncEvent::Phase(Phase::CheckingRepository));
            if let Some(gate) = &script.gate {
                tokio::select! {
                    permit = gate.acquire() => permit.expect("gate open").forget(),
                    () = request.ctx.cancel.cancelled() => return Err(RemoteError::Cancelled),
                }
            }
            if let Some(metadata) = script.install {
                *request.metadata = metadata;
            }
            if let Some(fail) = script.fail {
                request.metadata.server_info = None;
                return Err(fail());
            }
            self.completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

/// A context over repositories in a temporary store, with a fake checker and a manual clock.
pub struct Fixture {
    pub dir: TempDir,
    pub checker: Arc<FakeChecker>,
    pub clock: Arc<ManualClock>,
    pub ctx: ViewerContext,
}

pub struct FixtureOptions {
    pub repositories: Vec<Repository>,
    pub frequency: CheckRepositoriesFrequency,
    pub configuration: Configuration,
    pub options: ContextOptions,
}

impl Default for FixtureOptions {
    fn default() -> Self {
        Self {
            repositories: Vec::new(),
            frequency: CheckRepositoriesFrequency::Disabled,
            configuration: Configuration::default(),
            options: ContextOptions {
                worker_threads: 2,
                synchronizer: None,
                ..ContextOptions::default()
            },
        }
    }
}

impl Fixture {
    pub fn new(repositories: Vec<Repository>) -> Self {
        Self::with(FixtureOptions {
            repositories,
            ..FixtureOptions::default()
        })
    }

    pub fn with_frequency(
        repositories: Vec<Repository>,
        frequency: CheckRepositoriesFrequency,
    ) -> Self {
        Self::with(FixtureOptions {
            repositories,
            frequency,
            ..FixtureOptions::default()
        })
    }

    pub fn with(options: FixtureOptions) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let checker = Arc::new(FakeChecker::default());
        let clock = ManualClock::new(start_time());
        let preferences = Preferences {
            check_repositories_frequency: options.frequency,
            ..Preferences::default()
        };
        let client = ClientData::from_parts(
            store(dir.path()),
            options.configuration,
            preferences,
            Vec::new(),
            options.repositories,
        );
        let ctx = ViewerContext::new(
            client,
            ContextOptions {
                checker: checker.clone(),
                clock: clock.clone(),
                ..options.options
            },
        )
        .expect("context");
        Self {
            dir,
            checker,
            clock,
            ctx,
        }
    }

    /// The next event; panics after [`EVENT_TIMEOUT`].
    pub fn next(&self) -> AppEvent {
        self.ctx
            .next_event_timeout(EVENT_TIMEOUT)
            .expect("an event within the timeout")
    }

    /// Events until (and including) the first that `stop` accepts.
    pub fn until(&self, stop: impl Fn(&AppEvent) -> bool) -> Vec<AppEvent> {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        let mut events = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = self
                .ctx
                .next_event_timeout(left)
                .unwrap_or_else(|| panic!("no matching event; received {events:#?}"));
            let done = stop(&event);
            events.push(event);
            if done {
                return events;
            }
        }
    }

    /// Events until a task of `kind` finished.
    pub fn until_finished(&self, kind: TaskKind) -> Vec<AppEvent> {
        self.until(|event| finished_kind(event) == Some(kind))
    }

    /// Every event that arrives before a [`QUIET`] pause.
    pub fn quiet(&self) -> Vec<AppEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.ctx.next_event_timeout(QUIET) {
            events.push(event);
        }
        events
    }

    /// Waits until the scheduler sleeps, then moves the clock.
    pub fn advance_after_sleep(&self, duration: Duration) {
        self.ctx.handle().block_on(self.clock.wait_for_sleepers(1));
        self.clock.advance(duration);
    }
}

pub fn started_kind(event: &AppEvent) -> Option<TaskKind> {
    match event {
        AppEvent::Task(TaskEvent::Started(info)) => Some(info.key.kind),
        _ => None,
    }
}

pub fn finished_kind(event: &AppEvent) -> Option<TaskKind> {
    match event {
        AppEvent::Task(TaskEvent::Finished { info, .. }) => Some(info.key.kind),
        _ => None,
    }
}

pub fn outcome_of(events: &[AppEvent], kind: TaskKind) -> TaskOutcome {
    events
        .iter()
        .find_map(|event| match event {
            AppEvent::Task(TaskEvent::Finished { info, outcome }) if info.key.kind == kind => {
                Some(outcome.clone())
            }
            _ => None,
        })
        .expect("a finished task")
}

pub fn count_started(events: &[AppEvent], kind: TaskKind) -> usize {
    events
        .iter()
        .filter(|event| started_kind(event) == Some(kind))
        .count()
}
