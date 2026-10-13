use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_repo_files::services::ServiceError;
use a3s_sync::remote::check_repository;
use a3s_sync::{EventError, EventSender, RemoteError, RemoteMetadata, status};
use a3s_transport::proxy::{ProxyError, transport_options};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::Protocol;
use a3s_types::constants::RepositoryStatus;
use futures::future::BoxFuture;

use crate::event::{AppEvent, Change};
use crate::repositories;
use crate::tasks::{Cx, TaskCtx};

/// Fetches the remote metadata of one repository: the network seam of every check.
pub trait RemoteChecker: Send + Sync + 'static {
    /// Updates `request.metadata` in place, with the semantics of
    /// [`a3s_sync::remote::check_repository`]: a fetched file replaces its field, the first
    /// failure clears its field and is returned.
    fn check<'a>(&'a self, request: RemoteCheck<'a>) -> BoxFuture<'a, Result<(), RemoteError>>;
}

/// One repository check, as passed to a [`RemoteChecker`].
#[derive(Debug)]
pub struct RemoteCheck<'a> {
    /// The repository name (for logs).
    pub repository_name: &'a str,

    /// Its download protocol.
    pub protocol: &'a Protocol,

    /// Connection options, with the configured proxy.
    pub options: &'a TransportOptions,

    /// The metadata of the previous check, updated in place.
    pub metadata: &'a mut RemoteMetadata,

    /// Cancellation and progress of the transfers.
    pub ctx: &'a TransferCtx,

    /// Where check events go.
    pub events: &'a EventSender,
}

/// The [`RemoteChecker`] that connects to the repository.
#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkChecker;

impl RemoteChecker for NetworkChecker {
    fn check<'a>(&'a self, request: RemoteCheck<'a>) -> BoxFuture<'a, Result<(), RemoteError>> {
        Box::pin(check_repository(
            request.repository_name,
            request.protocol,
            request.options,
            request.metadata,
            request.ctx,
            request.events,
        ))
    }
}

/// What started a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckTrigger {
    /// The periodic check.
    Periodic,

    /// The user.
    Manual,
}

/// How the check of one repository ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The metadata was fetched.
    Ok,

    /// The check failed.
    Failed(EventError),

    /// The check was cancelled; the previous metadata is kept.
    Cancelled,
}

/// The result of one repository check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryCheckResult {
    /// The repository.
    pub name: String,

    /// How the check ended.
    pub outcome: CheckOutcome,

    /// The status computed from the new metadata; `None` when the check was cancelled or the
    /// repository was removed meanwhile.
    pub status: Option<RepositoryStatus>,
}

/// A repository as it was when a check run started.
pub(crate) struct CheckTarget {
    pub(crate) name: String,
    protocol: Protocol,
    options: Result<TransportOptions, ProxyError>,
    pub(crate) notify: bool,
    pub(crate) auto: bool,
}

/// Checks the repositories named in `names` (all with `None`), concurrently, and returns the
/// targets and one result per target, both in name order.
///
/// A repository is checked by one run at a time, and at most
/// [`a3s_sync::remote::MAX_CONCURRENT_REPOSITORY_CHECKS`] checks run at once in the process.
/// The new metadata and check error are written to the repository's runtime state, except
/// after a cancel.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:41-76,
// ui/repository/dialogs/progress/ProgressSynchronizationDialog.java:96-135
pub(crate) async fn check_many(
    cx: &Cx,
    names: Option<&[String]>,
    trigger: CheckTrigger,
    task: &TaskCtx,
    events: &EventSender,
) -> (Vec<CheckTarget>, Vec<RepositoryCheckResult>) {
    let targets: Vec<CheckTarget> = {
        let state = cx.state().read();
        let client = &state.client;
        for name in names.unwrap_or_default() {
            if client.repository(name).is_err() {
                tracing::warn!("Repository {name} not found: it is not checked.");
            }
        }
        client
            .repositories()
            .filter(|r| names.is_none_or(|names| names.contains(&r.name)))
            .map(|r| CheckTarget {
                name: r.name.clone(),
                protocol: r.protocol.clone(),
                options: transport_options(&r.protocol, client.proxy()),
                notify: r.notify,
                auto: r.auto,
            })
            .collect()
    };
    let checks: Vec<_> = targets
        .iter()
        .map(|target| check_one(cx, target, trigger, task, events))
        .collect();
    let results = futures::future::join_all(checks).await;
    (targets, results)
}

async fn check_one(
    cx: &Cx,
    target: &CheckTarget,
    trigger: CheckTrigger,
    task: &TaskCtx,
    events: &EventSender,
) -> RepositoryCheckResult {
    let outcome = match &target.options {
        Ok(options) => run_check(cx, target, options, task, events).await,
        Err(err) => {
            // Like a failed connection: the server info is gone and nothing is attempted.
            let error = EventError::new(ErrorKind::InvalidInput, err.to_string());
            update_runtime(cx, &target.name, |runtime| {
                runtime.metadata.server_info = None;
                runtime.last_check_error = Some(error.clone());
            });
            CheckOutcome::Failed(error)
        }
    };
    if let CheckOutcome::Failed(error) = &outcome {
        tracing::info!("{}", failure_line(trigger, &target.name, &error.message));
    }
    RepositoryCheckResult {
        name: target.name.clone(),
        outcome,
        status: None,
    }
}

