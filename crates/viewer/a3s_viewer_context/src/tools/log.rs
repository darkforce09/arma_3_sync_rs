//! The game log: the newest (or a chosen) Arma 3 RPT file, followed while it grows.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use a3s_repo_files::services::ServiceError;
use a3s_tools::rpt::{DEFAULT_POLL_INTERVAL, RptEvent, RptTail};

use super::blocking;
use crate::error::ContextError;
use crate::state::AppState;
use crate::tasks::{Cx, TaskError, TaskId, TaskKey, TaskKind};

/// How many lines the log keeps; older lines are dropped first.
pub const MAX_LOG_LINES: usize = 100_000;

/// How often the folder is listed for a newer log.
const LIST_INTERVAL: Duration = Duration::from_secs(1);

/// What a log line reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Anything else.
    Info,

    /// A warning.
    Warning,

    /// An error, such as a script error.
    Error,
}

/// The kind of an RPT line: an error when it mentions an error, else a warning when it mentions
/// a warning (ignoring case).
pub fn line_kind(line: &str) -> LineKind {
    let lower = line.to_lowercase();
    if lower.contains("error") {
        LineKind::Error
    } else if lower.contains("warning") {
        LineKind::Warning
    } else {
        LineKind::Info
    }
}

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The text, without its terminator.
    pub text: String,

    /// What it reports.
    pub kind: LineKind,
}

impl LogLine {
    fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            kind: line_kind(&text),
            text,
        }
    }
}

/// Which log to follow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LogChoice {
    /// The newest file of the folder, switching when a newer one appears.
    #[default]
    Newest,

    /// One file.
    File(PathBuf),
}

/// What the game log tool shows.
#[derive(Clone, Debug, Default)]
pub struct LogState {
    /// The RPT folder: the stored one, else the detected one.
    pub folder: Option<PathBuf>,

    /// The RPT files of the folder, newest first.
    pub files: Vec<PathBuf>,

    /// The followed file.
    pub file: Option<PathBuf>,

    /// Its lines, at most [`MAX_LOG_LINES`].
    pub lines: VecDeque<LogLine>,

    /// Increases whenever `lines` change.
    pub revision: u64,

    /// The last read error.
    pub error: Option<String>,

    /// The follow whose updates are kept; updates of an older one are dropped.
    session: u64,
}

impl LogState {
    fn push(&mut self, text: impl Into<String>) {
        if self.lines.len() == MAX_LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(LogLine::new(text));
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.error = None;
        self.revision += 1;
    }
}

/// The stored RPT folder.
fn stored_folder(state: &AppState) -> Option<PathBuf> {
    state
        .client
        .configuration()
        .rpt_options
        .rpt_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Stores the RPT folder.
pub fn set_rpt_folder(cx: &Cx, folder: &Path) -> Result<(), ServiceError> {
    cx.state()
        .write()
        .client
        .set_rpt_path(Some(folder.display().to_string()))
}

/// The `.rpt` files of `folder`, newest first.
pub fn rpt_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rpt"))
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    files.into_iter().map(|(_, path)| path).collect()
}

/// Follows the log `choice` names in [`AppState::tools`]`.log`, replacing any follow.
pub fn follow_log(cx: &Cx, choice: LogChoice) -> Result<TaskId, ContextError> {
    let (session, stored, platform) = {
        let mut state = cx.state().write();
        let log = &mut state.tools.log;
        log.session += 1;
        log.file = None;
        log.clear();
        (
            log.session,
            stored_folder(&state),
            state.tools.platform.clone(),
        )
    };
    cancel_follow(cx);
    cx.spawn_task(
        TaskKey::new(TaskKind::RptTail),
        false,
        move |task| async move {
            let folder = match stored {
                Some(folder) => Some(folder),
                None => blocking(&task, move |_| platform.rpt_dir()).await?,
            };
            let mut tail: Option<RptTail> = None;
            let mut followed: Option<PathBuf> = None;
            let outcome = loop {
                let files = match folder.clone() {
                    Some(folder) => blocking(&task, move |_| rpt_files(&folder)).await?,
                    None => Vec::new(),
                };
                let target = match &choice {
                    LogChoice::Newest => files.first().cloned(),
                    LogChoice::File(path) => Some(path.clone()),
                };
                {
                    let mut state = task.cx.state().write();
                    let log = &mut state.tools.log;
                    if log.session != session {
                        break Ok(());
                    }
                    log.folder.clone_from(&folder);
                    log.files = files;
                    if target != followed {
                        log.file.clone_from(&target);
                        log.clear();
                    }
                }
                if target != followed {
                    if let Some(old) = tail.take() {
                        blocking(&task, move |_| old.stop()).await?;
                    }
                    tail = match &target {
                        Some(path) => Some(
                            start_tail(&task.cx, session, path.clone())
                                .map_err(|err| follow_error(&err))?,
                        ),
                        None => None,
                    };
                    followed = target;
                    task.cx.shared.events.repaint();
                }
                tokio::select! {
                    () = task.cancel.cancelled() => break Err(TaskError::Cancelled),
                    () = tokio::time::sleep(LIST_INTERVAL) => {}
                }
            };
            if let Some(old) = tail.take() {
                // Stopping joins the tail thread; that wait belongs on the blocking pool.
                tokio::task::spawn_blocking(move || old.stop());
            }
            outcome
        },
    )
}

/// Stops following the log.
pub fn stop_log(cx: &Cx) {
    cx.state().write().tools.log.session += 1;
    cancel_follow(cx);
}

fn cancel_follow(cx: &Cx) {
    for task in cx.running_tasks() {
        if task.key.kind == TaskKind::RptTail {
            cx.cancel_task(task.id);
        }
    }
}

/// Tails `path` on its own thread into the log of `session`.
fn start_tail(cx: &Cx, session: u64, path: PathBuf) -> std::io::Result<RptTail> {
    let cx = cx.clone();
    let sink = move |event: RptEvent| {
        {
            let mut state = cx.state().write();
            let log = &mut state.tools.log;
            if log.session != session {
                return;
            }
            match event {
                RptEvent::Preloaded(text) => text.lines().for_each(|line| log.push(line)),
                RptEvent::Reset => log.lines.clear(),
                RptEvent::Lines(lines) => lines.into_iter().for_each(|line| log.push(line)),
                RptEvent::Error(message) => log.error = Some(message),
            }
            log.revision += 1;
        }
        cx.shared.events.repaint();
    };
    RptTail::spawn(path, DEFAULT_POLL_INTERVAL, sink)
}

fn follow_error(err: &std::io::Error) -> TaskError {
    TaskError::Message(a3s_sync::EventError::new(
        a3s_foundation::error::ErrorKind::Io,
        format!("Cannot follow the log: {err}"),
    ))
}
