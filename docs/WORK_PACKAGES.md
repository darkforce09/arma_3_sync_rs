# Work packages

Catalogue of the remaining work, split into packages sized for one sub-agent
(~500–2500 Rust lines). Each WP gets a full spec in `docs/specs/WPxx.md` before
implementation. Java paths are relative to
`java_version/Arma3Sync/src/main/java/fr/soe/a3s/` (`J/`), Java tests to
`.../src/test/java/fr/soe/a3s/` (`T/`), updater to `.../updater/src/` (`U/`).

General notes:
- The DTO layer (`J/dto/**`, `ObjectDTOtransformer`) is not ported. The client-side state carried by
  `SyncTreeLeafDTO`/`SyncTreeDirectoryDTO` (destinationPath, localSHA1, complete, status,
  selected, optional) becomes the `a3s_sync` plan model.
- Services used by both CLI and viewer (`RepositoryService`, `ProfileService`,
  `ConfigurationService`) live below `viewer` (in `a3s_repo_files::local`, `a3s_sync`,
  `a3s_admin`, `a3s_addons`), never in `top`.
- `repo.ofcra.org` is unreachable; live tests use WOG FTP only. HTTP/zsync are tested locally.

## Wave 0

| WP | Crate(s) | Java anchors | Deps | Notes / acceptance highlights |
| --- | --- | --- | --- | --- |
| WP01 | `tools/java_fixture_harness`, `a3s_dev_tools` | `J/dao/A3SFilesAccessor`, `EncryptionProvider`, `dao/repository/RepositoryBuildProcessor`, `service/Arma3LauncherPresetExporter`, `J/domain/**` (used via classpath, not ported) | — | **Needs JDK 25.** Deterministic fixtures: `a3s.cfg`, `a3s.prefs`, profiles, encrypted `*.a3s.repository` (HTTP/FTP/SFTP), `export.a3s.autoconfig`, sample repo built compressed/uncompressed for HTTP+FTP (`.a3s/*`, `.zsync`, `.pbo.zip`), launcher preset HTML, class-descriptor dump (incl. computed UIDs of `domain.AutoConfig`, `FileAttributes`, `configration.Proxy`), Java-reads-Rust-output cross-check command. |
| WP02 | `a3s_paths`, `a3s_log`, `a3s_format`, `a3s_error`, `a3s_build_info` | `J/dao/ApplicationPaths` (289), `DataAccessConstants`, `J/utils/DebugLogger`, `UnitConverter`, `RepositoryConsoleErrorPrinter`, `J/exception/**`, `J/main/Version`, `version.properties` | — | Port `T/dao/ApplicationPathsTest`, `T/main/VersionTest`. Rust config dir must not collide with Java's; legacy Java locations discoverable (no copy). `a3s_format` matches `UnitConverter` output. |
| WP03 | `a3s_types` (new `local` + `constants` modules) | `J/domain/repository/Repository` (426), `J/domain/Profile`, `Preferences`, `J/domain/configration/*`, `TreeDirectory`, `TreeLeaf`, `Addon`, `FileAttributes`, `AbstractProtocoleFactory`, `J/constant/*` | — | **Wire-touching.** serde (TOML) structs + lenient Java decoders by field name. Port `T/domain/RepositoryUploadSettingsTest`, `ProtocolSupportTest`. Repository defaults (check conns 4, upload 4, max 10, `noAutoDiscover=true`). |
| WP06 | `a3s_transport` (upload side) | `J/dao/connection/ftp/FtpDAO` (upload session, exists, mkdir, delete), `sftp/SftpDAO`, `webdav/WebdavDAO`, `AbstractConnexionDAO` upload parts | transport committed | `exists/upload_file/upload_bytes/mkdirs/delete` for FTP(S)/SFTP/WebDAV; HTTP → Unsupported. In-process test servers (e.g. `libunftp`, `russh` server, axum WebDAV stub). Path traversal rejected. |
| WP17 | `a3s_tools` (bikey, rpt, radio) | `J/dao/CommonDAO.extractBikeys*`, `J/ui/tools/rpt/Tail`, `LogFileTailer`, TFAR/ACRE2 logic in `J/ui/tools/tfar/*`, `acre2/*`, `ConfigurationService.isTS3*` | WP02 | Bikey copy+count+cancel; tailer handles append/truncate/rotate; installer validation messages identical. |
| WP20 | `a3s_ui` | `J/ui/theme/*`, `ThemeManager`, `UiStyle`, `UiColors`, `UIConstants`, `J/ui/icon/*` + 43 SVGs, `FontInstaller` + Geist TTFs, `J/ui/components/*`, `CheckTreeCellRenderer`, `IndeterminateIcon`, `AbstractDialog`, `AbstractProgressDialog` | — | Port `T/ui/theme/ThemeTokensTest`. Light/dark `egui::Visuals` from tokens, icons with tone/state tint, tri-state check tree, progress bar, modal + progress dialogs, demo example. |
| WP31 | `a3s_updater` (core) | `U/main/java/fr/soe/a3sUpdater/config/*`, `dao/JsonManifestDAO`, `GitHubReleaseDAO`, `HttpDAO`, `XmlDAO`, `FtpDAO` (safe extraction), `service/VersionComparator`, `DiagnosticLog` | — | Port 19 updater tests (UpdateConfig, JsonManifest, GitHubRelease, FtpDAOSafety, FtpDAOManagedFiles). SHA-256 verification, zip-slip rejection. |