async fn run_check(
    cx: &Cx,
    target: &CheckTarget,
    options: &TransportOptions,
    task: &TaskCtx,
    events: &EventSender,
) -> CheckOutcome {
    let lock = cx.repository_lock(&target.name);
    let _repository = tokio::select! {
        biased;
        () = task.cancel.cancelled() => return CheckOutcome::Cancelled,
        guard = lock.lock() => guard,
    };
    let _permit = tokio::select! {
        biased;
        () = task.cancel.cancelled() => return CheckOutcome::Cancelled,
        permit = cx.shared.check_permits.acquire() => match permit {
            Ok(permit) => permit,
            Err(_closed) => return CheckOutcome::Cancelled,
        },
    };

    let mut metadata = RemoteMetadata::default();
    update_runtime(cx, &target.name, |runtime| {
        runtime.checking = true;
        metadata = runtime.metadata.clone();
    });
    let ctx = task.transfer_ctx();
    let result = tokio::select! {
        biased;
        () = task.cancel.cancelled() => Err(RemoteError::Cancelled),
        result = cx.shared.checker.check(RemoteCheck {
            repository_name: &target.name,
            protocol: &target.protocol,
            options,
            metadata: &mut metadata,
            ctx: &ctx,
            events,
        }) => result,
    };
    let outcome = match &result {
        Ok(()) => CheckOutcome::Ok,
        Err(RemoteError::Cancelled) => CheckOutcome::Cancelled,
        Err(err) => CheckOutcome::Failed(EventError::from_remote(err)),
    };
    update_runtime(cx, &target.name, |runtime| {
        runtime.checking = false;
        match &outcome {
            CheckOutcome::Cancelled => {}
            CheckOutcome::Ok => {
                runtime.metadata = metadata;
                runtime.last_check_error = None;
            }
            CheckOutcome::Failed(error) => {
                runtime.metadata = metadata;
                runtime.last_check_error = Some(error.clone());
            }
        }
    });
    outcome
}

/// Runs `f` on the runtime state of `name` if the repository still exists, then requests a
/// repaint so the repository's badge follows.
fn update_runtime(cx: &Cx, name: &str, f: impl FnOnce(&mut crate::RepositoryRuntime)) {
    {
        let mut state = cx.state().write();
        if state.client.repository(name).is_err() {
            return;
        }
        f(state.repositories.runtime_mut(name));
    }
    cx.shared.events.repaint();
}

/// The log line of a failed check: the periodic check breaks the line, the manual one does not.
// Mirrors ui/main/tasks/TaskCheckRepositories.java:57-58,
// ui/repository/dialogs/progress/ProgressSynchronizationDialog.java:111-115
fn failure_line(trigger: CheckTrigger, name: &str, message: &str) -> String {
    match trigger {
        CheckTrigger::Periodic => format!("Error when checking repository {name}:\n{message}"),
        CheckTrigger::Manual => format!("Error when checking repository {name}: {message}"),
    }
}

/// Applies the results of a check run in order: merges each repository's new metadata into its
/// settings and the configuration (persisted at once) and sets its status. Cancelled checks and
/// removed repositories are skipped. Then emits [`Change::Repository`] and
/// [`repositories::Event::CheckCompleted`].
// Mirrors ui/main/tasks/TaskCheckRepositories.java:78-83,
// ui/repository/dialogs/progress/ProgressSynchronizationDialog.java:139-147
pub(crate) fn apply_results(cx: &Cx, trigger: CheckTrigger, results: &mut [RepositoryCheckResult]) {
    let mut failures: Vec<(String, ServiceError)> = Vec::new();
    {
        let mut guard = cx.state().write();
        let state = &mut *guard;
        for result in results.iter_mut() {
            if result.outcome == CheckOutcome::Cancelled
                || state.client.repository(&result.name).is_err()
            {
                continue;
            }
            let metadata = state.repositories.runtime(&result.name).metadata;
            let mut configuration = state.client.configuration().clone();
            let mut update = None;
            let saved = state.client.update_repository(&result.name, |repository| {
                let applied = status::update_repository(repository, &mut configuration, &metadata);
                update = Some(applied);
                Ok(applied.repository_changed)
            });
            if let Err(err) = saved {
                failures.push((format!("Saving repository {}", result.name), err));
            }
            let Some(update) = update else {
                continue;
            };
            if update.configuration_changed
                && let Err(err) = state.client.update_configuration(|stored| {
                    *stored = configuration;
                    true
                })
            {
                failures.push(("Saving configuration".to_owned(), err));
            }
            state.repositories.runtime_mut(&result.name).status = update.status;
            result.status = Some(update.status);
        }
    }
    for (context, err) in failures {
        tracing::warn!("{context}: {}", err.user_message());
        cx.emit(AppEvent::Error {
            context,
            error: EventError::new(err.kind(), err.user_message()),
        });
    }
    cx.emit(AppEvent::Changed(Change::Repository));
    cx.emit(AppEvent::Repositories(
        repositories::Event::CheckCompleted {
            trigger,
            results: results.to_vec(),
        },
    ));
}
