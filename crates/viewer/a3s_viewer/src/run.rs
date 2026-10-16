//! Starting the window: single-instance lock, client data, context, tray and eframe.

use std::sync::Arc;

use a3s_foundation::paths::legacy::discover_legacy_java;
use a3s_foundation::paths::{AppDirs, Os, PlatformEnv};
use a3s_platform::detect::{detect_arma3_dir, java_install_dirs};
use a3s_platform::registry::SystemRegistry;
use a3s_platform::single_instance::{self, Acquire, AcquireOptions, InstanceId, InstanceMessage};
use a3s_repo_files::legacy::should_offer_import;
use a3s_repo_files::local::LocalStore;
use a3s_repo_files::services::{ClientData, LoadOutcome};
use a3s_updater::apply::relaunch::RelaunchPlan;
use a3s_viewer_context::{ContextError, ContextOptions, RepaintHook, ViewerContext};
use parking_lot::Mutex;

use crate::desktop::Desktop;
use crate::requests::{self, RequestSender, WindowRequest};
use crate::sheets::WelcomeSetup;
use crate::shell::{Shell, ShellSetup};
use crate::tray::Tray;
use crate::windows::Appearance;

/// The window title and the name eframe knows the application by.
const APP_NAME: &str = "Arma3Sync";

/// The Wayland app id and X11 class, matching the desktop entry's name.
const APP_ID: &str = "arma3sync";

/// The window size when none was saved.
const DEFAULT_SIZE: [f32; 2] = [1100.0, 720.0];

/// The smallest window size.
const MIN_SIZE: [f32; 2] = [760.0, 480.0];

/// How the window is started.
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// The application directories, created.
    pub dirs: AppDirs,

    /// Development mode (`-dev`).
    pub dev_mode: bool,

    /// Started with the operating system (`-run`): the window starts hidden in the tray.
    pub run_mode: bool,

    /// Take the single-instance lock (every mode but `-unlock`).
    pub single_instance: bool,
}

/// How [`run`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The window was closed.
    Closed,

    /// Another instance runs; it was asked to show its window.
    AlreadyRunning,
}

