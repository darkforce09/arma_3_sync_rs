//! `build`: builds a repository from its main folder.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use a3s_admin::{BuildEvent, BuildOptions, build_repository};
use a3s_foundation::error::{ErrorKind, UserFacing as _};
use a3s_repo_files::local::CacheKind;
use a3s_types::autoconfig_url::determine_autoconfig_url;
use parking_lot::Mutex;

use super::{PercentLine, report_repository_error};
use crate::app::{App, CommandResult, Failed};

/// Builds the repository `name`, then stores its new revision and autoconfig URL and the SHA-1
/// cache of its files.
// Mirrors console/CommandGeneral.java:38-123, service/administration/RepositoryBuildProcessor.java:35-50
pub(crate) fn run(app: &App, name: &str) -> CommandResult {
    let repository = super::repository(app, name)?;
    let store = Arc::clone(app.data.lock().store());
    say!("Building repository: {name}");

    let stage = Mutex::new((String::new(), PercentLine::default()));
    let on_event = |event: BuildEvent| {
        let mut stage = stage.lock();
        match event {
            BuildEvent::Stage(text) => {
                say!("{text}");
                stage.0 = text.trim_end_matches("...").to_owned();
                stage.1.reset();
            }
            BuildEvent::Progress(percent) => {
                let (label, line) = &mut *stage;
                line.update(label, percent);
            }
            BuildEvent::Warning(warning) => say_error!("{warning}"),
        }
    };
    let mut cache = store.read_sha1_cache(name, CacheKind::Build);
    let result = build_repository(
        &repository,
        &mut cache,
        &BuildOptions::default(),
        &AtomicBool::new(false),
        &on_event,
    );
    // The hashes computed so far are kept even when the build failed.
    if let Err(err) = store.write_sha1_cache(name, CacheKind::Build, &cache) {
        tracing::warn!("Cannot save the SHA-1 cache of {name}: {err}");
    }

    let saved = result
        .map_err(|err| (ErrorKind::Io, err.to_string()))
        .and_then(|outcome| {
            app.data
                .lock()
                .update_repository(name, |stored| {
                    outcome.apply_to(stored);
                    // Mirrors dao/repository/RepositoryBuildProcessor.java:279-280
                    stored.auto_config_url = Some(determine_autoconfig_url(&stored.protocol));
                    Ok(true)
                })
                .map_err(|err| (err.kind(), err.user_message()))
        });
    match saved {
        Ok(_) => {
            say!("Repository {name} - build finished.");
            Ok(())
        }
        Err((kind, message)) => {
            say_error!("Repository {name} - build finished with error.");
            report_repository_error(name, kind, &message);
            Err(Failed)
        }
    }
}
