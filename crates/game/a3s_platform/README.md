# a3s_platform

Operating-system integration of the client, one module per concern:

- `registry`: Windows registry reads and writes (`winreg`, Windows only), an in-memory registry
  for tests, and the process-wide positive and negative cache Java kept (`system_registry()`).
- `detect`, `steam`: the Arma 3, Steam and TeamSpeak 3 folders, the Arma 3 RPT folder, the Java
  client's install folders; Steam libraries (`libraryfolders.vdf`, `appmanifest_*.acf`) and
  Proton prefixes on Linux.
- `folders`: Documents, Desktop and home, the Arma 3 user folder and the log-export folder.
- `process`: "is Arma 3 / TeamSpeak / Steam running" (`tasklist` on Windows, `/proc` on Linux).
- `autostart`: start with the OS (per-user `Run` value, XDG autostart entry, macOS LaunchAgent).
- `shortcut`: desktop shortcuts (`.lnk` on Windows, `.desktop` on Linux).
- `single_instance`: one client per configuration folder; a second start asks the first to come to
  the front (`acquire`; the caller exits with `EXIT_CODE_ALREADY_RUNNING`).

Every OS access goes through an injectable seam (`PlatformEnv`, `RegistryRead`/`RegistryWrite`,
`SpecialFolders`, `ProcessProbe`), so tests of every OS's behaviour run on any host without
touching the real registry, autostart folders or game installs. Detection never fails: an
unreadable source is "not found".

Boundaries: depends only on `a3s_foundation`; OS-specific crates (`winreg`, `mslnk`) are
Windows-only dependencies confined here. No UI: bringing the window to the front, dialogs and
persisting preferences belong to the viewer.
