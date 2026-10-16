//! Publishing a repository: scan the source folder, build, verify the server, upload, as one
//! cancellable task whose steps report their state; a failed or stopped run is retried from its
//! step.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_admin::build::{STAGE_DELETE_ZIP, STAGE_DELETE_ZSYNC, STAGE_SHA1, STAGE_ZIP, STAGE_ZSYNC};
use a3s_admin::{
    BuildError, BuildEvent, BuildOptions, CheckError, EventsDoc, UploadError, build_repository,
    check_repository_content, upload_repository,
};
use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_repo_files::A3sFile;
use a3s_repo_files::layout::A3S_FOLDER;
use a3s_repo_files::local::CacheKind;
use a3s_sync::{EventError, EventSender, Phase, SyncEvent};
use a3s_transport::proxy::transport_options;
use a3s_types::autoconfig_url::determine_autoconfig_url;
use a3s_types::local::Repository;
use a3s_types::{ServerInfo, SyncTreeDirectory};

use super::status::{RemoteRepository, blocking, built_changes, scan_source, set};
use super::{Event, ModChanges, Remote};
use crate::command::Command;
use crate::error::ContextError;
use crate::event::AppEvent;
use crate::repositories;
use crate::tasks::{Cx, TaskCtx, TaskError, TaskId, TaskKey, TaskKind};

/// A step of a publish, in run order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublishStep {
    /// Compare the source folder with the published repository.
    Scan,
    /// Build the repository in the source folder.
    Build,
    /// Check that the server holds every file of the published repository.
    Verify,
    /// Upload the changed files and the metadata.
    Upload,
}

impl PublishStep {
    /// Every step, in run order.
    pub const ALL: [Self; 4] = [Self::Scan, Self::Build, Self::Verify, Self::Upload];

    /// The position in [`Self::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Where a step stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StepStatus {
    /// Not run.
    #[default]
    Pending,
    /// Running.
    Running,
    /// Finished.
    Done,
    /// Failed; the run can be retried from this step.
    Failed(EventError),
    /// Stopped by the user; the run can be resumed from this step.
    Stopped,
}

/// The stage of a running build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildStage {
    /// Computing SHA-1 checksums.
    Checksums,
    /// Writing `.zsync` files.
    Zsync,
    /// Compressing files.
    Compression,
    /// Deleting outputs that are no longer used.
    Cleanup,
}

/// The stage of a running upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadStage {
    /// Checking that unchanged files are still on the server.
    Checking,
    /// Uploading files.
    Uploading,
    /// Deleting files the repository no longer holds.
    Deleting,
}

/// What a step did or is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StepDetail {
    /// Nothing yet.
    #[default]
    None,
    /// The source folder was compared.
    Scanned {
        /// Files in the repository.
        files: usize,
        /// Files changed since the published revision.
        changed_files: usize,
        /// Bytes of the new and changed files.
        changed_bytes: u64,
    },
    /// The build runs.
    Building {
        /// Its stage, once it hashes.
        stage: Option<BuildStage>,
        /// Progress of the stage.
        percent: Option<u8>,
    },
    /// The build finished.
    Built {
        /// The built revision.
        revision: i32,
        /// Files whose checksum was computed again.
        rehashed: usize,
    },
    /// The server check runs.
    Verifying {
        /// Progress.
        percent: Option<u8>,
    },
    /// The server check finished.
    Verified {
        /// Files checked.
        checked: usize,
        /// Files missing on the server; the upload restores them.
        missing: usize,
    },
    /// The server holds no repository yet.
    NothingPublished,
    /// The upload runs; transfer progress is in the task snapshot.
    Uploading {
        /// Its stage.
        stage: Option<UploadStage>,
        /// Progress of a check or delete stage.
        percent: Option<u8>,
    },
    /// No upload: there is no upload connection, and the server, which serves the source
    /// folder itself, holds every file.
    ServedDirectly,
    /// The upload finished.
    Uploaded {
        /// Files uploaded.
        files: usize,
        /// Their bytes.
        bytes: u64,
        /// Remote files deleted.
        deleted: usize,
    },
}