## Wave 1

| WP | Crate(s) | Java anchors | Deps | Notes |
| --- | --- | --- | --- | --- |
| WP04 | `a3s_repo_files::{local, legacy}` | `J/dao/A3SFilesAccessor` (filter limits, atomic write), `EncryptionProvider` (AES-128-ECB key), `dao/repository/RepositoryDAO` (naming `sanitize__sha1[0..6]`, legacy names, readAll error map), `ProfileDAO`, `ConfigurationDAO` (r/w only), `PreferencesDAO` (r/w only) | WP02, WP03 | Port `T/dao/A3SFilesAccessorTest`, `PreferencesDAOTest`. TOML store, atomic writes, idempotent one-time import, default profile always present. JDK fixtures (WP01) deferred if absent. |
| WP05 | `a3s_types::protocol::url`, `a3s_repo_files::autoconfig_export` | `J/dao/connection/AutoConfigURLAccessMethods` (192), `J/service/SslValidationPolicy`, `J/dao/CommonDAO` import/export autoconfig, `SftpDAO` url helpers | WP03 | Port `AutoConfigURLAccessMethodsTest`, `SftpDAOTest`, `SslValidationPolicyTest`. |
| WP07 | `a3s_sync::{model, remote, status, events}` | `J/service/ConnectionService` (fetch metadata, checkRepository, importAutoConfig, partial-transfer probe), `J/dto/sync/*` (client state), `RemoteFile`, `J/exception/remote/*`, `RepositoryService` (`updateRepository` 256–390, `determineRepositorySyncStatus` 1475, revision handling), `J/ui/main/tasks/TaskCheckRepositories` logic | WP02, WP03 | Port `TaskCheckRepositoriesTest`. Freeze plan model + event enum + `SyncOptions` in first commit so WP08/09 can start. |
| WP11 | `a3s_admin::build` | `J/dao/repository/RepositoryBuildProcessor` (661), `J/service/administration/RepositoryBuildProcessor`, `RepositorySHA1Processor`, `RepositoryZsyncProcessor`, `RepositoryDeleteZSyncProcessor`, `J/dao/zip/ZipBatchProcessor`, `DeleteZipBatchProcessor`, `ZipDAO.zip`, `RepositoryService.buildRepository` etc. | WP03 | **Wire-touching.** `.a3s/*` byte-identical (dates pinned), `.zsync` byte-identical, `.pbo.zip` semantic. Revision only bumps on content change; changelog ≤ 10. JDK cross-check deferred if absent. |
| WP13 | `a3s_addons` | `J/service/AddonService` (731), `J/dao/AddonDAO`, `J/constant/GameDLCs`, `ProfileService` groups/priorities (251–400), `RepositoryService.getGroupFromRepository` etc. | WP03 | Addon tree scan, duplicates, DLC list, priorities, modsets from repo tree, missing/duplicate detection. |
| WP15 | `a3s_platform` | `J/dao/WinRegistry`, `PreferencesDAO` Run key, `ConfigurationDAO.determine*Path`, JShellLink usages, JUnique in `J/main/ArmA3Sync` 174–255 | WP02 | Single instance + focus message; autostart (HKCU Run / XDG autostart); Arma/Steam/TS3 detection; `.lnk` shortcuts. Windows code must `cargo check --target x86_64-pc-windows-gnu`. |

