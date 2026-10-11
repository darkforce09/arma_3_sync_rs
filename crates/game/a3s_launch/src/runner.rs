//! Running a launch plan on its own thread, reporting what happens as [`LaunchEvent`]s, and
//! restarting the game when auto-restart is on.

use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use a3s_platform::process::{ARMA3_EXES, ProcessProbe, is_any_running};

use crate::LaunchError;
use crate::command::{CommandSpec, LaunchMethod, LaunchPlan, command_line, log_line};
use crate::steam::SteamWatch;

/// What a launch event is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchTarget {
    /// The game.
    Arma3,

    /// An external application, by its configured name.
    External { name: String },
}

/// What happened to a launched process.
#[derive(Clone, Debug)]
pub enum LaunchEvent {
    /// The process was started (Java's "end" observer: the UI minimises or closes on it).
    Started {
        target: LaunchTarget,

        /// The command line, `-password=` values hidden.
        command_line: String,
    },

    /// The process exited; `None` when the code is unknown (killed by a signal, or a game
    /// started through Steam).
    Exited {
        target: LaunchTarget,
        code: Option<i32>,
    },

    /// Auto-restart starts the game again; `run` counts the restarts from 1.
    Restarting { run: u32 },

    /// Starting or following the process failed; this launch chain ends.
    Error {
        target: LaunchTarget,
        error: Arc<LaunchError>,
    },
}

/// Receives the launch events, on the launch threads.
pub type EventSink = Arc<dyn Fn(LaunchEvent) + Send + Sync>;

/// Stops auto-restart when the game keeps exiting right after starting (a refused UAC prompt, a
/// `.bat` that returns at once, a crash on start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestartGuard {
    /// A run shorter than this is a fast exit.
    pub min_run: Duration,

    /// This many fast exits in a row stop auto-restart.
    pub max_fast_exits: u32,
}

impl Default for RestartGuard {
    fn default() -> Self {
        Self {
            min_run: Duration::from_secs(10),
            max_fast_exits: 3,
        }
    }
}

/// How a game launch is followed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunOptions {
    /// Start the game again each time it exits.
    pub auto_restart: bool,
    pub guard: RestartGuard,
    pub steam_watch: SteamWatch,
}

/// A running game launch. Dropping it detaches the launch thread; nothing ever kills the game.
#[derive(Debug)]
pub struct LaunchHandle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl LaunchHandle {
    /// Stops further restarts. The running game is left alone.
    pub fn stop_auto_restart(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Whether the launch thread has ended.
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Waits for the launch thread to end.
    pub fn join(self) -> std::thread::Result<()> {
        self.thread.join()
    }
}

/// Runs `plan` on the `a3s-launch-arma3` thread and returns at once.
///
/// A direct or elevated launch sends `Started`, waits for the process and sends `Exited`. A
/// Steam launch sends `Started` once Steam is asked to start the game; with auto-restart it then
/// follows the game in the process list (`probe`) and sends `Exited { code: None }` when it is
/// gone. With auto-restart the plan runs again, after `Restarting`, until
/// [`LaunchHandle::stop_auto_restart`], an error or [`RestartGuard`] stops it.
// Mirrors dao/LauncherDAO.java:193-358
pub fn spawn_plan(
    plan: LaunchPlan,
    options: RunOptions,
    probe: Arc<dyn ProcessProbe>,
    events: EventSink,
) -> Result<LaunchHandle, LaunchError> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("a3s-launch-arma3".to_owned())
        .spawn(move || run_game(&plan, options, probe.as_ref(), &events, &thread_stop))
        .map_err(LaunchError::Thread)?;
    Ok(LaunchHandle { stop, thread })
}