/// One step of a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepState {
    /// Where it stands.
    pub status: StepStatus,
    /// What it did.
    pub detail: StepDetail,
}

/// The running or last publish of a repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishRun {
    /// One state per [`PublishStep`], in run order.
    pub steps: [StepState; 4],

    /// The revision the build created.
    pub revision: Option<i32>,

    /// What players get: the built repository compared with the published one.
    pub changes: Option<ModChanges>,

    /// The run finished with every step done.
    pub published: bool,
}

impl PublishRun {
    /// The state of `step`.
    pub fn step(&self, step: PublishStep) -> &StepState {
        &self.steps[step.index()]
    }
}

/// Starts publishing `repository` from `from`. The steps before `from` keep their state from
/// the last run; a running status refresh is stopped.
pub fn publish(cx: &Cx, repository: &str, from: PublishStep) -> Result<TaskId, ContextError> {
    if let Some(id) = cx.is_running(&TaskKey::repository(TaskKind::PublishingStatus, repository)) {
        cx.cancel_task(id);
    }
    let name = repository.to_owned();
    cx.spawn_task(
        TaskKey::repository(TaskKind::Publish, repository),
        true,
        move |task| run_publish(task, name, from),
    )
}

/// Stops the running publish of `repository`; `false` when none runs.
pub fn cancel(cx: &Cx, repository: &str) -> bool {
    cx.is_running(&TaskKey::repository(TaskKind::Publish, repository))
        .is_some_and(|id| cx.cancel_task(id))
}

/// How a step ended, when it did not finish.
enum StepEnd {
    Stopped,
    Failed(EventError),
}

async fn run_publish(task: TaskCtx, name: String, from: PublishStep) -> Result<(), TaskError> {
    let cx = task.cx.clone();
    let mut events_at_start: Option<EventsDoc> = None;
    set(&cx, &name, |publishing| {
        let previous = publishing.run.take().unwrap_or_default();
        let mut run = PublishRun::default();
        for step in PublishStep::ALL {
            if step < from {
                run.steps[step.index()] = previous.step(step).clone();
            }
        }
        if from > PublishStep::Build {
            run.revision = previous.revision;
            run.changes = previous.changes;
        }
        publishing.run = Some(run);
        events_at_start.clone_from(&publishing.events);
    });

    for step in PublishStep::ALL.into_iter().filter(|step| *step >= from) {
        update(&cx, &name, step, |state| state.status = StepStatus::Running);
        let ended = match step {
            PublishStep::Scan => scan(&cx, &name).await,
            PublishStep::Build => build(&task, &name).await,
            PublishStep::Verify => verify(&task, &name).await,
            PublishStep::Upload => upload(&task, &name).await,
        };
        match ended {
            Ok(()) => update(&cx, &name, step, |state| state.status = StepStatus::Done),
            Err(StepEnd::Stopped) => {
                update(&cx, &name, step, |state| state.status = StepStatus::Stopped);
                return Err(TaskError::Cancelled);
            }
            Err(StepEnd::Failed(error)) => {
                tracing::warn!("Publishing {name} failed at {step:?}: {}", error.message);
                update(&cx, &name, step, |state| {
                    state.status = StepStatus::Failed(error.clone());
                });
                cx.emit(AppEvent::Admin(Event::PublishFailed {
                    repository: name,
                    step,
                    error,
                }));
                return Ok(());
            }
        }
    }
    finish(&cx, &name, events_at_start).await;
    Ok(())
}

