# a3s_viewer

The Arma3Sync desktop window, built with eframe over `a3s_viewer_context` (state, commands,
background work) and `a3s_ui` (theme, icons, widgets).

`run(&RunOptions)` takes the single-instance lock (a second start shows the first window and
returns `Exit::AlreadyRunning`), loads the client data, starts the context's background work,
puts the icon in the tray and opens the window. The `arma3sync` binary calls it for no command
and the Java flags `-dev`, `-run` (start hidden in the tray) and `-unlock` (no single-instance
lock), after checking `display_available()`.

The window (`Shell`):

- Unified toolbar: screen title, search field (⌘/Ctrl+F), the menu (Check now, Settings, Tools,
  Quit), the profile pop-up and the Play button ("Update and play" while a repository of the
  active profile's addon groups has updates). Play calls `launch::request`, which only logs.
- Source-list sidebar: Library (Mods, Servers, Activity), Repositories with badges from the
  runtime state (changed-addon count when updated, "…" while checking, "!" after a failed
  check) and Add repository, Publishing (repositories with a local build folder), and the last
  check time. Each row routes to a screen under `screens/`.
- Repository screen: header with the URL, revision and mod count, the "…" menu (Edit
  connection, Check now, Remove repository with an Undo toast), one primary button ("Update
  <size>", "Resume <size>", "Up to date"), event cards with "Use preset", the changes a download
  makes (new, updated, removed to the Trash, with sizes) and the collapsed changelog of the
  revisions not downloaded yet. Opening it compares the repository with its download folder
  when the last comparison is older than the server revision.
- Activity screen: running downloads (progress, bytes, speed, time left, current files, Pause
  and Stop), paused downloads (Resume, Stop) and the recent results ("Show issues").
- Publishing screen (a repository built on this computer), with a Publish / Events / Settings
  segment and one primary button ("Publish revision N", "Upload events" when only the events
  changed, "Published"). Publish: tiles for the published revision, the local changes and the
  repository size; the step list (Scan source folder, Build, Verify, Upload) with live details
  and Retry or Resume under a failed or stopped step; the mods players get. Events: the event
  list (Add, Duplicate, Remove with Undo) and the selected event (name, description, each mod
  required or optional), saved to the source folder as you type. Settings: source folder,
  upload connection, the auto-config link with Copy, the build and upload switches, and the
  excluded files and hidden folders.
- Sheets (`egui::Modal`, one at a time): Welcome on the first start or while Java data waits for
  import (Arma 3 and mods folders through the system folder dialog, the legacy importer),
  load warnings, quit confirmation (running tasks, failed save), Add repository and Edit
  connection (an auto-config link fills the fields; protocol, address, port, anonymous,
  download folder; login, SSL validation and the proxy under Advanced; Test connection), and
  Update issues (one plain line per failed file, Retry, Retry all, Copy report, Play anyway),
  Upload connection (protocol, host, port, path, login) and the path lists of a published
  repository (add through the system dialog, remove with Undo).
- Toasts (egui-toast) for the context's events, including an Undo kind that runs a callback.
- Settings and the tool windows as separate OS windows (deferred viewports).
- Tray icon (StatusNotifierItem through `ksni` on Linux, `tray-icon` on Windows and macOS):
  Show, Hide, Check now, Quit. Closing or minimizing hides into the tray when the "minimized"
  preference says Tray; otherwise closing quits (prepare, save, close).
- Shortcuts: ⌘/Ctrl+R check now, ⌘/Ctrl+, Settings, ⌘/Ctrl+F search.

Tests drive `Shell` headless with `egui_kittest`: call `logic` then `show` per frame.
`tests/snapshots.rs` renders through wgpu on lavapipe (Linux); regenerate with
`UPDATE_SNAPSHOTS=1`.

Boundaries: the only crate that knows eframe, the tray and desktop notifications. Domain work
goes through `ViewerContext`; no network or recursive file I/O on the UI thread.