fn run_game(
    plan: &LaunchPlan,
    options: RunOptions,
    probe: &dyn ProcessProbe,
    events: &EventSink,
    stop: &AtomicBool,
) {
    let target = LaunchTarget::Arma3;
    let error = |error: LaunchError| {
        events(LaunchEvent::Error {
            target: LaunchTarget::Arma3,
            error: Arc::new(error),
        });
    };
    let mut fast_exits = 0;
    let mut run = 0;
    loop {
        tracing::info!("{}", log_line(&plan.shown_command));
        let started_at = Instant::now();
        match plan.method {
            LaunchMethod::Direct | LaunchMethod::Elevated => {
                match run_once(&plan.command, &plan.shown_command, &target, events) {
                    Ok(()) => {}
                    Err(err) => return error(err),
                }
            }
            LaunchMethod::Steam => {
                let child = match spawn(&plan.command) {
                    Ok(child) => child,
                    Err(err) => return error(err),
                };
                reap(child);
                events(LaunchEvent::Started {
                    target: target.clone(),
                    command_line: command_line(&plan.shown_command),
                });
                if !options.auto_restart {
                    return;
                }
                match watch_steam_game(probe, options.steam_watch, stop) {
                    SteamGame::Exited => events(LaunchEvent::Exited {
                        target: target.clone(),
                        code: None,
                    }),
                    SteamGame::Stopped => return,
                    SteamGame::NotSeen => {
                        return error(LaunchError::SteamGameNotSeen {
                            secs: options.steam_watch.appear_timeout.as_secs(),
                        });
                    }
                }
            }
        }

        if !options.auto_restart || stop.load(Ordering::SeqCst) {
            return;
        }
        if started_at.elapsed() < options.guard.min_run {
            fast_exits += 1;
        } else {
            fast_exits = 0;
        }
        if fast_exits >= options.guard.max_fast_exits {
            return error(LaunchError::RestartLoop {
                secs: options.guard.min_run.as_secs(),
                count: fast_exits,
            });
        }
        run += 1;
        events(LaunchEvent::Restarting { run });
    }
}

/// Starts `command`, sends `Started`, waits for it and sends `Exited`.
pub(crate) fn run_once(
    command: &CommandSpec,
    shown_command: &[String],
    target: &LaunchTarget,
    events: &EventSink,
) -> Result<(), LaunchError> {
    let mut child = spawn(command)?;
    events(LaunchEvent::Started {
        target: target.clone(),
        command_line: command_line(shown_command),
    });
    let status = child.wait().map_err(|source| LaunchError::Wait {
        program: command.program_text(),
        source,
    })?;
    events(LaunchEvent::Exited {
        target: target.clone(),
        code: status.code(),
    });
    Ok(())
}

fn spawn(command: &CommandSpec) -> Result<Child, LaunchError> {
    command
        .to_command()
        .spawn()
        .map_err(|source| LaunchError::Spawn {
            program: command.program_text(),
            source,
        })
}

/// Waits for the Steam process on its own thread, so it never lingers as a zombie. Steam may
/// stay running as the Steam client long after the game exits.
fn reap(mut child: Child) {
    let spawned = std::thread::Builder::new()
        .name("a3s-launch-steam-reaper".to_owned())
        .spawn(move || {
            if let Err(err) = child.wait() {
                tracing::debug!("waiting for steam failed: {err}");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("cannot start the thread waiting for steam: {err}");
    }
}

enum SteamGame {
    Exited,
    Stopped,
    NotSeen,
}

/// Polls the process list until the game has appeared and gone again.
fn watch_steam_game(probe: &dyn ProcessProbe, watch: SteamWatch, stop: &AtomicBool) -> SteamGame {
    let deadline = Instant::now() + watch.appear_timeout;
    loop {
        if stop.load(Ordering::SeqCst) {
            return SteamGame::Stopped;
        }
        if is_any_running(probe, &ARMA3_EXES) {
            break;
        }
        if Instant::now() >= deadline {
            return SteamGame::NotSeen;
        }
        std::thread::sleep(watch.poll);
    }
    loop {
        std::thread::sleep(watch.poll);
        if stop.load(Ordering::SeqCst) {
            return SteamGame::Stopped;
        }
        if !is_any_running(probe, &ARMA3_EXES) {
            return SteamGame::Exited;
        }
    }
}