/// The server now holds the local build: it becomes the published repository, the source
/// folder is compared with it again, and the repository is checked like any other.
async fn finish(cx: &Cx, name: &str, events_at_start: Option<EventsDoc>) {
    let Ok(repository) = repository(cx, name) else {
        return;
    };
    let store = Arc::clone(cx.state().read().client.store());
    let read = blocking(move || {
        let a3s =
            std::path::Path::new(repository.path.as_deref().unwrap_or_default()).join(A3S_FOLDER);
        let sync =
            a3s_repo_files::read_a3s::<SyncTreeDirectory>(&a3s.join(A3sFile::Sync.file_name()))
                .ok();
        let server_info =
            a3s_repo_files::read_a3s::<ServerInfo>(&a3s.join(A3sFile::ServerInfo.file_name())).ok();
        let cache = store.read_sha1_cache(&repository.name, CacheKind::Build);
        let scan = scan_source(&repository, sync.as_ref(), &cache);
        (sync, server_info, scan)
    })
    .await;
    let Some((sync, server_info, scan)) = read else {
        return;
    };
    let revision = server_info.as_ref().map_or(0, |info| info.revision);
    set(cx, name, |publishing| {
        publishing.remote = Remote::Loaded(RemoteRepository {
            server_info: server_info.map(Arc::new),
            sync: sync.map(Arc::new),
        });
        publishing.local = Some(scan);
        if publishing.events == events_at_start {
            publishing.events_changed = false;
        }
        if let Some(run) = &mut publishing.run {
            run.published = true;
        }
    });
    tracing::info!("Repository {name} published at revision {revision}.");
    cx.emit(AppEvent::Admin(Event::Published {
        repository: name.to_owned(),
        revision,
    }));
    let check = Command::Repositories(repositories::Command::Check {
        names: vec![name.to_owned()],
    });
    if let Some(commands) = &*cx.shared.commands.lock() {
        commands.send(check).ok();
    }
}

async fn scan(cx: &Cx, name: &str) -> Result<(), StepEnd> {
    let repository = repository(cx, name)?;
    let published = published_sync(cx, name);
    let store = Arc::clone(cx.state().read().client.store());
    let scanned = blocking(move || {
        let cache = store.read_sha1_cache(&repository.name, CacheKind::Build);
        scan_source(&repository, published.as_deref(), &cache)
    })
    .await
    .ok_or(StepEnd::Stopped)?;
    let detail = scanned.as_ref().ok().map(|scan| StepDetail::Scanned {
        files: scan.files,
        changed_files: scan.changed_files,
        changed_bytes: scan.changed_bytes,
    });
    set(cx, name, |publishing| {
        publishing.local = Some(scanned.clone());
        if let (Some(run), Some(detail)) = (&mut publishing.run, detail) {
            run.steps[PublishStep::Scan.index()].detail = detail;
        }
    });
    scanned.map(drop).map_err(StepEnd::Failed)
}

/// Builds, then stores the SHA-1 cache (also after a failure), the new revision and the
/// autoconfig link.
async fn build(task: &TaskCtx, name: &str) -> Result<(), StepEnd> {
    let cx = task.cx.clone();
    let repository = repository(&cx, name)?;
    let published = published_sync(&cx, name);
    let store = Arc::clone(cx.state().read().client.store());
    update(&cx, name, PublishStep::Build, |state| {
        state.detail = StepDetail::Building {
            stage: None,
            percent: None,
        };
    });

    let cancelled = Arc::new(AtomicBool::new(false));
    let watcher = {
        let cancelled = Arc::clone(&cancelled);
        let token = task.cancel.clone();
        cx.handle().spawn(async move {
            token.cancelled().await;
            cancelled.store(true, Ordering::Relaxed);
        })
    };
    let built = {
        let cx = cx.clone();
        let name = name.to_owned();
        blocking(move || {
            let mut cache = store.read_sha1_cache(&repository.name, CacheKind::Build);
            let on_event = |event: BuildEvent| build_progress(&cx, &name, &event);
            let result = build_repository(
                &repository,
                &mut cache,
                &BuildOptions::default(),
                &cancelled,
                &on_event,
            );
            // The hashes computed so far are kept even when the build failed.
            if let Err(err) = store.write_sha1_cache(&repository.name, CacheKind::Build, &cache) {
                tracing::warn!("Cannot save the SHA-1 cache of {}: {err}", repository.name);
            }
            result
        })
        .await
    };
    watcher.abort();
    let outcome = match built {
        None | Some(Err(BuildError::Cancelled)) => return Err(StepEnd::Stopped),
        Some(Err(err)) => {
            return Err(StepEnd::Failed(EventError::new(
                ErrorKind::Io,
                err.to_string(),
            )));
        }
        Some(Ok(outcome)) => outcome,
    };
    let stored = cx.state().write().client.update_repository(name, |stored| {
        outcome.apply_to(stored);
        // Mirrors dao/repository/RepositoryBuildProcessor.java:279-280
        stored.auto_config_url = Some(determine_autoconfig_url(&stored.protocol));
        Ok(true)
    });
    if let Err(err) = stored {
        return Err(StepEnd::Failed(EventError::new(
            err.kind(),
            err.user_message(),
        )));
    }
    let revision = outcome.server_info.revision;
    let changes = built_changes(published.as_deref(), &outcome.sync);
    set(&cx, name, |publishing| {
        if let Some(run) = &mut publishing.run {
            run.revision = Some(revision);
            run.changes = Some(changes);
            run.steps[PublishStep::Build.index()].detail = StepDetail::Built {
                revision,
                rehashed: outcome.rehashed.len(),
            };
        }
    });
    Ok(())
}

