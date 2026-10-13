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
  active profile's addon groups has updates). Play starts the context's Launch task (after the
  updates of the enabled addon groups for "Update and play"); without a usable Arma 3 executable
  a toast offers "Choose folder…" (Settings > Game). Once the game started, the "game launch"
  preference hides the window to the tray, minimizes it, quits or does nothing.
- Source-list sidebar: Library (Mods, Servers, Activity), Repositories with badges from the
  runtime state (changed-addon count when updated, "…" while checking, "!" after a failed
  check) and Add repository, Publishing (repositories with a local build folder), and the last
  check time. Each row routes to a screen under `screens/`.
- Mods screen: All / Enabled / Load order. The tree shows the active profile's addon groups
  (from repositories, events and the user) with a status pill per mod (Ready, Update with its
  download size when the repository's comparison is current, Not installed with its size,
  Missing, Enabled twice) and the addons found on this computer by folder; the toolbar search
  filters it, Space or the checkbox toggles a row and every change is saved at once. "+" adds a
  group, a mods folder or a group from a repository, "−" removes the selected group or folder
  (Undo in a toast); double-click renames a user group. "Event preset" enables exactly an
  event's mods. Load order lists the enabled mods in the order the game loads them; dragging
  saves the priority list. Dropping an Arma 3 Launcher preset (`.html`) opens a sheet comparing
  it with the local mods, whose "Enable matched" adds a group with the matched mods.
- Repository screen: header with the URL, revision and mod count, the "…" menu (Edit
  connection, Check now, Remove repository with an Undo toast), one primary button ("Update
  <size>", "Resume <size>", "Up to date"), event cards with "Use preset", the changes a download
  makes (new, updated, removed to the Trash, with sizes) and the collapsed changelog of the
  revisions not downloaded yet. Opening it compares the repository with its download folder
  when the last comparison is older than the server revision.
- Event presets, from the Mods screen or an event card, go through the context's
  `profiles::use_event_preset` (the event's required mods enabled, every other mod disabled)
  with an Undo toast.
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
  download folder; login, SSL validation and the proxy under Advanced; Test connection), Update
  issues (one plain line per failed file, Retry, Retry all, Copy report, Play anyway; during
  "Update and play" the game starts anyway and the sheet offers no Play), the Arma 3 Launcher
  preset comparison, Add server and Edit server, Upload connection (protocol, host, port, path,
  login) and the path lists of a published repository (add through the system dialog, remove
  with Undo).
- Toasts (egui-toast) for the context's events, including an Undo kind that runs a callback and
  an action kind whose button asks the window for an action (e.g. "Choose folder…" opens
  Settings at the Game pane).
- Servers screen: the favourite servers with address, preset and a saved-password mark; Join
  stores the server (and its preset as the default addon group) in the configuration's
  `server_name`/`default_modset`, which the launch reads; add (+), edit (double-click, the
  server sheet), remove with Undo.
- Settings (`SettingsWindow`, its own OS window through an immediate viewport, with its own
  toasts). Every change is written at once through `ViewerContext::mutate` and the
  `ClientData` services; there is no OK or Apply.
  - General: appearance (stored in the preferences' look-and-feel field: the Java "Noire" look
    is Dark, its other named looks Light, the default look Auto), the window when the game
    starts, open at login (`a3s_platform::autostart`, entry with `-run`), keep running in the
    tray, the check frequency (rescheduled at once), update notifications of every repository,
    and the version with an update check (`a3s_updater`, no download).
  - Game, per profile: Arma 3 folder, start through Steam or the executable (Linux; the
    profile's `game_start`, which Play uses), BattlEye, in-game name (the Arma 3 profiles next
    to the Arma 3 user folder), common flags, "Customize…" sheets for performance and developer
    options, and the command line planned by `a3s_launch::plan_launch` (the same planning as
    Play) over a background addon scan.
  - Profiles: add, rename (double-click), duplicate, remove with Undo, desktop shortcut.
  - Downloads: mods folder, connections, speed limit and timeouts, written to every repository
    (the client keeps them per repository).
  - Network: the proxy. Companion apps: external applications, also by dropping a program
    onto the pane. Data: Java data import, auto-config import and export, the logs folder.
- The tool windows, each its own OS window (an immediate viewport, like Settings) with its own
  toasts for its results:
  - Server keys: the enabled mods of the active profile (From), the target folder (To, saved
    as soon as it is chosen), the keys found with the mods without a key behind a disclosure,
    and "Copy N keys".
  - Radio plugin: TFAR or ACRE 2; the plugin in the mod, the TeamSpeak 3 folder (stored or
    detected, "Change…") and version, the installed plugin, a warning while TeamSpeak runs
    or the folders are incomplete, and "Install plugin" / "Update plugin".
  - Game log: the newest RPT log followed live (or an older one from the file pop-up),
    monospace and coloured by line type, a filter field, "Errors only", "Show in folder"; the
    view follows the end unless scrolled up.
- Tray icon (StatusNotifierItem through `ksni` on Linux, `tray-icon` on Windows and macOS):
  Show, Hide, Check now, Quit. Closing or minimizing hides into the tray when the "minimized"
  preference says Tray; otherwise closing quits (prepare, save, close).
- Shortcuts: ⌘/Ctrl+R check now, ⌘/Ctrl+, Settings, ⌘/Ctrl+F search.

Tests drive `Shell` headless with `egui_kittest`: call `logic` then `show` per frame; the
Settings window is tested alone (`SettingsWindow::show`) over a `Desktop::isolated` home folder.
`tests/snapshots.rs` renders through wgpu on lavapipe (Linux); regenerate with
`UPDATE_SNAPSHOTS=1`.

Boundaries: the only crate that knows eframe, the tray and desktop notifications. Domain work
goes through `ViewerContext`; no network or recursive file I/O on the UI thread.
