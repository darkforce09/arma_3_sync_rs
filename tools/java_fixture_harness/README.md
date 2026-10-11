# Java fixture harness

Runs the real Arma3Sync Java code (`java_version/Arma3Sync`) to produce the golden fixtures in
`tests/fixtures/` and to cross-check files written by the Rust implementation. Nothing is ported:
the harness compiles the minimal set of Arma3Sync sources with plain `javac` and calls them.

Use it through the Rust wrapper (`crates/build/a3s_dev_tools`):

```sh
source scripts/java_env.sh
cargo run -p a3s_dev_tools -- regen-fixtures --out .workstation/fixtures   # generate only
cargo run -p a3s_dev_tools -- regen-fixtures --write-fixtures                 # install into tests/fixtures
cargo run -p a3s_dev_tools -- regen-fixtures --check
cargo run -p a3s_dev_tools -- java-verify <files or dirs>...
```

## Requirements

- JDK 25 from `scripts/java_env.sh` (override with `A3S_JAVA_HOME`). No Gradle, no downloads.
- The Java sources: `$A3S_JAVA_SRC`, else `<repo>/java_version/Arma3Sync/src/main/java`, else the
  same path next to the main checkout (`git rev-parse --git-common-dir`), so linked worktrees work.
- `bash`, `perl`, `flock`, `sha256sum`.

## Scripts

| Script | Purpose | Exit codes |
| --- | --- | --- |
| `build.sh [--clean]` | Compiles into `build/` (incremental: prints `up to date` when the stamp matches) | 0, 3 compile/patch, 4 no JDK/sources |
| `run.sh <command> [args]` | Builds if needed, then runs `harness.Main` with a pinned time zone (UTC), locale (en_US) and encoding (UTF-8) | 0, 1 verification failure, 2 usage, 3 internal, 4 no JDK/sources |

stdout carries only protocol lines and JSON; every other message (javac, Arma3Sync's own
`System.out` progress, child JVMs) goes to stderr.

## Commands

| Command | Output |
| --- | --- |
| `descriptors` | Descriptor dump of every serializable `domain`/`constant` class plus the JDK classes found in streams |
| `generate --out <dir> [--only descriptors,local,built,presets,roundtrip] [--work <dir>] [--keep-work]` | The fixture tree (below); `<dir>` must be empty or missing; `--work` and `--out` must not contain each other |
| `verify [--dump-dir <dir>] [--semantic] <path>...` | One `OK`/`FAIL`/`SKIP` line per file, then `SUMMARY ok=… fail=… skip=…` (tab separated); exit 1 on failures |
| `dump [--semantic] <file>` | Canonical JSON of the object graph |
| `roundtrip <in> <out>` | `A3SFilesAccessor.read` then the upload serialization (`FtpDAO.java:584-588`) |
| `selftest` | Port of `A3SFilesAccessorTest` |

`verify` reads each file with the reader Arma3Sync uses: `.a3s/{sync,serverinfo,changelogs,autoconfig,events}`
with `A3SFilesAccessor.read` (object filter included), `*.a3s.repository` with the AES cipher,
`*.repository.gz` (harness plain copies), `*.a3s.profile`, `a3s.cfg`, `a3s.prefs` and
`*.a3s.autoconfig` (`CommonDAO.importAutoConfig`). `*.pbo.zip`/`*.ebo.zip` are unzipped with
`ZipDAO.unZip` (on a copy) and compared with the sibling source; `*.zsync` files are regenerated
with jazsyncmake from the sibling source and the file's `URL:` line and compared byte for byte.

## Fixture tree

| Entry | Content |
| --- | --- |
| `java_descriptors.txt` | UID (`declared`/`computed`), wire flags, super class, fields in wire order, enum constants, transient fields |
| `java_local/` | Per-user and portable layouts written by the real DAOs, plain repository copies, 4 historical variants, the autoconfig export, identity dumps in `dumps/` |
| `java_built/<variant>/` | `http_plain`, `http_zip`, `ftp_plain`, `ftp_zip`: steps `1_initial`, `2_rebuild_nochange` (same JVM), `3_modified` (new JVM), each with `repo/`, `uploaded/` and `dumps/`; `scenario.json`, `generated_files.json` |
| `java_built/events_session/` | The GUI event calls (`s01`…`s07`), `session.json`, dumps |
| `java_presets/` | Arma 3 Launcher preset export cases (`preset.html`, `result.txt`, `case.json`) |
| `java_roundtrip/` | `HashSet`/`HashMap` capacities after fresh writes and round trips; `capacities.txt` |
| `java_harness_manifest.txt` | JDK, Java commit and `<sha256>  <size>  <path>` per file |