fn build_progress(cx: &Cx, name: &str, event: &BuildEvent) {
    match event {
        BuildEvent::Stage(text) => {
            let stage = match *text {
                STAGE_SHA1 => BuildStage::Checksums,
                STAGE_ZSYNC => BuildStage::Zsync,
                STAGE_ZIP => BuildStage::Compression,
                STAGE_DELETE_ZSYNC | STAGE_DELETE_ZIP => BuildStage::Cleanup,
                _ => return,
            };
            update(cx, name, PublishStep::Build, |state| {
                state.detail = StepDetail::Building {
                    stage: Some(stage),
                    percent: None,
                };
            });
        }
        BuildEvent::Progress(percent) => update(cx, name, PublishStep::Build, |state| {
            if let StepDetail::Building {
                percent: current, ..
            } = &mut state.detail
            {
                *current = Some(*percent);
            }
        }),
        BuildEvent::Warning(warning) => tracing::warn!("Building {name}: {warning}"),
    }
}

/// Checks that the server holds every file of the published repository. Missing files are not
/// a failure: the upload restores them.
async fn verify(task: &TaskCtx, name: &str) -> Result<(), StepEnd> {
    let cx = task.cx.clone();
    let repository = repository(&cx, name)?;
    let proxy = cx.state().read().client.proxy().clone();
    let options = transport_options(&repository.protocol, &proxy).map_err(|err| {
        StepEnd::Failed(EventError::new(ErrorKind::InvalidInput, err.to_string()))
    })?;
    update(&cx, name, PublishStep::Verify, |state| {
        state.detail = StepDetail::Verifying { percent: None };
    });
    let events = {
        let cx = cx.clone();
        let name = name.to_owned();
        EventSender::from_fn(move |event| {
            if let SyncEvent::Progress { percent, .. } = event {
                update(&cx, &name, PublishStep::Verify, |state| {
                    state.detail = StepDetail::Verifying {
                        percent: Some(percent),
                    };
                });
            }
        })
    };
    let checked =
        check_repository_content(&repository, &options, &task.transfer_ctx(), &events).await;
    let detail = match checked {
        Ok(report) => StepDetail::Verified {
            checked: report.checked,
            missing: report.missing.len(),
        },
        Err(CheckError::Cancelled) => return Err(StepEnd::Stopped),
        Err(CheckError::Remote(err)) if err.is_not_found() => StepDetail::NothingPublished,
        Err(err) => {
            return Err(StepEnd::Failed(EventError::new(
                err.kind(),
                err.user_message(),
            )));
        }
    };
    update(&cx, name, PublishStep::Verify, |state| {
        state.detail = detail;
    });
    Ok(())
}

