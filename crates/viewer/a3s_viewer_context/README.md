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

The active profile's addons are scanned by an `AddonScan` task (`profiles::Command::ScanAddons`;
the UI calls `profiles::ensure_scan` every frame, which asks again whenever the profile, its
folders or its Arma 3 executable changed) into `profiles::State::scan`: the addon registry, the
available-addons tree and each addon's size; the launch priority list is brought in line with it.
`launch::Command::Play` runs a `Launch` task: optionally the downloads of the enabled addon
groups' repositories (as `Download` tasks), then the launch plan of the active profile
(`a3s_launch::plan_launch`), the companion apps and the game through the `launch::GameLauncher`
in `launch::State` (tests put a recording launcher there). Launch problems never stop the game:
they arrive as `launch::Event`s.

Network access and time go through two seams, `RemoteChecker` and `Clock`, plus `Synchronizer`
for background downloads; tests use fakes and `ManualClock`. Each domain (`repositories`,
`sync`, `admin`, `profiles`, `launch`, `tools`) has its own `Command`, `Event` and `State`.

Boundaries: depends on the foundation, store and net crates; no egui. Never hold a state guard
across `.await`, and keep network and recursive file I/O outside the lock. Drop the context on
a plain thread, not inside an async context.
