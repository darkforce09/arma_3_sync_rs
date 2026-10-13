# Arma3Sync architecture

This document describes the structure of the Rust rewrite. The layout follows
[rerun](https://github.com/rerun-io/rerun): a single Cargo workspace whose crates live in
`crates/<group>/<crate>`, with dependency versions and lints defined once in the root `Cargo.toml`.

## Goals

1. **Repository compatibility.** Anything Arma3Sync puts on a repository server (the `.a3s/` folder,
   `.zsync` files, `.pbo.zip` files) must be byte-compatible with the Java implementation, in both
   directions.
2. **Feature parity** with the Java client: synchronization, repository administration, launcher,
   profiles, addon groups, events, tools.
3. **A native egui UI**, single binary, Linux / Windows / macOS.

## Crate groups

A crate may depend on crates from its own group or a group listed above it in this table.

| Crate | Purpose | Java counterpart |
| --- | --- | --- |
| **foundation** | | |
| `a3s_foundation` | Error taxonomy and chain formatting, display formatting, logging, application directories, Windows command-line and PowerShell quoting. | `exception/*`, `utils/*`, `DebugLogger`, `dao/ApplicationPaths` |
| **store** | | |
| `a3s_jserial` | Generic Java Object Serialization Stream Protocol reader/writer. | JDK `ObjectInput/OutputStream` |
| `a3s_types` | Domain types + their exact Java class descriptors. | `domain/*`, `constant/*` |
| `a3s_repo_files` | `.a3s/*` gzip files, local TOML store and the in-memory client data over it (configuration, preferences, profiles, repositories), legacy Java data importer, parallel SHA-1 with mtime cache. | `dao/A3SFilesAccessor`, `dao/*DAO`, `dao/repository/RepositorySHA1Processor`, `service/{Configuration,Preferences,Profile,Common}Service`, repository CRUD of `service/RepositoryService` |
| `a3s_zsync` | jazsync-compatible `.zsync` writer, reader and patcher. | `jazsync/*`, `jazsyncmake/*` |
| **net** | | |
| `a3s_transport` | HTTP(S), FTP(S), SFTP, WebDAV; proxies, timeouts, throttling, cancel. | `dao/connection/*` |
| `a3s_sync` | Client check → plan → download. | `service/synchronization/*`, `service/connection/*` |
| `a3s_admin` | Repository build, content check, upload and events editing. | `service/administration/*`, `dao/repository/RepositoryBuildProcessor`, `service/connection/Connection{Check,Upload,Delete}Processor` |
| `a3s_updater` | Self-updater: `updater.toml`, release lookup (GitHub, `a3s.json`, legacy `a3s.xml`), verified download, safe archive installation, self-replacement of the running binary, UAC elevation, console update flows. | `updater/` (`fr.soe.a3sUpdater`), `service/UpdaterProcess` |
| **game** | | |
| `a3s_addons` | Addon discovery and registry (mods and CDLCs under their symbolic keys), launch priorities, modset groups from repositories and events, missing/duplicate checks on addon groups. | `service/AddonService`, `dao/AddonDAO`, addon parts of `ProfileService`, `RepositoryService`, `ui/main/AddonsPanel` |
| `a3s_platform` | OS integration: registry, Arma 3 / Steam / TeamSpeak 3 detection, special folders, process probe, autostart, desktop shortcuts, single instance. | `dao/ConfigurationDAO` (registry), `dao/WinRegistry`, `dao/PreferencesDAO` (Run key), `dao/LauncherDAO.isApplicationRunning`, JUnique and `JShellLink` use |
| `a3s_launch` | Arma 3 run parameters, `Arma3.cfg` mod-list clean-up, the launch command for each OS (BattlEye, UAC elevation, `.bat`/`.sh`, Steam/Proton), the launch thread with auto-restart, external applications. The one place that spawns the game. | `service/LaunchService`, `dao/LauncherDAO` (launching), run-parameter parts of `ui/main/LauncherOptionsPanel` and `ui/main/MainPanel` |
| `a3s_tools` | Bikey extraction, RPT log tailing, TFAR and ACRE 2 installer logic, Arma 3 Launcher preset import and export. | `dao/CommonDAO`, `ui/tools/*`, `service/Arma3LauncherPreset{Importer,Exporter}` |
| **viewer** | | |
| `a3s_ui` | Theme tokens (port of `ui/theme`), Geist fonts, Phosphor icon font, widgets. | `ui/theme`, `ui/components` |
| `a3s_viewer_context` | Application state, command bus, event channel, background tasks with cancellation and coalesced progress, periodic repository check and automatic updates. No UI code. | `ui/Facade`, `controller/*`, `ui/main/tasks/*`, background parts of `ui/main/MainPanel` and `ui/repository/dialogs/progress/ProgressSynchronizationDialog` |
| `a3s_viewer` | The desktop window: eframe application, unified toolbar with the Play button, source-list sidebar, screens, sheets, toasts, Settings and tool windows, tray icon, single instance. | `ui/main/MainPanel`, `ui/main/LaunchPanel`, `ui/main/dialogs/WelcomeDialog`, window start of `main/ArmA3Sync` |
| **application** | | |
| `arma3sync` | The `arma3sync` binary: command line, interactive console, window start, build information. | `main/ArmA3Sync`, `console/*`, `version.properties` |
| **development** | | |
| `a3s_dev_tools` | Java fixture harness wrapper: fixture regeneration, Java cross-checks. Dev-dependency only. | – |
| `a3s_test_servers` | In-process FTP(S), SFTP, WebDAV, TCP relay and SOCKS servers for the network crates' tests. Dev-dependency only. | – |

## Repository wire format

Every metadata file in a repository's `.a3s/` folder is a **gzip-compressed Java serialization
stream** containing exactly one object. There is no version field; compatibility depends entirely on
fully-qualified class names, `serialVersionUID`s and field descriptors.

| File | Root class (`fr.soe.a3s.…`) |
| --- | --- |
| `.a3s/sync` | `domain.repository.SyncTreeDirectory` (root named `racine`) |
| `.a3s/serverinfo` | `domain.repository.ServerInfo` |
| `.a3s/changelogs` | `domain.repository.Changelogs` |
| `.a3s/autoconfig` | `domain.repository.AutoConfig` |
| `.a3s/events` | `domain.repository.Events` |

Class descriptors (fields in Java wire order: primitives sorted by name, then objects sorted by name):

| Class | serialVersionUID | Fields |
| --- | --- | --- |
| `domain.repository.SyncTreeDirectory` | `-2855304993780573704` | `Z deleted, Z hidden, Z markAsAddon, Z updated, List list, String name, SyncTreeDirectory parent` |
| `domain.repository.SyncTreeLeaf` | `8849248143660225239` | `Z compressed, J compressedSize, Z deleted, J size, Z updated, String name, SyncTreeDirectory parent, String sha1` |
| `domain.repository.ServerInfo` | `7697232677958952953` | `Z compressedPboFilesOnly, Z noPartialFileTransfer, I numberOfConnections, J numberOfFiles, Z repositoryContentUpdated, I revision, J totalFilesSize, Date buildDate, Set hiddenFolderPaths` |
| `domain.repository.Changelogs` | `7332983755625818660` | `List list` |
| `domain.repository.Changelog` | `749128678979864247` | `Z contentUpdated, I revision, List addons, Date buildDate, List deletedAddons, List newAddons, List updatedAddons` |
| `domain.repository.Events` | `-5141643688299352462` | `List list` |
| `domain.repository.Event` | `7456226002765813117` | `Map addonNames, String description, Map dlcNames, String name, Map userconfigFolderNames` |
| `domain.repository.AutoConfig` | `2706280725619197755` | `List favoriteServers, AbstractProtocole protocole, String repositoryName` |
| `domain.AbstractProtocole` | `-886357032287815203` | `Z validateSSLCertificate, String connectionTimeOut, EncryptionMode encryptionMode, String login, String password, String port, ProtocolType protocolType, String readTimeOut, String url` |
| `domain.Http` / `Ftp` / `Webdav` / `Socks` / `Sftp` | `-20` / `-1917448655850030044` / `-30` / `-40` / `1` | (no own fields) |
| `domain.configration.FavoriteServer` | `4613236292596702132` | `I port, Z selected, String ipAddress, String modsetName, String name, String password, String repositoryName` |

Enums: `constant.ProtocolType { FTP, FTPS, SFTP, HTTP, HTTPS, A3S, SOCKS4, SOCKS5, HTTP_WEBDAV,
HTTPS_WEBDAV }`, `constant.EncryptionMode { NO_ENCRYPTION, EXPLICIT_SSL, IMPLICIT_SSL }`.

JDK classes appearing in streams: `ArrayList`, `HashMap`, `HashSet`, `Date`, `Boolean`.

Limits enforced by Java readers (so a Rust writer must stay inside them): nesting depth ≤ 100,
≤ 250 000 references, ≤ 256 MB stream.

### Other repository content

* Files live at `<root>/<relative path>`; directories are `/`-joined names below `racine`.
* A folder named `racine` below the root resets the remote path, as in Java: `@a/racine/x.pbo` is
  stored and downloaded as `<root>/x.pbo`, while clients keep it at `@a/racine/x.pbo` locally.
* `<file>.zsync` exists for HTTP repositories: jazsync header (`zsync: jazsync`, `Blocksize: 8192`,
  `Hash-Lengths`, `SHA-1`, …) followed by per-block rsum + truncated MD4.
* `<file>.pbo.zip` / `.ebo.zip` exists when the repository is compressed: one DEFLATE-9 entry.
* File SHA-1 is lowercase hex; **an empty file hashes to the literal `"0"`**.

### HTTP client behaviour

`User-Agent: ArmA3Sync`, `Accept-Encoding: identity`, `Cache-Control: no-cache, no-store, max-age=0`,
`Pragma: no-cache`, preemptive Basic auth unless the login is `anonymous`. Partial transfer support is
probed with `Range: bytes=0-1` on `.a3s/sync` (must answer 206).

## Local client data

The Rust client stores its own configuration as TOML in the platform config directory
(`a3s_foundation::paths`). On first start it offers a one-time import of the Java client's data
(`a3s.cfg`, `a3s.prefs`, `*.a3s.profile`, AES-encrypted `*.a3s.repository`).

## Intentional differences from Java

Wire compatibility is the requirement; these differences are deliberate.

* **Compatibility levels.** `.a3s/*` files and `.zsync` files are byte-identical to Java's;
  `.pbo.zip`/`.ebo.zip` are semantically equal (same entry and content, not the same bytes).
* **Local client data** is a new TOML store in its own directory, `arma3sync-rs`, which never
  collides with the Java client's; the Java client's files are imported once on request.
* **SFTP** accepts any host key and logs its fingerprint, like Java's MINA default.
  `deny.toml` ignores RUSTSEC-2023-0071 (RSA timing) because only public-key verification is used;
  re-evaluate if client public-key authentication is added.
* **Repository upload** differences are local only: a file below a nested `racine` folder is
  read from its real local path (Java reads `<folder>/<remote path>`, which is missing or another
  file); a failed upload resumes by remote path for the same target, an unchanged local `.a3s/sync` and
  unchanged files (Java resumes at an index that counts failed files as done); parallel workers
  share one queue and the first failure stops them all; the metadata upload is not interrupted by
  a cancel once it started; nothing is deleted below a directory that is deleted, and the
  top-level `.a3s` is never deleted; the content check keeps one connection per worker. Files are
  overwritten in place (as in Java), so a stop leaves a partial file under the old metadata until
  the next run re-uploads it. The uploaded `.a3s` files are byte-identical to Java's (the JDK 25
  round trip of the local files).
