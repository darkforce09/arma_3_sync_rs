//! `update`: checks for an Arma3Sync update and installs it.

use a3s_updater::apply::console::{EXIT_FAILURE, run_console_update};
use a3s_updater::apply::source_for_preference;
use a3s_updater::{CancellationToken, DiagnosticLog, UpdateContext, UpdateSession};

use crate::app::App;
use crate::build_info;

/// Runs the console update of this installation; returns the updater's exit code (0 updated,
/// 2 already up to date, 1 failed). The source follows the "prefer GitHub releases"
/// preference, and an `updater.toml` in the configuration folder takes precedence over the
/// installation's.
///
/// Nothing is relaunched: the command ends after the update, and every client-data change was
/// already written when it was made.
// Mirrors console/CommandGeneral.java:567-589, service/UpdaterProcess.java:43-63
pub(crate) fn run(app: &App, dev: bool) -> u8 {
    let prefer_github = app.data.lock().preferences().prefer_github_updates;
    let context = UpdateContext::for_current_process(
        build_info::VERSION,
        dev,
        source_for_preference(prefer_github),
        Some(app.dirs.config_dir().to_path_buf()),
        DiagnosticLog::from_env(),
    );
    let code = match context {
        Ok(context) => {
            let session = UpdateSession::new(context);
            app.runtime.block_on(run_console_update(
                &session,
                &mut std::io::stdout(),
                &mut std::io::stderr(),
                &CancellationToken::new(),
            ))
        }
        Err(err) => {
            say_error!("Cannot locate the running executable: {err}");
            EXIT_FAILURE
        }
    };
    u8::try_from(code).unwrap_or(crate::EXIT_FAILURE)
}
