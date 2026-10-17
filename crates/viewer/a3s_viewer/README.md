# a3s_viewer

The Arma3Sync desktop window, built with eframe over `a3s_viewer_context` (state, commands,
background work) and `a3s_ui` (theme, icons, widgets).

`run(&RunOptions)` takes the single-instance lock (a second start shows the first window and
returns `Exit::AlreadyRunning`), loads the client data, starts the context's background work,
puts the icon in the tray and opens the window. The `arma3sync` binary calls it for no command
and the Java flags `-dev`, `-run` (start hidden in the tray) and `-unlock` (no single-instance
lock), after checking `display_available()`.

The window (`Shell`):

- Unified toolbar: screen title, search field (⌘/Ctrl+F; it narrows the lists of the current
  screen: the Mods tree, the servers, Activity, a repository's changes, the Publishing events
  and the mods players get), the menu (Check now, Rescan mods folders, Settings, Tools, About
  Arma3Sync, Quit), the profile pop-up and the Play button ("Update and play"
  while a repository with an enabled mod of the active profile has updates). Play starts the
  context's Launch task (after downloading the enabled mods of those repositories for "Update
  and play"); without a usable Arma 3 executable a toast offers "Choose folder…" (Settings >
  Game). Before starting, mods enabled twice stop it with an explanation, and enabled mods not
  on this computer ask first ("Update and play" when a repository has them, "Play without
  them", Cancel). Once the game started, the "game launch" preference hides the window to the
  tray, minimizes it, quits or does nothing; while it restarts by itself, "Stop restarting"
  ends that.
- Source-list sidebar (resizable; long names end in "…" and show whole on hover): Library
  (Mods, Servers, Activity), Repositories with badges from the runtime state (the top-level
  folders a download changes, from the current comparison, else the addons changed in the
  changelogs when a newer revision is out; "…" while checking, "!" after a failed check, with
  its reason as tooltip) and Add repository, Publishing (repositories with a local source
  folder, and "Publish a folder", which creates one), and the last check time (with Stop while
  a check asked for by the user runs). Each row routes to a screen under `screens/`.
- Mods screen: All / Enabled / Load order. The tree shows the active profile's addon groups
  (from repositories, events and the user) with a status pill per mod (Ready, Update with its
  download size when the repository's comparison is current, Not installed with its size,
  Missing, Enabled twice) and the addons found on this computer by folder (scanned again after
  every download and on "Rescan mods folders"; the groups made from a repository or its events
  follow it after each check and download, keeping their enabled mods); the toolbar search
  filters it, Space or the checkbox toggles a row and every change is saved at once; a mod
  dragged from "On this computer" onto a user group joins it; a mod's tooltip says where it is
  and optional event mods say "(optional)". "+" adds a group, a mods folder, a group from a
  repository or from an event (Undo), or imports an Arma 3 Launcher preset; "−" removes the
  selected group or folder (Undo in a toast); the "⋯" menu expands or collapses every group,
  disables every mod (Undo) or scans again. The context menu renames a user group at any depth
  (double-click too; a refused name says why), duplicates a group, exports a group's enabled
  mods as an Arma 3 Launcher preset (asking for the Workshop ids it cannot find, remembered for
  the next export), moves a mods folder up or down (the upper folder wins a duplicate name) and
  removes. "Event preset" enables exactly an event's mods. Load order lists the enabled mods in
  the order the game loads them; dragging saves the priority list. A preset dropped or
  imported opens a sheet comparing it with the local mods (missing ones with their Workshop id
  and "Open in Steam"), whose "Enable matched" adds a group with the matched mods.
- Repository screen: the whole repository on one screen. The header has the URL, revision, build
  date, mod count and download folder (Change…), the "…" menu (Edit connection, Download
  settings, Publish from this computer, Remove repository with an Undo toast) and one primary
  button ("Update <size>" of the ticked items, "Download <size>" when nothing is on disk yet,
  "Nothing selected", "Resume <size>", "Updating… N%", "Up to date"). One status line says
  "Checking the repository…" or "Comparing with your download folder…" with a spinner, else
  when it was checked and what a download does ("1 to update (2 files, 237.05 KB) · 14 up to
  date"), next to Check now (checks and compares again) and Changelog (the last 10 builds). The
  running or paused download of the repository shows under it as on the Activity screen (Pause,
  Stop, Resume; then the result with Report and Show issues), then notices (a failed check or
  comparison) and one row per event (Download only its mods, Use preset). The toolbar has Select
  all, Select none, Expand all / Collapse all, the switches "Use copies in other mods folders"
  (auto-discover) and "Remove extra files" (exact match, never in the game folder), and
  Settings…; the switches and Change… wait while a download runs or is paused. The body is the
  tree of every top-level folder and file of the repository, each with a checkbox, a status
  pill (Up to date, Update, Not downloaded, Removed, Downloading) and the files and bytes it
  needs, every folder opening to all its files with their own status; ticks (also on single
  files) choose what the next Update downloads, an unticked removed folder stays, and "Keep
  this folder" in its context menu keeps it for good. While Arma 3 runs, Update asks first.
  Opening it compares the repository with its download folder when the last comparison is
  missing or out of date.
- Event presets, from the Mods screen or an event card, go through the context's
  `profiles::use_event_preset` (the event's required mods enabled, every other mod disabled)
  with an Undo toast.
- Activity screen: running downloads (progress, bytes, speed, time left, current files, Pause
  and Stop, connections and disk writes; "Waiting for the server" after 15 s without data; the
  automatic retries after a lost connection; Details: files, the current file), paused
  downloads (Resume, Stop) and the recent results ("Report", "Show issues").
- Publishing screen (a repository with a source folder on this computer; without one it asks
  for the folder), with a Publish / Events / Settings segment and one primary button
  ("Publish revision N", "Build revision N" without an upload connection, "Upload events" when
  only the events changed, "Published"). Without an upload connection the server is expected
  to serve the source folder: the build counts as published when Verify finds every file.
  Publish: tiles for the published revision, the local changes and the repository size; the
  step list (Scan source folder, Build, Verify, Upload) with live details and Retry or Resume
  under a failed or stopped step; the mods players get; the History of the last builds (Copy);
  the result of "Check the server files" from the "⋯" menu (every missing file listed). Events:
  the event list (Add, Duplicate, Remove with Undo) and the selected event (name, description,
  each mod required or optional, Select all and Select none, the required DLC), saved to the
  source folder as you type. Settings: source folder (and Stop publishing from this computer,
  with Undo), upload connection, the auto-config link with Copy, the build and upload switches,
  "Test the server" for partial downloads (HTTP), connections per player and parallel uploads
  (1 to 10), which of the favourite servers the auto-config carries (stored at once, published
  by the next publish), and the excluded files and hidden folders.
- Sheets (`egui::Modal`, one at a time): Welcome on the first start or while Java data waits for
  import (Arma 3 and mods folders through the system folder dialog, the legacy importer),
  load warnings, quit confirmation (running tasks, failed save), Add repository and Edit
  connection (an auto-config link fills the fields; protocol, address, port, anonymous,
  download folder; login, SSL validation and the proxy under Advanced; Test connection), Publish
  a folder (the same form with the address players download from and a source folder instead
  of the auto-config link and the download folder), Download settings of one repository
  (download folder, notify, download updates automatically, connections up to the server's cap,
  speed limit, timeouts, the folders kept when updating; stored at once),
  Update issues (one sheet per repository; one plain line per failed file, Retry, Retry all,
  Copy report, Play anyway; during "Update and play" the game starts anyway and the sheet
  offers no Play), the Arma 3 Launcher preset comparison, Add server and Edit server, Upload
  connection (protocol, host, port, path, login; Test connection) and the path lists of a
  published repository (add through the system dialog, remove with Undo), a download report
  (Copy), the question before an update while Arma 3 runs, the Play check, the export of a
  launcher preset, and About (version, licence, project page, credits, "Log files…").
- Toasts (egui-toast) for the context's events, including an Undo kind that runs a callback and
  an action kind whose button asks the window for an action (e.g. "Choose folder…" opens
  Settings at the Game pane, and "Show" on "Updates available" opens the repository).
- Servers screen: the favourite servers with address, preset and a saved-password mark; Join
  stores the server (and its preset) in the configuration's `server_name`/`default_modset` and
  enables exactly the preset group's mods (with Undo); Play enables them again before
  launching; add (+), edit (double-click, the server sheet), remove with Undo.
- Settings (`SettingsWindow`, its own OS window through an immediate viewport, with its own
  toasts). Every change is written at once through `ViewerContext::mutate` and the
  `ClientData` services; there is no OK or Apply.
  - General: appearance (stored in the preferences' look-and-feel field: the Java "Noire" look
    is Dark, its other named looks Light, the default look Auto), the window when the game
    starts, open at login (`a3s_platform::autostart`, entry with `-run`), keep running in the
    tray, the check frequency (rescheduled at once), and the version with an update check
    (`a3s_updater`) and, when a newer version exists, "Install and restart" (download, verify
    and install; the window then saves, quits, and `run` starts the new version once the
    single-instance lock is released).
  - Game, per profile: Arma 3 folder, start through Steam or the executable (Linux; the
    profile's `game_start`, which Play uses), BattlEye, in-game name (the Arma 3 profiles next
    to the Arma 3 user folder), common flags and "Restart when it closes", "Customize…" sheets
    for performance (the memory allocators of the Arma 3 `Dll` folder) and developer options,
    each option explained in its tooltip, and the command line planned by
    `a3s_launch::plan_launch` (the same planning as Play) over a background addon scan.
  - Profiles: add, rename (double-click), duplicate, remove with Undo, desktop shortcut.
  - Network: the proxy. Companion apps: external applications, also by dropping a program
    onto the pane. Data: Java data import, auto-config import and export (a sheet chooses the
    profiles, favourite servers and repositories it takes), the logs folder.
- The tool windows, each its own OS window (an immediate viewport, like Settings) with its own
  toasts for its results:
  - Server keys: from the enabled mods of the active profile or from any folder (stored), the
    target folder (To, saved as soon as it is chosen), the keys found with the mods without a
    key behind a disclosure, and "Copy N keys".
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