/// Why the window could not run.
#[derive(Debug, thiserror::Error)]
pub enum ViewerError {
    /// The background runtime could not start.
    #[error("Cannot start the background work: {0}")]
    Context(#[from] ContextError),

    /// The window could not be opened.
    #[error("Cannot open the window: {0}")]
    Window(#[from] eframe::Error),
}

/// Whether a graphical session is available: on Linux a Wayland or X11 display.
// Mirrors main/ArmA3Sync.java:202-207
pub fn display_available() -> bool {
    if cfg!(target_os = "linux") {
        ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|var| std::env::var_os(var).is_some_and(|value| !value.is_empty()))
    } else {
        true
    }
}

/// Runs the window until it closes.
///
/// A second start with the single-instance lock asks the first one to show its window and
/// returns [`Exit::AlreadyRunning`] without touching the client data.
// Mirrors main/ArmA3Sync.java:194-265
pub fn run(options: &RunOptions) -> Result<Exit, ViewerError> {
    tracing::info!(
        "DevMode = {}, RunMode = {}, SingleInstanceMode = {}",
        options.dev_mode,
        options.run_mode,
        options.single_instance
    );
    let ctx = egui::Context::default();
    let (requests, receiver) = requests::channel(&ctx);
    let instance = if options.single_instance {
        match lock_instance(&options.dirs, &requests) {
            Acquire::Primary(guard) => Some(guard),
            Acquire::Secondary => return Ok(Exit::AlreadyRunning),
            Acquire::Unlocked { .. } => None,
        }
    } else {
        None
    };

    let store = Arc::new(LocalStore::new(&options.dirs));
    let welcome = welcome_setup(&store);
    let log_dir = options.dirs.log_dir();
    let LoadOutcome { data, warnings } = ClientData::load(store);
    let size = saved_size(&data);
    let appearance = Appearance::from_look(data.preferences().look_and_feel);
    let repaint_ctx = ctx.clone();
    let viewer = ViewerContext::new(
        data,
        ContextOptions {
            repaint: RepaintHook::from_fn(move || repaint_ctx.request_repaint()),
            dev_mode: options.dev_mode,
            run_mode: options.run_mode,
            ..ContextOptions::default()
        },
    )?;
    viewer.start_background();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id(APP_ID)
        .with_inner_size(size)
        .with_min_inner_size(MIN_SIZE)
        .with_visible(!options.run_mode);
    match eframe::icon_data::from_png_bytes(a3s_ui::icon::APP_ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(Arc::new(icon)),
        Err(err) => tracing::warn!("Cannot decode the application icon: {err}"),
    }
    let native = eframe::NativeOptions {
        viewport,
        ..eframe::NativeOptions::default()
    };
    let relaunch: Arc<Mutex<Option<RelaunchPlan>>> = Arc::default();
    let slot = Arc::clone(&relaunch);
    let setup = ShellSetup {
        load_warnings: warnings,
        welcome,
        requests: receiver,
        hidden: options.run_mode,
    };
    eframe::run_native_ext(
        APP_NAME,
        native,
        Some(ctx),
        Box::new(move |creation| {
            let ctx = &creation.egui_ctx;
            a3s_ui::install(ctx);
            ctx.set_theme(appearance.theme());
            let tray = Tray::spawn(&requests);
            let mut shell = Shell::new(viewer, setup);
            shell.set_desktop(Desktop::system(Some(log_dir)));
            shell.set_relaunch_slot(slot);
            shell.set_tray_available(ctx, tray.is_some());
            Ok(Box::new(ViewerApp { shell, _tray: tray }))
        }),
    )?;
    // The new version takes the single-instance lock, so it is released first.
    drop(instance);
    if let Some(plan) = relaunch.lock().take()
        && let Err(err) = a3s_updater::apply::relaunch::relaunch(&plan, || {})
    {
        tracing::error!("Cannot start the updated Arma3Sync: {err}");
    }
    Ok(Exit::Closed)
}

/// Takes the lock of this configuration folder; a later instance's request shows the window.
fn lock_instance(dirs: &AppDirs, requests: &RequestSender) -> Acquire {
    let id = InstanceId::for_config_dir(dirs.config_dir(), Os::current());
    let options = AcquireOptions {
        socket_dir: Some(dirs.cache_dir().to_path_buf()),
        ..AcquireOptions::default()
    };
    let requests = requests.clone();
    single_instance::acquire(&id, &options, move |message| {
        if message == InstanceMessage::Activate {
            requests.send(WindowRequest::Show);
        }
    })
}

/// The Welcome sheet's start: shown on the first start and while Java data waits for its
/// import. Read before the client data is loaded, which may write the configuration.
fn welcome_setup(store: &LocalStore) -> Option<WelcomeSetup> {
    let first_run = !store.configuration_path().exists();
    let env = PlatformEnv::current()
        .map_err(|err| tracing::warn!("Cannot read the environment: {err}"))
        .ok()?;
    let registry = SystemRegistry;
    let sources = discover_legacy_java(&env, &java_install_dirs(&env, &registry));
    let offer_import = should_offer_import(store, &sources);
    (first_run || offer_import).then(|| WelcomeSetup {
        arma3_dir: detect_arma3_dir(&env, &registry),
        legacy_sources: if offer_import { sources } else { Vec::new() },
    })
}

/// The window size saved on the last exit, or the default.
fn saved_size(data: &ClientData) -> [f32; 2] {
    let configuration = data.configuration();
    if configuration.width > 0 && configuration.height > 0 {
        [configuration.width as f32, configuration.height as f32]
    } else {
        DEFAULT_SIZE
    }
}

/// The eframe application: the shell and the tray icon it outlives.
struct ViewerApp {
    shell: Shell,
    _tray: Option<Tray>,
}

impl eframe::App for ViewerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shell.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shell.show(ui);
    }
}
