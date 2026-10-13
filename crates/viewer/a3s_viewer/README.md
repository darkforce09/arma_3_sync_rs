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