## Wave 2

| WP | Crate(s) | Java anchors | Deps | Notes |
| --- | --- | --- | --- | --- |
| WP08 | `a3s_sync::{plan, completion, event_filter}` | `RepositoryService.checkForAddons` 413 + destination/hidden/extra-file logic, `J/service/EventExtractor`, `service/synchronization/FilesCheckProcessor`, `FilesSynchronizationManager`, `FilesCompletionProcessor`, `service/connection/ConnectionCompletionProcessor`, `HttpDAO.getFileCompletion` | WP07 | Plan sets (new/updated/deleted/hidden/extra), exactMatch, autodiscover, SHA-1 cache, totals; completion % = `a3s_zsync::match_blocks`. |
| WP09 | `a3s_sync::download` | `service/connection/ConnectionDownloadProcessor`, `service/synchronization/FilesSynchronizationProcessor`, disk-usage + connection-wait listeners, `AbstractConnexionDAO.downloadFile`, `HttpDAO.downloadPartialFile`, `DataRange` | WP07 | K connections = min(client, server); `.part` + rename; resume; zsync patch; throttle; cancel < 1 s; max-error abort; ConnectionLost. |
| WP12 | `a3s_admin::{check, upload, events}` | `service/administration/RepositoryCheckProcessor`, `FilesUploadManager`, `RepositoryUploadProcessor`, `service/connection/ConnectionCheckProcessor`, `ConnectionUploadProcessor`, `ConnectionDeleteProcessor`, `ConnectionService.uploadRepository/upLoadEvents`, `RepositoryService` events CRUD 979–1186 | WP06, WP07, WP11 | Metadata uploaded last; second upload transfers 0 files; parallel ≤ 10. |
| WP14 | `a3s_repo_files::local::service`, `a3s_sync::proxy` | `J/service/ProfileService` (710), `ConfigurationService` (428), `PreferencesService`, `CommonService`, `RepositoryService` CRUD/setters | WP04, WP05 | All mutations persisted and tested; autoconfig export/import round trip; proxy → `ProxyConfig`. |
| WP16 | `a3s_launch` | `J/service/LaunchService` (482), `J/dao/LauncherDAO` (455) | WP13, WP15 | Golden run-parameter table; `Arma3.cfg` ModLauncherList strip; PowerShell elevation quoting; autoRestart. |
| WP18 | `a3s_tools::preset` | `J/service/Arma3LauncherPresetImporter`, `Arma3LauncherPresetExporter`, `WorkshopMetadataStore`, `docs/ARMA3_LAUNCHER_PRESET_EXPORT.md` | WP13, WP02 | Port 12 Java tests (`T/service/Arma3LauncherPreset*Test`). |
| WP32 | `a3s_updater::{apply, elevation}` | `U/.../service/Service`, `ElevationSupport` (454), `main/ArmA3SyncUpdater`, `console/Console`, `J/service/UpdaterProcess` | WP31, WP15 | Port `ElevationSupportTest` (10). Self-replace on Linux/Windows; check exit codes match Java. |

## Wave 3

| WP | Crate(s) | Java anchors | Deps | Notes |
| --- | --- | --- | --- | --- |
| WP10 | `a3s_sync::{unzip, cleanup, report, pipeline}` | `J/dao/zip/UnZipFlowProcessor`, `ZipDAO.unZip`, `ConnectionService.unZip/deleteExtraLocalFiles`, `FilesSynchronizationReportManager`, `J/dao/FileAccessMethods`, `J/ui/repository/workers/AddonsChecker`, `AddonsDownloader`, `AddonsAutoUpdater`, `UserconfigUpdater` | WP08, WP09 | Port `T/dao/FileAccessMethodsTest`. End-to-end sync against local HTTP; second run downloads 0; userconfig copy; revision + SHA-1 cache persisted. |
| WP21 | `a3s_viewer_context` | `J/ui/Facade`, `J/controller/*` (→ event enum), `J/ui/main/tasks/TasksManager`, `TaskCheckRepositories`, `workers/SwingUi` | WP14, WP07 | AppState, tokio command runner → channel → repaint, cancel tokens, periodic checker with fake clock. Pre-split modules per domain. |

