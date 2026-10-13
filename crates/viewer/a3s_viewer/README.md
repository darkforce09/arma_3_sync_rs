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
- Sheets (`egui::Modal`, one at a time): Welcome on the first start or while Java data waits for
  import (Arma 3 and mods folders through the system folder dialog, the legacy importer),
  load warnings, quit confirmation (running tasks, failed save), Add repository.
- Toasts (egui-toast) for the context's events, including an Undo kind that runs a callback.
- Servers screen: the favourite servers with address, preset and a saved-password mark; Join
  stores the server (and its preset as the default addon group) in the configuration's
  `server_name`/`default_modset`, which the launch reads; add (+), edit (double-click, a sheet),
  remove with Undo.
- Settings (`SettingsWindow`, its own OS window through an immediate viewport, with its own
  toasts). Every change is written at once through `ViewerContext::mutate` and the
  `ClientData` services; there is no OK or Apply.
  - General: appearance (stored in the preferences' look-and-feel field: the Java "Noire" look
    is Dark, its other named looks Light, the default look Auto), the window when the game
    starts, open at login (`a3s_platform::autostart`, entry with `-run`), keep running in the
    tray, the check frequency (rescheduled at once), update notifications of every repository,
    and the version with an update check (`a3s_updater`, no download).
  - Game, per profile: Arma 3 folder, start through Steam or the executable (Linux; the
    profile's `game_start`), BattlEye, in-game name (the Arma 3 profiles next to the Arma 3 user
    folder), common flags, "Customize…" sheets for performance and developer options, and the
    command line from `a3s_launch` over a background addon scan.
  - Profiles: add, rename (double-click), duplicate, remove with Undo, desktop shortcut.
  - Downloads: mods folder, connections, speed limit and timeouts, written to every repository
    (the client keeps them per repository).
  - Network: the proxy. Companion apps: external applications, also by dropping a program
    onto the pane. Data: Java data import, auto-config import and export, the logs folder.
- The tool windows as separate OS windows (deferred viewports).
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
