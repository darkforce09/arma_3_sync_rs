# a3s_updater

Self-updater core: reads `updater.toml`, finds the latest release, downloads its archive with
SHA-256 verification and installs it into an installation directory.

How to use: `UpdateConfig::load` (with `ConfigOverrides::from_env()`), `resolve_release` for the
target triple from `host_target()`, `is_newer` against the running version, `download_release`
into a `StagingDir`, then `install_archive` with a `FileReplacer` (`CopyReplacer` renames each
file over its target). `DiagnosticLog` is the opt-in file log (`A3S_UPDATER_DEBUG=true`).

Release sources, in order: the GitHub Releases API (asset chosen by
`github.asset_pattern`, default `Arma3Sync-{version}-{target}.zip`), the JSON manifest
`a3s.json` (optional per-target `targets` table), the remote legacy `a3s.xml`, then the
installation's own `a3s.xml`. Legacy XML releases carry no checksum and are refused for download
unless `DownloadOptions::allow_unverified` is set.

Archives are untrusted: every entry name is checked (no `..` escape, absolute, drive or device
paths, symbolic links), every CRC is verified and the size limits are enforced before the
installation is touched. `.a3s-updater-files` in the installation lists the files the updater
installed, so the next update deletes the ones it no longer ships; user data (`a3s.cfg`,
`updater.toml`, `profiles/`, ...) is never written or deleted.

Boundaries: HTTP(S) only, through its own `reqwest` client (`User-Agent: ArmA3Sync-Updater`,
HTTPS-only unless `allow_http`). Replacing the running executable, elevation, relaunch and the
command-line or UI flow belong to the caller.
