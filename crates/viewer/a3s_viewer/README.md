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
  (from repositories, events and the user) with a status pill per mod (Ready, Update, Not
  installed with its size, Missing, Enabled twice) and the addons found on this computer by
  folder; the toolbar search filters it, Space or the checkbox toggles a row and every change is
  saved at once. "+" adds a group, a mods folder or a group from a repository, "−" removes the
  selected group or folder (Undo in a toast); double-click renames a user group. "Event preset"
  enables exactly an event's mods (`screens::mods::use_event_preset`, also for other screens).
  Load order lists the enabled mods in the order the game loads them; dragging saves the
  priority list. Dropping an Arma 3 Launcher preset (`.html`) opens a sheet comparing it with
  the local mods, whose "Enable matched" adds a group with the matched mods.
- Sheets (`egui::Modal`, one at a time): Welcome on the first start or while Java data waits for
  import (Arma 3 and mods folders through the system folder dialog, the legacy importer),
  load warnings, quit confirmation (running tasks, failed save), Add repository, Arma 3 Launcher
  preset comparison.
- Toasts (egui-toast) for the context's events, including an Undo kind that runs a callback and
  an action kind whose button asks the window for an action.
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
