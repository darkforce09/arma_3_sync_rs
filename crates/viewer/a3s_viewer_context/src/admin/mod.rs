//! Repository administration: the publishing status of a repository built on this computer,
//! the publish run (scan, build, verify, upload), and its events.

mod events;
mod publish;
mod status;

use std::collections::BTreeMap;

use a3s_admin::UploadResume;
use a3s_sync::EventError;

pub use self::events::{edit_events, restore_events, upload_events};
pub use self::publish::{
    BuildStage, PublishRun, PublishStep, StepDetail, StepState, StepStatus, UploadStage, cancel,
    publish,
};
pub use self::status::{
    LocalScan, ModChanges, Publishing, Remote, RemoteRepository, forget, refresh,
};

use crate::error::ContextError;
use crate::tasks::Cx;

/// Repository administration commands.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Command {
    /// Reads the published repository and compares the source folder with it ([`refresh`]).
    Refresh {
        /// The repository.
        repository: String,
        /// Also reads the server, not only the source folder.
        remote: bool,
    },

    /// Publishes the repository, starting at `from` ([`publish`]).
    Publish {
        /// The repository.
        repository: String,
        /// The first step to run; the earlier ones are kept from the last run.
        from: PublishStep,
    },

    /// Stops a running publish ([`cancel`]).
    CancelPublish {
        /// The repository.
        repository: String,
    },

    /// Uploads the events file alone ([`upload_events`]).
    UploadEvents {
        /// The repository.
        repository: String,
    },
}

/// Repository administration events.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// A publish finished: the server holds `revision`.
    Published {
        /// The repository.
        repository: String,
        /// The published revision.
        revision: i32,
    },

    /// A publish stopped at `step` with `error`; it can be retried from that step.
    PublishFailed {
        /// The repository.
        repository: String,
        /// The failed step.
        step: PublishStep,
        /// Why.
        error: EventError,
    },

    /// The events file was uploaded.
    EventsUploaded {
        /// The repository.
        repository: String,
    },
}

/// Repository administration state.
#[derive(Debug, Default)]
pub struct State {
    /// Publishing status by repository name; an entry is created on its first write.
    pub repositories: BTreeMap<String, Publishing>,

    /// What interrupted uploads already uploaded, by repository, for the next attempt.
    resume: BTreeMap<String, UploadResume>,
}

impl State {
    /// The publishing status of `name`, or the defaults if there is none.
    pub fn publishing(&self, name: &str) -> Publishing {
        self.repositories.get(name).cloned().unwrap_or_default()
    }

    /// The publishing status of `name`, created with the defaults if there is none.
    pub fn publishing_mut(&mut self, name: &str) -> &mut Publishing {
        self.repositories.entry(name.to_owned()).or_default()
    }
}

pub(crate) fn handle(cx: &Cx, command: Command) -> Result<(), ContextError> {
    match command {
        Command::Refresh { repository, remote } => refresh(cx, &repository, remote).map(drop),
        Command::Publish { repository, from } => publish(cx, &repository, from).map(drop),
        Command::CancelPublish { repository } => {
            cancel(cx, &repository);
            Ok(())
        }
        Command::UploadEvents { repository } => upload_events(cx, &repository).map(drop),
    }
}
