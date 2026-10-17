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
folder and publishes what a download does to every file and folder (`sync::Downloads::plans`,
a `sync::StatusTree`), keeping the checked session so `Download` starts at once; a
`sync::Selection` limits it (everything but some paths, or only some paths). A check asked for
by the user compares again every repository compared before, so files changed on disk show at
an unchanged revision; `sync::plan_is_stale` says when a comparison is out of date (another
server revision, or made before the addon scan that auto-discovery uses). Auto-discovery finds
addon folders through the active profile's addon scan. While Arma 3 runs, `Download` does not start
and emits `sync::Event::GameRunning` instead; `DownloadAnyway` starts it (automatic updates
wait for the next check). `Pause` cancels and keeps the session for `Resume` (with its event
and selection), `Stop` discards it, `Retry` downloads failed files again. A lost connection is
resumed by itself after `RECONNECT_DELAY`, at most `RECONNECT_ATTEMPTS` times in a row
(`sync::Downloads::reconnecting` while it waits). Extra local files go to the Trash, except in
the repository's kept folders. Each finished download is recorded with its report in
`sync::Downloads::recent` and announced with `sync::Event::DownloadFinished`.
`profiles::use_event_preset` and `use_group_preset` make an event's mods or one addon group
the selection of the active profile; after each successful check and download the groups made
from a repository and its events follow its new content in every profile, keeping their
enabled mods; `repositories::test_connection` and
`fetch_autoconfig` serve the repository sheet.

The active profile's addons are scanned by an `AddonScan` task (`profiles::Command::ScanAddons`;
the UI calls `profiles::ensure_scan` every frame, which asks again whenever the profile, its
folders or its Arma 3 executable changed, and after every download; `profiles::rescan` asks
for one at once) into `profiles::State::scan`: the addon registry, the available-addons tree
and each addon's size; the launch priority list is brought in line with it.
`launch::Command::Play` runs a `Launch` task: it enables the preset group of the server it
joins, then optionally the user's downloads of the enabled mods of the enabled addon
groups' repositories that have updates or lack one of those mods
(`sync::download_selection`, so failed files reach `sync::Downloads::recent` and
`sync::Event::DownloadFinished` like any download), then the launch plan of the active profile
(`a3s_launch::plan_launch`), the companion apps and the game through the
`launch::GameLauncher` in `launch::State` (tests put a recording launcher there).
Update and launch problems never stop the game: launch problems arrive as `launch::Event`s.

Repository administration is `admin`: `refresh` reads the published repository (download
protocol) and compares the source folder with it without hashing (the build's SHA-1 cache tells
which files still match) into `admin::Publishing`; `publish` runs scan, build, verify and upload
as one cancellable `Publish` task whose steps report their state and detail, and retries from a
failed or stopped step (an interrupted upload resumes); without an upload connection the server
is expected to serve the source folder, and the run ends after Verify finds every file there;
`edit_events` changes the events and writes the source folder's `.a3s/events` at once, which
the next build carries and the upload publishes, and `upload_events` uploads them alone.
`autoconfig_servers` and `set_autoconfig_server` choose which of the client's favourite servers
the next build writes to the repository's `.a3s/autoconfig`. `test_partial_transfer` asks the
server whether it answers range requests and `check_server` whether it holds every published
file (into `admin::Publishing`); `test_upload_connection` tries an upload connection. The local
scan carries the last builds' changelogs (`LocalScan::history`).

The tool windows call `tools` functions with the context's `Cx` and read `tools::State`:
`find_keys`, `find_keys_in_folder` and `copy_keys` (the `.bikey` files of the active profile's
enabled mods or of a stored source folder, into the stored target folder), `inspect_radio`
and `install_radio` (the TFAR or ACRE 2 plugin of the mods, TeamSpeak 3's folder, version and
installed plugin, the copy as a task), `follow_log` and `stop_log` (the newest or a chosen RPT
log tailed into `tools::LogState`, each line classified as error, warning or other). The keys
and the radio mod are looked up in `profiles::State::scan` when it matches the active profile,
else in a fresh scan of it. OS lookups (TeamSpeak and RPT folders, running processes, a running
Arma 3) go through `tools::Platform`, which tests replace.

Network access and time go through two seams, `RemoteChecker` and `Clock`, plus `Synchronizer`
for background downloads; tests use fakes and `ManualClock`. Each domain (`repositories`,
`sync`, `admin`, `profiles`, `launch`, `tools`) has its own `Command`, `Event` and `State`.

Boundaries: depends on the foundation, store and net crates; no egui. Never hold a state guard
across `.await`, and keep network and recursive file I/O outside the lock. Drop the context on
a plain thread, not inside an async context.