/// Uploads over the upload connection; what an interrupted attempt uploaded is skipped.
async fn upload(task: &TaskCtx, name: &str) -> Result<(), StepEnd> {
    let cx = task.cx.clone();
    let repository = repository(&cx, name)?;
    let proxy = cx.state().read().client.proxy().clone();
    let Some(protocol) = repository.upload_protocol.clone() else {
        return served_directly(&cx, name);
    };
    let options = transport_options(&protocol, &proxy).map_err(|err| {
        StepEnd::Failed(EventError::new(ErrorKind::InvalidInput, err.to_string()))
    })?;
    update(&cx, name, PublishStep::Upload, |state| {
        state.detail = StepDetail::Uploading {
            stage: None,
            percent: None,
        };
    });
    let events = {
        let cx = cx.clone();
        let name = name.to_owned();
        let inner = task.sync_events();
        EventSender::from_fn(move |event| {
            match &event {
                SyncEvent::Phase(phase) => {
                    let stage = match phase {
                        Phase::CheckingRemoteFiles => Some(UploadStage::Checking),
                        Phase::Uploading => Some(UploadStage::Uploading),
                        Phase::DeletingRemoteFiles => Some(UploadStage::Deleting),
                        _ => None,
                    };
                    if let Some(stage) = stage {
                        update(&cx, &name, PublishStep::Upload, |state| {
                            state.detail = StepDetail::Uploading {
                                stage: Some(stage),
                                percent: None,
                            };
                        });
                    }
                }
                SyncEvent::Progress { percent, .. } => {
                    update(&cx, &name, PublishStep::Upload, |state| {
                        if let StepDetail::Uploading {
                            percent: current, ..
                        } = &mut state.detail
                        {
                            *current = Some(*percent);
                        }
                    });
                }
                _ => {}
            }
            inner.emit(event);
        })
    };
    let mut resume = cx
        .state()
        .write()
        .admin
        .resume
        .remove(name)
        .unwrap_or_default();
    let uploaded = upload_repository(
        &repository,
        &options,
        &mut resume,
        &task.transfer_ctx(),
        &events,
    )
    .await;
    if !resume.is_empty() {
        cx.state()
            .write()
            .admin
            .resume
            .insert(name.to_owned(), resume);
    }
    match uploaded {
        Ok(report) => {
            update(&cx, name, PublishStep::Upload, |state| {
                state.detail = StepDetail::Uploaded {
                    files: report.uploaded_files,
                    bytes: report.uploaded_bytes,
                    deleted: report.deleted,
                };
            });
            Ok(())
        }
        Err(UploadError::Cancelled) => Err(StepEnd::Stopped),
        Err(err) => Err(StepEnd::Failed(EventError::new(
            err.kind(),
            err.user_message(),
        ))),
    }
}

/// Without an upload connection the build counts as published when the server, which then
/// serves the source folder itself, holds every file of it.
fn served_directly(cx: &Cx, name: &str) -> Result<(), StepEnd> {
    let verified = cx
        .state()
        .read()
        .admin
        .repositories
        .get(name)
        .and_then(|publishing| publishing.run.as_ref())
        .map(|run| run.step(PublishStep::Verify).detail.clone());
    match verified {
        Some(StepDetail::Verified { missing: 0, .. }) => {
            update(cx, name, PublishStep::Upload, |state| {
                state.detail = StepDetail::ServedDirectly;
            });
            Ok(())
        }
        _ => Err(StepEnd::Failed(EventError::new(
            UploadError::UploadProtocolNotSet.kind(),
            "The server does not hold this build and there is no upload connection. Set one \
             in Settings, or serve the source folder at the repository's address.",
        ))),
    }
}

fn repository(cx: &Cx, name: &str) -> Result<Repository, StepEnd> {
    cx.state()
        .read()
        .client
        .repository(name)
        .cloned()
        .map_err(|err| StepEnd::Failed(EventError::new(err.kind(), err.user_message())))
}

fn published_sync(cx: &Cx, name: &str) -> Option<Arc<SyncTreeDirectory>> {
    cx.state()
        .read()
        .admin
        .repositories
        .get(name)
        .and_then(|publishing| publishing.remote.repository())
        .and_then(|remote| remote.sync.clone())
}

/// Changes the state of `step` in the current run and redraws, without an event: progress
/// changes often.
fn update(cx: &Cx, name: &str, step: PublishStep, f: impl FnOnce(&mut StepState)) {
    {
        let mut state = cx.state().write();
        let Some(run) = state
            .admin
            .repositories
            .get_mut(name)
            .and_then(|publishing| publishing.run.as_mut())
        else {
            return;
        };
        f(&mut run.steps[step.index()]);
    }
    cx.shared.events.repaint();
}
