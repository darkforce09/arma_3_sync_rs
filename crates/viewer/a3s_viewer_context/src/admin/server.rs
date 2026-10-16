//! Questions to the server of a repository built on this computer: whether it answers partial
//! (range) requests, which delta updates need, and whether it holds every file of the published
//! repository.

use a3s_admin::{CheckError, check_repository_content};
use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_repo_files::A3sFile;
use a3s_repo_files::layout::A3S_FOLDER;
use a3s_sync::{
    EventError, EventSender, PartialTransferSupport, RemoteConnection, RemoteError, SyncEvent,
};
use a3s_transport::proxy::transport_options;
use a3s_transport::{TransferCtx, Transport};
use a3s_types::Protocol;

use super::status::set;
use crate::error::ContextError;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskId, TaskKey, TaskKind};

/// What the partial transfer test found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeTest {
    /// The test runs.
    Running,
    /// The server answers range requests: delta updates work.
    Supported,
    /// The server ignores range requests: players download whole files.
    Unsupported,
    /// Not an HTTP repository; range requests do not apply.
    NotApplicable,
    /// The server could not be asked.
    Failed(EventError),
}

/// What the check of the server's files found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerCheck {
    /// The check runs, with its progress.
    Running(Option<u8>),
    /// The check finished: files tested, and the paths missing on the server.
    Done {
        /// Files tested.
        checked: usize,
        /// The messages of the missing files.
        missing: Vec<String>,
    },
    /// The server holds no repository yet.
    NothingPublished,
    /// The check failed.
    Failed(EventError),
}

/// Asks the server of `repository` whether it answers range requests, as a
/// [`TaskKind::PartialTransferTest`]; the answer goes to
/// [`super::Publishing::range_test`].
// Mirrors ui/repository/dialogs/progress/ProgressTestPartiaFileTransfer.java:40-94
pub fn test_partial_transfer(cx: &Cx, repository: &str) -> Result<TaskId, ContextError> {
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::PartialTransferTest, repository),
        true,
        move |task| async move {
            set(&task.cx, &name, |publishing| {
                publishing.range_test = Some(RangeTest::Running);
            });
            let result = probe(&task, &name).await;
            let test = match result {
                Ok(PartialTransferSupport::Supported) => RangeTest::Supported,
                Ok(PartialTransferSupport::Unsupported) => RangeTest::Unsupported,
                Ok(PartialTransferSupport::NotApplicable) => RangeTest::NotApplicable,
                Err(error) if error.kind == ErrorKind::Cancelled => {
                    set(&task.cx, &name, |publishing| publishing.range_test = None);
                    return Err(TaskError::Cancelled);
                }
                Err(error) => RangeTest::Failed(error),
            };
            set(&task.cx, &name, |publishing| {
                publishing.range_test = Some(test);
            });
            Ok(())
        },
    )
}

async fn probe(task: &TaskCtx, name: &str) -> Result<PartialTransferSupport, EventError> {
    let (repository, proxy) = {
        let state = task.cx.state().read();
        let repository = state
            .client
            .repository(name)
            .map_err(|err| EventError::new(err.kind(), err.user_message()))?
            .clone();
        (repository, state.client.proxy().clone())
    };
    let options = transport_options(&repository.protocol, &proxy)
        .map_err(|err| EventError::new(ErrorKind::InvalidInput, err.to_string()))?;
    let ctx = task.transfer_ctx();
    let mut connection = RemoteConnection::connect(&repository.protocol, &options, &ctx)
        .await
        .map_err(|err| EventError::from_remote(&err))?;
    let support = connection
        .probe_partial_transfer(&ctx)
        .await
        .map_err(|err| EventError::from_remote(&err));
    connection.close().await;
    support
}

/// Checks that the server of `repository` holds every file of the published repository, as an
/// [`TaskKind::AdminCheck`]; the result goes to [`super::Publishing::server_check`].
// Mirrors ui/repository/workers/RepositoryChecker.java:52-215
pub fn check_server(cx: &Cx, repository: &str) -> Result<TaskId, ContextError> {
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::AdminCheck, repository),
        true,
        move |task| async move {
            set(&task.cx, &name, |publishing| {
                publishing.server_check = Some(ServerCheck::Running(None));
            });
            let check = run_check(&task, &name).await;
            if check.is_none() {
                set(&task.cx, &name, |publishing| publishing.server_check = None);
                return Err(TaskError::Cancelled);
            }
            set(&task.cx, &name, |publishing| {
                publishing.server_check = check;
            });
            Ok(())
        },
    )
}

/// The check's result; `None` once cancelled.
async fn run_check(task: &TaskCtx, name: &str) -> Option<ServerCheck> {
    let cx = task.cx.clone();
    let (repository, proxy) = {
        let state = cx.state().read();
        match state.client.repository(name) {
            Ok(repository) => (repository.clone(), state.client.proxy().clone()),
            Err(err) => {
                return Some(ServerCheck::Failed(EventError::new(
                    err.kind(),
                    err.user_message(),
                )));
            }
        }
    };
    let options = match transport_options(&repository.protocol, &proxy) {
        Ok(options) => options,
        Err(err) => {
            return Some(ServerCheck::Failed(EventError::new(
                ErrorKind::InvalidInput,
                err.to_string(),
            )));
        }
    };
    let events = {
        let cx = cx.clone();
        let name = name.to_owned();
        EventSender::from_fn(move |event| {
            if let SyncEvent::Progress { percent, .. } = event {
                set(&cx, &name, |publishing| {
                    publishing.server_check = Some(ServerCheck::Running(Some(percent)));
                });
            }
        })
    };
    let checked =
        check_repository_content(&repository, &options, &task.transfer_ctx(), &events).await;
    Some(match checked {
        Ok(report) => ServerCheck::Done {
            checked: report.checked,
            missing: report.error_messages(),
        },
        Err(CheckError::Cancelled) => return None,
        Err(CheckError::Remote(err)) if err.is_not_found() => ServerCheck::NothingPublished,
        Err(err) => ServerCheck::Failed(EventError::new(err.kind(), err.user_message())),
    })
}

/// Connects to the upload connection `protocol` with the configured proxy.
///
/// It also looks for a published repository there: `Ok(true)` when it holds one, `Ok(false)`
/// when it is reachable but empty. Runs on the context's runtime.
pub async fn test_upload_connection(
    cx: &Cx,
    protocol: &Protocol,
    ctx: &TransferCtx,
) -> Result<bool, EventError> {
    let proxy = cx.state().read().client.proxy().clone();
    let options = transport_options(protocol, &proxy)
        .map_err(|err| EventError::new(ErrorKind::InvalidInput, err.to_string()))?;
    let failed =
        |err: a3s_transport::Error| EventError::from_remote(&RemoteError::connect(protocol, err));
    let mut transport = Transport::connect(protocol, &options)
        .await
        .map_err(failed)?;
    let published = transport
        .exists(&format!("{A3S_FOLDER}/{}", A3sFile::Sync.file_name()), ctx)
        .await
        .map_err(failed);
    transport.close().await;
    published
}