* **Events editing** treats a null `addonNames` in an old events file as empty (Java throws), and
  renaming an event may keep its own name (Java reports a duplicate).
* **Repository build:** the compression stage's own outputs (`<name>.zip` for any compressible
  file, so also `.ebo.zip`) are never sync leaves (Java turns its `.ebo.zip` into a leaf on the
  next build); a repository whose path is a symbolic link reports the real `totalFilesSize` of the
  linked folder (Java reports 0). `.zip` and `.zsync` files are written to
  `<name>.<random>.a3s.tmp` (umask permissions, a name every builder skips) and renamed, where
  Java writes them in place. The `.a3s` metadata files get umask permissions too (Java leaves
  them `0600`, unreadable to a web server running as another user). A cancelled build stops
  and writes no `.a3s` file (Java writes partial metadata), and a re-hashed file enters the SHA-1
  cache only once its `.zip`/`.zsync` is regenerated or deleted, so the next build never reuses
  stale ones.
* **Sync safety:** the plan rejects remote names that are empty, `.`, `..`, absolute, contain a
  separator, end with `.` or a space, or are Windows device names (`CON`, `NUL`, `COM1`, ...).
  Downloads and deletions stay inside the destination or an addon folder that auto-discovery found
  for the repository's addon of the same name; the `userconfig` copy writes only below
  `<Arma 3 folder>/<userconfig folder name>`. An extra local symbolic link is never
  followed (it is one file node); deletions remove the link, never its target (a Windows directory
  link or junction with `remove_dir`), and are checked again against the destination and the
  auto-discovered addon folders before anything is removed. The caller chooses whether extra
  files go to the system Trash (the UI) or are deleted permanently like Java (the headless
  console); a link is unlinked either way. Downloads, and the extra-file scan of a
  repository folder, do follow links the user created inside the destination. Archives a run still
  needs (`.zip`, `.part`) are never deleted as extra files. Java has none of these checks.
