//! Editing a repository's events: every change is written to the source folder's
//! `.a3s/events` at once (the next build carries it) and uploaded by the next publish, or alone
//! with [`upload_events`].

#![allow(
    clippy::result_large_err,
    reason = "the events errors carry transport errors; they are returned once per edit"
)]

use a3s_admin::{EventsDoc, EventsError};
use a3s_foundation::error::UserFacing as _;
use a3s_sync::EventError;
use a3s_transport::proxy::transport_options;

use super::Event;
use crate::error::ContextError;
use crate::event::AppEvent;
use crate::state::AppState;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskId, TaskKey, TaskKind};

/// Changes the events of `repository` with `f` and writes them to its source folder; on a
/// failed write nothing changes.
pub fn edit_events<R>(
    state: &mut AppState,
    repository: &str,
    f: impl FnOnce(&mut EventsDoc) -> R,
) -> Result<R, EventsError> {
    let path = state
        .client
        .repository(repository)
        .ok()
        .and_then(|stored| stored.path.clone());
    let publishing = state.admin.publishing_mut(repository);
    let mut doc = publishing.events.clone().unwrap_or_default();
    let result = f(&mut doc);
    doc.write_local(path.as_deref())?;
    publishing.events = Some(doc);
    publishing.events_changed = true;
    Ok(result)
}

/// Puts back `doc` as the events of `repository`, e.g. to undo a removal.
pub fn restore_events(
    state: &mut AppState,
    repository: &str,
    doc: EventsDoc,
) -> Result<(), EventsError> {
    edit_events(state, repository, |current| *current = doc)
}

/// Uploads the events of `repository` over its upload connection.
pub fn upload_events(cx: &Cx, repository: &str) -> Result<TaskId, ContextError> {
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::UploadEvents, repository),
        true,
        move |task| run_upload(task, name),
    )
}

async fn run_upload(task: TaskCtx, name: String) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let (doc, repository, proxy) = {
        let state = cx.state().read();
        let repository = state.client.repository(&name)?.clone();
        let doc = state.admin.publishing(&name).events.unwrap_or_default();
        (doc, repository, state.client.proxy().clone())
    };
    let protocol = repository
        .upload_protocol
        .as_ref()
        .ok_or_else(|| message(&EventsError::UploadProtocolNotSet))?;
    let options = transport_options(protocol, &proxy).map_err(|err| {
        TaskError::Message(EventError::new(
            a3s_foundation::error::ErrorKind::InvalidInput,
            err.to_string(),
        ))
    })?;
    a3s_admin::upload_events(&doc, &repository, &options, &task.transfer_ctx())
        .await
        .map_err(|err| message(&err))?;
    {
        let mut state = cx.state().write();
        let publishing = state.admin.publishing_mut(&name);
        if publishing.events.as_ref() == Some(&doc) {
            publishing.events_changed = false;
        }
    }
    cx.emit(AppEvent::Admin(Event::EventsUploaded { repository: name }));
    Ok(())
}

fn message(err: &EventsError) -> TaskError {
    TaskError::Message(EventError::new(err.kind(), err.user_message()))
}