## Wave 4

| WP | Crate(s) | Java anchors | Deps | Notes |
| --- | --- | --- | --- | --- |
| WP19 | `arma3sync-cli`, `arma3sync` | `J/main/ArmA3Sync` (legacy flags), `J/console/CommandConsole` (756), `CommandGeneral`, `CommandLine`, `J/constant/ConsoleCommands`, `J/service/UpdaterProcess` | WP10, WP11, WP12, WP14, WP17, (WP32) | Legacy flags (`-sync/-build/-check/-extract/-update/-console/-debug`) map to subcommands; interactive console scripted via stdin; non-zero exit codes. |
| WP22 | `a3s_viewer` shell | `J/ui/main/MainPanel` (1691), `InfoPanel`, `LaunchPanel`, `dialogs/WelcomeDialog`, `J/ui/ErrorLogDialog`, `repository/dialogs/error/*`, `J/ui/help/About*` | WP20, WP21, WP15, WP16 | **Serial.** eframe app, menus, tray, window size, launch button, first-run legacy import prompt, one stub module per later panel already registered, kittest helpers. |

## Wave 5 (egui-implementer, after WP22)

| WP | Panel module | Java anchors | Deps |
| --- | --- | --- | --- |
| WP23 | `panels/addons` | `J/ui/main/AddonsPanel` (1938, may split), `main/tree/*`, `TreeDnD2`, `J/ui/groups/*` | WP22, WP13, WP18 |
| WP24 | `panels/{addon_options, launcher_options, external_apps, online}`, `dialogs/profiles` | `LauncherOptionsPanel` (1195), `AddonOptionsPanel`, `ExternalApplicationsPanel` + dialog, `OnlinePanel`, `J/ui/profiles/*` | WP22, WP14, WP16 |
| WP25 | `panels/sync`, repository tab container | `J/ui/main/SyncPanel`, `J/ui/repository/RepositoryPanel`, `dialogs/InfoUpdatedRepositoryDialog` | WP22, WP21, WP07, WP14 |
| WP29 | `dialogs/*` | `repository/dialogs/connection/*`, `J/ui/help/PreferencesDialog`, `ProxyConfigurationDialog`, `AutoConfigImport/ExportDialog`, `progress/ProgressTestPartiaFileTransfer` | WP22, WP14, WP05, WP15 |
| WP30 | `tools/*` | `J/ui/tools/WizardDialog`, `tfar/*`, `acre2/*`, `bikey/*`, `rpt/RptViewerPanel` | WP22, WP17, WP15 |

## Wave 6

| WP | Module | Java anchors | Deps |
| --- | --- | --- | --- |
| WP26 | `repository/download` | `J/ui/repository/DownloadPanel` (1555), `repository/tree/*`, `DownloadSettingsDialog`, `ProgressSynchronizationDialog`, `ReportDialog`, `ConnectionLostDialog`, `ChangelogPanel`, `ModdsetsSelectionDialog` | WP25, WP10 |
| WP27 | `repository/admin` | `J/ui/repository/AdminPanel` (973), `BuildRepositoryOptionsDialog` (655), `UploadRepositoryConnectionDialog`, `ProgressConnectionAsAdminDialog`, workers `RepositoryBuilder/Checker/Uploader` | WP25, WP11, WP12 |
| WP28 | `repository/events` | `J/ui/repository/EventsPanel` (1138), `repository/events/*`, `UploadEventsConnectionDialog`, `ProgressUploadEventsDialog` | WP25, WP12 |

## Wave 7

| WP | Scope | References | Deps |
| --- | --- | --- | --- |
| WP33 | `.github/workflows/release.yml`, `packaging/**`, `scripts/**` | `installer/nsis/*`, `release/*.ps1`, `release-components.json`, `packaging/winget/*`, `.github/workflows/publish-winget.yml`, `installer/assets/*` | WP19, WP22, WP32 |

Followed by an end-to-end parity pass (CLI + GUI vs. Java behaviour).

## Critical paths

- WP03 → WP07 → WP08/WP09 → WP10 → WP19 / WP26
- WP14 → WP21 → WP22 → WP25 → WP26
