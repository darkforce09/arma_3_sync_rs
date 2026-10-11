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
| `a3s_foundation` | Error taxonomy and chain formatting, display formatting, logging, application directories. | `exception/*`, `utils/*`, `DebugLogger`, `dao/ApplicationPaths` |
| **store** | | |
| `a3s_jserial` | Generic Java Object Serialization Stream Protocol reader/writer. | JDK `ObjectInput/OutputStream` |
| `a3s_types` | Domain types + their exact Java class descriptors. | `domain/*`, `constant/*` |
| `a3s_repo_files` | `.a3s/*` gzip files, local TOML store and the in-memory client data over it (configuration, preferences, profiles, repositories), legacy Java data importer, parallel SHA-1 with mtime cache. | `dao/A3SFilesAccessor`, `dao/*DAO`, `dao/repository/RepositorySHA1Processor`, `service/{Configuration,Preferences,Profile,Common}Service`, repository CRUD of `service/RepositoryService` |
| `a3s_zsync` | jazsync-compatible `.zsync` writer, reader and patcher. | `jazsync/*`, `jazsyncmake/*` |
| **net** | | |
| `a3s_transport` | HTTP(S), FTP(S), SFTP, WebDAV; proxies, timeouts, throttling, cancel. | `dao/connection/*` |
| `a3s_sync` | Client check → plan → download. | `service/synchronization/*`, `service/connection/*` |
| `a3s_admin` | Repository build (check and upload to come). | `service/administration/*`, `dao/repository/RepositoryBuildProcessor` |
| **game** | | |
| `a3s_addons` | Addon discovery and registry (mods and CDLCs under their symbolic keys), launch priorities, modset groups from repositories and events, missing/duplicate checks on addon groups. | `service/AddonService`, `dao/AddonDAO`, addon parts of `ProfileService`, `RepositoryService`, `ui/main/AddonsPanel` |
| `a3s_platform` | OS integration: registry, Arma 3 / Steam / TeamSpeak 3 detection, special folders, process probe, autostart, desktop shortcuts, single instance. | `dao/ConfigurationDAO` (registry), `dao/WinRegistry`, `dao/PreferencesDAO` (Run key), `dao/LauncherDAO.isApplicationRunning`, JUnique and `JShellLink` use |
| `a3s_tools` | Bikey extraction, RPT log tailing, TFAR and ACRE 2 installer logic, Arma 3 Launcher preset import and export. | `dao/CommonDAO`, `ui/tools/*`, `service/Arma3LauncherPreset{Importer,Exporter}` |
| **viewer** | | |
| `a3s_ui` | Theme tokens (port of `ui/theme`), Geist fonts, SVG icons, widgets. | `ui/theme`, `ui/icon`, `ui/components` |
| **application** | | |
| `arma3sync` | The `arma3sync` binary and its build information. | `main/ArmA3Sync`, `version.properties` |
| **development** | | |
| `a3s_dev_tools` | Java fixture harness wrapper: fixture regeneration, Java cross-checks. Dev-dependency only. | – |

Planned parts (launch, the viewer application, the updater)
get a crate when their ttm ticket starts, inside the group that fits.

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
* **Repository upload** writes nested `racine` folders to their real paths (Java uploads them to
  wrong paths).
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
  auto-discovered addon folders before anything is removed. Downloads, and the extra-file scan of a
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
  the proxy only (Java's default `Authenticator` also answered the repository server) and a
  SOCKS4 proxy speaks SOCKS4 (Java's default made it SOCKS5).
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
* **Local-only fixes** (no wire effect) are allowed where Java has a bug or a security hole, for
  example control characters rejected in local file names, and UI tokens adjusted for readability
  in the dark theme. UI and error text need not match Java.

## Testing strategy

* Golden fixtures in `tests/fixtures/` are produced by the Java implementation through
  `tools/java_fixture_harness`. Rust must decode them, and re-encode them byte-identically.
* Live, read-only tests against a public repository are `#[ignore]`d and run with
  `cargo test -- --ignored`.
