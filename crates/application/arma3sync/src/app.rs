//! Start-up of a command or the window: directories, logging, update leftovers and client
//! data.

use std::sync::Arc;

use a3s_foundation::error::UserFacing as _;
use a3s_foundation::log::{self, LogConfig};
use a3s_foundation::paths::{AppDirs, PlatformEnv};
use a3s_repo_files::local::LocalStore;
use a3s_repo_files::services::ClientData;
use parking_lot::Mutex;

use a3s_platform::single_instance::MSG_ALREADY_RUNNING;
use a3s_viewer::{Exit, RunOptions};

use crate::cli::{Command, WindowMode};
use crate::{EXIT_FAILURE, EXIT_SUCCESS, build_info, commands, console};

/// Environment variable that enables the diagnostic log when it equals `true` (ignoring case).
pub(crate) const DEBUG_VAR: &str = "A3S_DEBUG";

/// What every command works with.
pub(crate) struct App {
    pub dirs: AppDirs,

    /// The client data; every change is written to the local store at once.
    pub data: Arc<Mutex<ClientData>>,

    /// Runs the network operations.
    pub runtime: tokio::runtime::Runtime,
}

/// A command ended unsuccessfully; what went wrong was already printed.
#[derive(Debug)]
pub(crate) struct Failed;

/// Result of a command.
pub(crate) type CommandResult = Result<(), Failed>;

/// Sets up the application and runs `command`; returns the process exit code.
// Mirrors main/ArmA3Sync.java:36-61,105-116
pub(crate) fn run(debug_flag: bool, command: Command) -> u8 {
    let Some((dirs, _log)) = start(debug_flag) else {
        return EXIT_FAILURE;
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("a3s-runtime")
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            say_error!("Cannot start the async runtime: {err}");
            return EXIT_FAILURE;
        }
    };
    let store = Arc::new(LocalStore::new(&dirs));
    // Load warnings are logged by `load` (stderr shows warnings).
    let data = ClientData::load(store).data;
    let app = App {
        dirs,
        data: Arc::new(Mutex::new(data)),
        runtime,
    };
    dispatch(&app, command)
}

/// Opens the window; returns the process exit code.
// Mirrors main/ArmA3Sync.java:194-265
pub(crate) fn run_window(debug_flag: bool, window: WindowMode) -> u8 {
    if !a3s_viewer::display_available() {
        say_error!("Can't start Arma3Sync. GUI is missing.");
        return EXIT_FAILURE;
    }
    let Some((dirs, _log)) = start(debug_flag) else {
        return EXIT_FAILURE;
    };
    let options = RunOptions {
        dirs,
        dev_mode: window.dev,
        run_mode: window.run,
        single_instance: window.single_instance,
    };
    match a3s_viewer::run(&options) {
        Ok(Exit::Closed) => EXIT_SUCCESS,
        Ok(Exit::AlreadyRunning) => {
            say_error!("{MSG_ALREADY_RUNNING}");
            EXIT_FAILURE
        }
        Err(err) => {
            tracing::error!("{err}");
            say_error!("{err}");
            EXIT_FAILURE
        }
    }
}

/// The start-up shared by every mode: directories, logging (the diagnostic log file with
/// `debug_flag` or `A3S_DEBUG=true`), the version lines and the update leftovers. `None` when
/// the directories cannot be created; the reason was printed.
fn start(debug_flag: bool) -> Option<(AppDirs, Option<log::LogHandle>)> {
    let debug = debug_flag
        || std::env::var(DEBUG_VAR).is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    let dirs = match resolve_dirs() {
        Ok(dirs) => dirs,
        Err(message) => {
            init_logging(debug, None);
            say_error!("{message}");
            return None;
        }
    };
    let log = init_logging(debug, Some(&dirs));
    tracing::info!("{}", build_info::installed_version_line());
    tracing::info!(
        "Build {} ({}, {})",
        build_info::GIT_HASH,
        build_info::TARGET,
        build_info::PROFILE
    );
    clean_update_leftovers();
    Some((dirs, log))
}

fn dispatch(app: &App, command: Command) -> u8 {
    let result = match command {
        Command::Build { repository } => commands::build::run(app, &repository),
        Command::Check { repository } => commands::check::run(app, &repository),
        Command::Sync {
            repository,
            destination,
            exact_match,
        } => commands::sync::run(app, &repository, &destination, exact_match),
        Command::Extract { source, target } => commands::extract::run(&source, &target),
        Command::Update { dev } => return commands::update::run(app, dev),
        Command::Console { dev } => console::run(app, dev),
    };
    match result {
        Ok(()) => EXIT_SUCCESS,
        Err(Failed) => EXIT_FAILURE,
    }
}

/// The application directories, created.
fn resolve_dirs() -> Result<AppDirs, String> {
    let env = PlatformEnv::current().map_err(|err| err.user_message())?;
    let dirs = AppDirs::resolve(&env).map_err(|err| err.user_message())?;
    dirs.ensure_created().map_err(|err| err.user_message())?;
    Ok(dirs)
}

/// Starts logging: warnings to stderr (`A3S_LOG` overrides), and with `debug` the rotating file
/// in the log folder.
fn init_logging(debug: bool, dirs: Option<&AppDirs>) -> Option<log::LogHandle> {
    let config = LogConfig {
        debug,
        log_dir: dirs.map(AppDirs::log_dir),
        stderr: true,
    };
    log::init(&config).ok()
}

/// Repairs and removes what interrupted self-updates left next to the executable. Leftovers of
/// a process that still runs (an update in progress) are left alone, so this is safe without
/// the single-instance lock.
fn clean_update_leftovers() {
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    if let Some(install_dir) = install_dir {
        let removed = a3s_updater::apply::replace::cleanup_leftovers(&install_dir);
        if removed > 0 {
            tracing::info!(
                "Removed {removed} update leftovers from {}",
                install_dir.display()
            );
        }
    }
}
