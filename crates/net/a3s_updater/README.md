# a3s_updater

Self-updater: reads `updater.toml`, finds the latest release, downloads its archive with SHA-256
verification and installs it into the running application's directory, replacing the running
binary and asking for UAC on Windows when the directory needs administrator rights.

How to use: build an `UpdateContext::for_current_process`, wrap it in an `UpdateSession`, call
`check` (cached per session), then `apply` with an event callback (`Preparing`, `Downloading`,
`WaitingForElevation`, `ElevatedInstallStarted`, `Installing`, `Done`) and a cancellation token.
`apply::console` runs the console flows (`run_console_update`, `run_check_only`,
`run_legacy_update_command`) and parses the updater flags; `apply::text` holds every text a GUI
shows and `ApplyError::dialog` maps an error to a dialog; `apply::relaunch` starts the updated
application after the caller released its single-instance lock. The binary must dispatch
`argv[1] == ELEVATED_SUBCOMMAND` to `run_elevated_install` before any other setup, and call
`cleanup_leftovers` on its directory at startup.

The building blocks stay public: `UpdateConfig::load` (with `ConfigOverrides::from_env()`),
`resolve_release` for the target triple from `host_target()`, `is_newer`, `download_release` into
a `StagingDir`, `validate_archive`, `stage_archive` and `install_staged` with a `FileReplacer`.
`DiagnosticLog` is the opt-in file log (`A3S_UPDATER_DEBUG=true`).

Release sources, in order: the GitHub Releases API (asset chosen by
`github.asset_pattern`, default `Arma3Sync-{version}-{target}.zip`), the JSON manifest
`a3s.json` (optional per-target `targets` table), the remote legacy `a3s.xml`, then the
installation's own `a3s.xml`. Legacy XML releases carry no checksum: they can be checked but not
applied.

Archives are untrusted: the archive is hashed into a private copy that validation and staging
both read; every entry name is checked (no `..` escape, absolute, drive or device paths,
symbolic links, case or file/directory collisions), every CRC is verified and the size limits
are enforced before the installation is touched. A release archive must hold `arma3sync[.exe]`
at its root and no portable user data. It is extracted inside the installation (the one place an
elevated installer can write and a less-privileged user cannot). `.a3s-updater-files` lists the
files the updater installed, so the next update deletes the ones it no longer ships; user data
(`a3s.cfg`, `updater.toml`, `profiles/`, the portable-mode marker and data folder, ...) is never
written or deleted.

Elevation (Windows only): the unelevated application downloads and verifies the archive, then
starts its own binary through PowerShell `Start-Process -Verb RunAs` with the hidden
subcommand; that child re-verifies the SHA-256, installs into its own directory, writes a
startup marker and a result file, and exits 0 or 1 while the parent waits.

Boundaries: HTTP(S) only, through its own `reqwest` client (`User-Agent: ArmA3Sync-Updater`,
HTTPS-only unless `allow_http`). AppImage, macOS bundle, Java and cargo-build installations are
checked but not updated. Rendering dialogs and command-line parsing belong to the application.
