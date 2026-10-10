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

Dependencies only point "down" this list (a crate may depend on crates from its own group or a
group listed below it):

```
top     ─┐  arma3sync, arma3sync-cli, a3s_updater
viewer  ─┤  a3s_viewer, a3s_viewer_context, a3s_ui
game    ─┤  a3s_addons, a3s_launch, a3s_tools, a3s_platform
net     ─┤  a3s_sync, a3s_admin, a3s_transport
store   ─┤  a3s_types, a3s_jserial, a3s_repo_files, a3s_zsync, a3s_hash
utils   ─┤  a3s_log, a3s_paths, a3s_format, a3s_error
build   ─┘  a3s_build_info, a3s_dev_tools
```

| Crate | Purpose | Java counterpart |
| --- | --- | --- |
| **build** | | |
| `a3s_build_info` | Version, git hash, target triple baked into binaries. | `version.properties` |
| `a3s_dev_tools` | Fixture regeneration, repo checks. | – |
| **utils** | | |
| `a3s_log` | `tracing` setup, log files. | `DebugLogger` |
| `a3s_paths` | Config/data/cache dirs, portable mode, legacy-path discovery. | `dao/ApplicationPaths` |
| `a3s_format` | Sizes, speeds, durations. | `utils/*` |
| `a3s_error` | Error chain formatting. | `exception/*` |
| **store** | | |
| `a3s_jserial` | Generic Java Object Serialization Stream Protocol reader/writer. | JDK `ObjectInput/OutputStream` |
| `a3s_types` | Domain types + their exact Java class descriptors. | `domain/*`, `constant/*` |
| `a3s_repo_files` | `.a3s/*` gzip files, local TOML store, legacy Java data importer. | `dao/A3SFilesAccessor`, `dao/*DAO` |
| `a3s_zsync` | jazsync-compatible `.zsync` writer, reader and patcher. | `jazsync/*`, `jazsyncmake/*` |
| `a3s_hash` | Parallel SHA-1 with mtime cache. | `dao/repository/RepositorySHA1Processor` |
| **net** | | |
| `a3s_transport` | HTTP(S), FTP(S), SFTP, WebDAV; proxies, timeouts, throttling, cancel. | `dao/connection/*` |
| `a3s_sync` | Client check → plan → download → unzip → cleanup. | `service/synchronization/*`, `service/connection/*` |
| `a3s_admin` | Build / check / upload repositories. | `service/administration/*`, `dao/repository/RepositoryBuildProcessor` |
| **game** | | |
| `a3s_addons` | Addon discovery, groups, modsets, priorities, CDLC table. | `service/AddonService`, `constant/GameDLCs` |
| `a3s_launch` | Run parameters (`-mod=` …), process launch. | `service/LaunchService`, `dao/LauncherDAO` |
| `a3s_tools` | Bikey extractor, TFAR/ACRE2 installers, RPT tail, launcher HTML presets. | `ui/tools/*`, `service/Arma3LauncherPreset*` |
| `a3s_platform` | Registry, shortcuts, single instance, autostart. | `dao/WinRegistry`, junique, jshortcut |
| **viewer** | | |
| `a3s_ui` | Theme tokens (port of `ui/theme`), Geist fonts, SVG icons, widgets. | `ui/theme`, `ui/icon`, `ui/components` |
| `a3s_viewer_context` | App state, commands, background tasks ↔ UI channel. | `controller/*` observers, `ui/Facade` |
| `a3s_viewer` | eframe app: tabs, repository tabs, dialogs, tray. | `ui/main`, `ui/repository`, … |
| **top** | | |
| `arma3sync` | Facade crate, shared entry point. | `main/ArmA3Sync` |
| `arma3sync-cli` | The `arma3sync` binary (GUI by default, CLI subcommands). | `main/ArmA3Sync`, `console/*` |
| `a3s_updater` | Self-update from GitHub releases. | `updater/` subproject |

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
(`a3s_paths`). On first start it offers a one-time import of the Java client's data
(`a3s.cfg`, `a3s.prefs`, `*.a3s.profile`, AES-encrypted `*.a3s.repository`).

## Testing strategy

* Golden fixtures in `tests/fixtures/` are produced by the Java implementation through
  `tools/java_fixture_harness`. Rust must decode them, and re-encode them byte-identically.
* Live, read-only tests against a public repository are `#[ignore]`d and run with
  `cargo test -- --ignored`.

## Roadmap

0. Workspace scaffold, CI, docs.
1. Golden fixtures from the Java implementation.
2. `a3s_jserial`, `a3s_types`, `a3s_repo_files`.
3. `a3s_zsync`, `a3s_hash`.
4. `a3s_transport`, `a3s_sync`, CLI `sync` / `check`.
5. `a3s_admin`, CLI `build` / upload.
6. `game` crates.
7. `viewer` crates (egui UI).
8. Updater and packaging.
