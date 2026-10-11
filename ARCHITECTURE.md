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

## Intentional differences from Java

Wire compatibility is the requirement; these differences are deliberate.

* **Compatibility levels.** `.a3s/*` files and `.zsync` files are byte-identical to Java's;
  `.pbo.zip`/`.ebo.zip` are semantically equal (same entry and content, not the same bytes).
* **Local client data** is a new TOML store in its own directory, `arma3sync-rs`, which never
  collides with the Java client's; the Java client's files are imported once on request.
* **SFTP** accepts any host key and logs its fingerprint, like Java's MINA default.
  `deny.toml` ignores RUSTSEC-2023-0071 (RSA timing) because only public-key verification is used;
  re-evaluate if client public-key authentication is added.
* **Repository upload** writes nested `racine` folders to their real paths (Java uploads them to
  wrong paths).
* **Repository build:** the compression stage's own outputs (`<name>.zip` for any compressible
  file, so also `.ebo.zip`) are never sync leaves (Java turns its `.ebo.zip` into a leaf on the
  next build); a repository whose path is a symbolic link reports the real `totalFilesSize` of the
  linked folder (Java reports 0).
* **Sync safety:** the plan rejects remote names that are empty, `.`, `..`, absolute or contain a
  separator, and every write and delete stays inside the destination. The extra-file scan never
  follows a symbolic link (it is one file node); deletions remove the link, never its target.
  Downloads do follow links the user created inside the destination. Java has none of these checks.
* **Local-only fixes** (no wire effect) are allowed where Java has a bug or a security hole, for
  example control characters rejected in local file names, and UI tokens adjusted for readability
  in the dark theme. UI and error text need not match Java.

## Testing strategy

* Golden fixtures in `tests/fixtures/` are produced by the Java implementation through
  `tools/java_fixture_harness`. Rust must decode them, and re-encode them byte-identically.
* Live, read-only tests against a public repository are `#[ignore]`d and run with
  `cargo test -- --ignored`.