* **Uncompressing:** every compressed leaf is uncompressed, `.ebo.zip` included (Java only
  `.pbo.zip`). Only the first entry is used and its name ignored; link and directory entries are
  refused, the extracted size and SHA-1 must match the leaf, and the old file is replaced only once
  the new one is complete. A leaf patched with zsync is never overwritten by a stale archive.
* **Synchronization state:** a check persists the repository and configuration changes at once; a
  failed synchronization persists the revision -1 at once (Java only in memory); a cancelled one
  leaves the revision alone (Java re-checks and takes the server revision); the headless
  synchronization updates the revision for the command line too. A silent `userconfig` copy with a
  missing source does nothing; the copy skips symbolic links in the source and never writes
  through a symbolic link in the Arma 3 folder (Java follows both). Pausing or cancelling also
  stops uncompressing, and a run returns only once its extractions stopped; an archive that fails
  to uncompress is downloaded again by the next run. Error reports list messages, never stack
  traces. `ConnectionLost` is the last event of a download run.
* **Autoconfig export and import** (`export.a3s.autoconfig`): an export that Java's own reader
  would reject (a profile tree nested deeper than about 47 groups, or over the reference or size
  limits) is refused instead of written. On import, a profile or repository element that is
  invalid (null name, missing protocol, wrong class) is skipped with a warning; Java fails part
  way through the import.
