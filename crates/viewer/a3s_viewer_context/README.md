# a3s_viewer_context

The viewer's application core, without any UI code: the shared `AppState` (the client data plus
per-repository runtime state such as status, last check error and remote metadata), a tokio
runtime, the command bus from the UI, the event channel to the UI, named background tasks with
cancellation and coalesced progress, and the periodic repository check with its automatic
updates.

The UI builds one `ViewerContext` from the loaded `ClientData`, sets a repaint hook (for egui,
`RepaintHook::from_fn(move || ctx.request_repaint())`) and calls `start_background`. Each frame
it reads the state with `read()`, drains `poll_events()` and shows running tasks from
`running_tasks()` and `task_snapshot()`. User actions are `send(Command::…)` (for example the
check shortcut sends `repositories::Command::Check`) or a synchronous `mutate`. A periodic check
that finds updates emits `repositories::Event::UpdatesAvailable` (shown as a toast; the
repository badges come from the runtime status); repositories set to update automatically are
synchronized in the background by `AutoUpdate` tasks when a `Synchronizer` is configured. On
exit: `prepare_exit`, `save_on_exit` (on failure the UI asks whether to exit anyway and, if the
user stays, calls `start_background` again), then `shutdown`.

The user's downloads are `sync` commands: `Compare` checks a repository against its download
folder and publishes the changes (`sync::Downloads::plans`), keeping the checked session so
`Download` starts at once; `Pause` cancels and keeps the session for the next `Download` to
resume, `Stop` discards it, `Retry` downloads failed files again. Extra local files go to the
Trash. Each finished download is recorded in `sync::Downloads::recent` and announced with
`sync::Event::DownloadFinished`. Repository administration is `admin`: `refresh` reads the published repository (download
protocol) and compares the source folder with it without hashing (the build's SHA-1 cache tells
which files still match) into `admin::Publishing`; `publish` runs scan, build, verify and upload
as one cancellable `Publish` task whose steps report their state and detail, and retries from a
failed or stopped step (an interrupted upload resumes); `edit_events` changes the events and
writes the source folder's `.a3s/events` at once, which the next build carries and the upload
publishes, and `upload_events` uploads them alone. `profiles::use_event_preset` selects an event's mods in the
active profile; `repositories::test_connection` and `fetch_autoconfig` serve the repository
sheet.

Network access and time go through two seams, `RemoteChecker` and `Clock`, plus `Synchronizer`
for background downloads; tests use fakes and `ManualClock`. Each domain (`repositories`,
`sync`, `admin`, `profiles`, `launch`, `tools`) has its own `Command`, `Event` and `State`.

Boundaries: depends on the foundation, store and net crates; no egui. Never hold a state guard
across `.await`, and keep network and recursive file I/O outside the lock. Drop the context on
a plain thread, not inside an async context.