Identity dumps give every object (strings included) an `id` in first-visit order and write repeat
visits as `{"ref":N}`, so shared instances in the Java streams are visible. Semantic dumps
(`--semantic`) drop identities and sort sets and maps.

Empty directories (`emptydir`, `Collection` after step 3) cannot be committed; `scenario.json`
lists them under `dirs`, and lists every source file with size, mtime and SHA-1.
`generated_files.json` lists the other snapshot files outside `.a3s/` (`.zsync`, `.pbo.zip`,
`.ebo.zip`) with size, pinned mtime (`null` for `.zsync` files, whose own mtime nothing reads) and
`sync_leaf` (Java's exclusion rule: the generated `.ebo.zip` is hashed by the next build). File
mtimes do not survive git: consumers restore them from both files before re-verifying `.zsync`
(the `MTime:` header is the source file's mtime).

String sharing in the streams is Arma3Sync's own: every value that stands for a separate user
input is a separate `String` object in the harness (`LocalData.typed`), because Java string
literals are interned and would otherwise show up as back-references a real client does not
write. Shared strings that remain (the `"0"` SHA-1, the autoconfig `repositoryName`, lower-cased
addon keys, `Boolean.TRUE`/`FALSE`, enum constants) come from Arma3Sync code.

`verify` reports a `FAIL` line for an argument below which no file is readable (a3s files are only
recognised inside a `.a3s` directory), instead of silently skipping everything.

`run.sh` and `build.sh` export `LC_ALL=C.UTF-8` (or `en_US.UTF-8`): the JDK takes the file-name
encoding from the locale and ignores `-Dsun.jnu.encoding`, so non-ASCII paths need it. When
neither locale is installed they print a warning and keep the host locale.

The build stamp hashes every `.java` file of the source root (names and contents), so
uncommitted, untracked and non-git sources (an extracted tarball via `A3S_JAVA_SRC`, Java commit
`unknown`) are all covered.

## Harness-only changes (not Java behaviour)

- **P1** `build/patched-src/…/RepositoryBuildProcessor.java`: the 3 `new Date()` become
  `harness.HarnessClock.now()` (pinned build dates).
- **P2** same file: the 2 `listFiles()` results go through `harness.HarnessFs.sorted` (scan order
  sorted by name, as in the Rust builder). Recorded as `"scan_order":"sorted-by-name"`.
- `stubs/org/apache/commons/io/FileUtils.java`: `sizeOf(File)` only, mirroring commons-io 2.15.
- Zip entries: Java writes the current time. After each build the harness rewrites the DOS
  time/date of every zip that build wrote to `0x0000`/`0x0021` and sets its mtime to the build
  date, in the working tree as well as in the snapshot. This is needed because of the Java quirk
  below: a generated `.ebo.zip` is hashed by the next build.
- `build/variants/<v>`: historical stream shapes compiled from edited copies of current domain
  classes (`repository_pre_connections`, `prefs_pre_github`, `cfg_with_aia`,
  `treeleaf_pre_workshop`), loaded first on a child JVM's class path.

## Java quirks captured (do not "fix")

- `RepositoryBuildProcessor.java:487-489` excludes names containing `.zsync`, `.pbo.zip` or `.a3s`
  only. A generated `@mod_b/Addons/mod_b.ebo.zip` therefore becomes a sync leaf on the next build:
  in `http_zip`/`ftp_zip` the unchanged rebuild (step 2) bumps the revision to 2 and records
  `@mod_b` as updated, and step 3 reaches revision 3.
- Hidden folder paths, empty-directory pruning, `addons` detection, the `"0"` SHA-1 of empty files
  and its sharing, the autoconfig `repositoryName` back-reference, events filtering and the
  `HashSet`/`HashMap` capacities are all visible in the dumps.

## Determinism

Two `generate` runs on the same machine, JDK and Java commit give byte-identical trees regardless
of host time zone, locale, `TMPDIR`, CPU count and directory order. No output contains the work
directory path (checked, also inside gzip streams). Deflate bytes in `.gz` files may differ on
another zlib build; the decompressed stream is the contract.