* **Client data** (`a3s_repo_files::services`): every change is written at once and a failed
  write leaves memory unchanged (Java keeps many changes in memory until exit and swallows write
  errors). Names are checked: an existing or blank profile or repository name is refused on
  create, duplicate and rename, `"Default"` cannot be removed or renamed, removing or renaming
  the active profile keeps a valid active profile, and an unknown repository is an error instead
  of a no-op. Preferences are loaded at start-up (Java never reads them and overwrites them on
  exit). A duplicated profile copies its own splash-screen setting (Java copies `noPause`), a
  renamed profile's old file is deleted (Java deletes the new name's file), and repositories are
  listed in name order. Autoconfig export does not clear the launch settings of the live
  profiles, and import never replaces an existing repository (Java overwrites its file with the
  bare imported one). A name held by a store file that cannot be read stays taken.
* **Proxy** is applied per connection with Java's effective routing, but its credentials go to
  the proxy only (Java's default `Authenticator` also answered the repository server), a
  SOCKS4 proxy speaks SOCKS4 (Java's default made it SOCKS5), and a SOCKS proxy also carries
  SFTP connections (Java's MINA client bypassed it).
* **Transports:** an FTP `DELE` or `RMD` refused with 550 counts as "already gone" only once
  `MLST`, `CWD` or a usable listing confirms the target is missing (Java ignores every 550).
  In an FTP upload session a name the cached listing holds only in another case is confirmed
  with `MLST`, so a case-insensitive server answers as outside a session (Java's session
  answers "missing"). WebDAV maps 409 on `PUT`/`MKCOL` to a missing parent and a 207 answer to `DELETE` to an
  error (Java maps only 401 and 404, and takes 207 as success).
* **Platform integration** (local only): start-with-Windows writes the per-user `Run` value
  `arma3sync-rs` (Java: machine-wide `ArmA3Sync`, which needs admin rights and is left alone)
  pointing at the real executable (Java: the working folder); Linux and macOS get XDG and
  LaunchAgent entries. Shortcut and `Run` arguments are quoted by Windows rules (Java did not
  quote), shortcut names are validated and the target must exist. Registry strings are read as
  UTF-16 and `REG_EXPAND_SZ` is accepted; blank values count as missing; TeamSpeak 3 detection
  also checks the all-users install. "Is it running" matches whole image names from one CSV
  `tasklist` (Java: substring of truncated names), and Linux scans `/proc`. Arma 3, its RPT and
  user folders are also found through Steam libraries and Proton prefixes. Single instance
  applies on every OS, per configuration folder (Java: Windows only).
* **Game launch** (local only; the parameters Arma 3 receives are Java's): `.sh` scripts run as
  `/bin/bash <script> <args…>` (Java's `bash -c` dropped the arguments); every enabled external
  application starts (Java started only the first, then never launched the game); executable
  kinds come from the real extension, ignoring case; `.bat` files start through
  `cmd /C start ""`, so a quoted path is not taken as the window title. Elevated launches quote
  each argument for the Windows command line, leave out an empty `-ArgumentList`, escape curly
  single quotes, open no console and report "started" when PowerShell starts (Java: when the game
  had exited). Auto-restart can be stopped and gives up after 3 runs in a row that each last
  under 10 s. The child writes to Arma3Sync's own output instead of a pipe and runs in its
  executable's folder; `-password=` values are hidden in the log; a favourite server or external
  application with a missing name, address, password, path or parameters is skipped or treated
  as empty instead of failing. `Arma3.cfg` keeps every byte outside the removed list and is
  cleaned on every OS where its folder is known (Java: Windows only). On Linux a Windows `.exe`
  starts through Steam (`steam -applaunch 107410 -noLauncher …`) with mod and mission paths in
  Wine `Z:` form; auto-restart then follows the game in the process list. The viewer starts the
  game even when selected addons are missing or updates before playing failed (it says so in a
  toast; Java refused to launch), and "Update and play" downloads the repositories of the enabled
  addon groups first.
* **Updater:** `updater.toml` keys, defaults and dev fallbacks, the `a3s.json`, GitHub and
  `a3s.xml` metadata fields and `.a3s-updater-files` stay Java-compatible; the JVM properties
  `-Da3s.updater.*` become `A3S_UPDATER_*` environment variables. Differences: the default GitHub
  asset is `Arma3Sync-{version}-{target}.zip` (`{target}` = Rust target triple) and `a3s.json`
  may carry a per-target `targets` table; a legacy XML without `<file>` derives the archive name
  from that pattern. Releases without SHA-256 (legacy XML) are not downloaded unless the caller
  allows it; a download larger or smaller than the announced `size`, or above 4 GiB, fails, and
  it is written to `<name>.part` and renamed only after its hash matched. A remote XML without
  `<nom>` falls through to the installation's `a3s.xml` (Java stops). The archive is hashed and
  validated from one private copy that staging also reads. Archives are rejected for
  symbolic-link entries; absolute, drive, device, `:`, trailing-dot or trailing-space names;
  paths that collide ignoring case or as file and directory; more than 65 536 entries or more
  than 4 GiB. Unix permission bits are kept as `(mode & 0o755) | 0o600`. Installation checks
  every target before writing: a stale managed file standing where a directory is needed is
  deleted first, any other conflict fails. Installed files are written to a temporary sibling
  and renamed over the target (Java copies in place); a stale managed path that is the same file
  as an installed one (case-insensitive file systems) is kept. The portable-mode marker and data
  folder of this application are user data too. TOML is parsed strictly by the `toml` crate;
  user-data prefixes match case-insensitively; the legacy FTP update transport is dropped; the
  diagnostic log defaults to `<data dir>/logs/arma3sync-updater-debug.log` of the Rust
  application.
* **Applying updates:** exit codes (check: 0 available, 2 none, 1 error; update: 0, 2, 1; legacy
  `-update` always 0) and the console lines stay Java's. There is no separate updater program:
  the application updates itself and replaces its running binary (rename over it on Unix, rename
  it away to `.a3s-old-*` on Windows). Leftovers of finished processes are removed (a renamed-away
  file whose target is missing is renamed back) by the next start that can write the
  installation; in a protected installation that is the next elevated update. With UAC the
  unelevated application downloads and verifies the archive, then starts its own binary elevated
  through PowerShell `-EncodedCommand` with a hidden `__a3s-elevated-install` subcommand that
  receives only the archive path, the requested version, the channel and a nonce. The child
  resolves the release itself through the installation's own `updater.toml` (no user
  configuration, no environment overrides), requires it to be the requested version and newer
  than its own, verifies the archive against the SHA-256 it resolved (a release without one is
  never installed elevated), installs into its own directory, and writes its marker, result and
  log into a `.a3s-elevated-<pid>-<nonce>` directory it creates there. The parent waits up to 10
  minutes and relaunches the application unelevated (Java hands the whole update to an elevated
  copy of the JRE and updater, forwards its `a3s.updater.*` properties and relaunches elevated).
  An in-process install that fails for lack of permissions is retried elevated only when the
  installation directory is no longer writable. A
  release archive must hold `arma3sync[.exe]` at its root and no portable user data; AppImage,
  macOS bundle, Java and cargo-build installations are refused. An unknown archive size proceeds
  with indeterminate progress; Linux and macOS fail before the download when the installation is
  not writable; unknown updater flags exit 1 in every mode; console progress prints once per
  percent; Windows argument quoting follows the MSVC rules (Java's doubled the backslashes before
  an embedded quote twice).
* **Viewer background work** (local only): at most 4 repository checks run at once in the whole
  process and a repository is never checked twice at once (Java's manual check uses one thread
  per processor and can overlap the periodic one); cancelling a manual check stops every
  connection (Java only the last one). Changing the check frequency reschedules only when it
  actually changed, the "updates available" memory survives a reschedule, the periodic check
  starts independently of the self-update check and starts again when the user stays after a
  failed save on exit; a periodic check interrupted by exit skips its follow-up. Automatic
  updates move extra local files to the Trash. Progress events are coalesced per task, and an
  event emitted on the UI thread is seen at the next poll (Java runs it inline).
* **Window** (local only): closing the main window hides it into the tray when the
  "minimized" preference is Tray (Java always quits on close); `-run` starts hidden in the tray,
  or shown when the desktop has no tray. Quitting asks first only while downloads, builds or
  uploads run. The theme follows the system appearance.
* **Insecure TLS override** is the environment variable `A3S_ALLOW_INSECURE_SSL=1` instead of
  Java's `-Da3s.allowInsecureSsl=true`.
* **Game tools:** each bikey extraction has its own cancel token (Java's flag is never reset, so
  every later extraction finds nothing), cancellation also stops the copies, a key that already is
  its own target is not copied onto itself (Java truncates it to 0 bytes), the walk is sorted by
  name and survives symbolic-link cycles, and a target that is a file is "not writable". The RPT
  tail stops and joins its thread (Java's never stops), continues exactly where the preload ended,
  holds back a partial last line until it ends or the file stops growing, decodes lines as UTF-8,
  waits for a missing file, detects rotation by file identity as well as shrinkage, and reports
  read errors once instead of swallowing them. Installer paths join with the platform separator
  and an empty field never resolves against the filesystem root.
* **Addon discovery** lists each folder in name order (ignoring case, ties in byte order) where
  Java takes the filesystem's order, which decides which of two same-named mods keeps the plain
  key; a lookup that several addons match (a duplicate inside a download location, a Workshop id,
  a DLC) takes the first in scan order instead of Java's hash order; a symbolic-link loop is
  scanned once instead of recursing until the stack overflows.
* **Arma 3 Launcher presets:** the exported HTML is byte-identical to Java's. On import, a numeric
  entity decodes to its real code point (Java truncates it to 16 bits), two adjacent surrogate
  entities form one character, a lone surrogate becomes U+FFFD, and an entity that is not a valid
  number stays as text (Java aborts the import); an unreadable local `meta.cpp` skips that addon
  with a warning (Java aborts). The HTML patterns' case-insensitive keywords also accept the
  Unicode case variants `ſ` and `K` (Kelvin sign). Java's `launcher-preset-export.log` is replaced by
  `tracing` debug events in the application log.
* **Command line:** failures exit with 1 and invalid arguments with 2 (Java always exits 0,
  `-update` included; `update` returns the updater's 0/1/2), and messages about failures go to
  stderr. The console reads its input to the end (Java fails at the end of the input), does
  not print passwords, offers every download protocol, and fixes two Java slips: answering
  "no" to the SSL question turns the validation off (Java turned it on either way), and the
  "exclude extra local content when sync" folders become excluded sync folders (Java added them
  to the files excluded from the build). Connections use the configured proxy (Java's console
  ignores it), and a synchronization resumes at most 10 times after a lost connection (Java
  retries forever). Update leftovers are cleaned at every start; command-line modes never take
  the single-instance lock, as in Java.
* **Local-only fixes** (no wire effect) are allowed where Java has a bug or a security hole, for
  example control characters rejected in local file names, and UI tokens adjusted for readability
  in the dark theme. UI and error text need not match Java.

## Testing strategy

* Golden fixtures in `tests/fixtures/` are produced by the Java implementation through
  `tools/java_fixture_harness`. Rust must decode them, and re-encode them byte-identically.
* Live, read-only tests against a public repository are `#[ignore]`d and run with
  `cargo test -- --ignored`.
