# Arma3Sync (Rust)

A Rust rewrite of the Java [Arma3Sync](https://github.com/vpzbrig21/Arma3Sync), the Arma 3 mod repository synchronizer, repository builder and launcher, for players and repository admins. Main parts: the Java serialization codec and domain types, repository files and zsync, transports (HTTP(S), FTP(S), SFTP, WebDAV), client sync, repository building, the egui UI, and the `arma3sync` binary.

---

## 1. Core Project Laws

The project is maintained by AI agents. Keep it small: every rule below states an outcome, and anything that serves no outcome is deleted rather than documented.

1. **No silent deferrals.** Do the whole ask. Never invent "out of scope", "deferred" or ship an MVP and call the task done. Only Sam's explicit words defer a piece; record it in ttm. Truly blocked: stop and ask.
2. **Git: direct to `main`.** For parallel work use a worktree: `git worktree add .worktrees/<name> -b <name>`, merge it into `main`, then `git worktree remove .worktrees/<name>` and delete the branch. No other branches. Commit only when asked.
3. **Everything earns its place.** A crate, module, command, check, test, README or document exists only if something uses it: CI, a runbook step, or a reader who needs it. Investigations and probes live in `.workstation/` and are deleted with their task; they never become commands. Extend an existing crate before adding one; a new crate needs a real boundary. Delete dead code instead of explaining it. When the clean fix needs a refactor, do the refactor.
4. **Wire compatibility with Java is the hard requirement.** Anything written to a repository (`.a3s/*`, `.zsync`, `.pbo.zip`/`.ebo.zip`) stays readable by the Java client and Java-built repositories stay readable here: `.a3s/*` and `.zsync` byte-identical, `.pbo.zip` semantically equal. Never change a Java class name, `serialVersionUID` or field descriptor (`crates/store/a3s_types/tests/types/descriptor_lock.rs` guards them). Wire-format code cites the Java source it mirrors (`// Mirrors dao/repository/RepositoryBuildProcessor.java:468`) and is covered by a golden fixture test. Wire names keep their Java spelling (`configration`, `protocole`, `racine`, `hidddenFolderPaths`). UI and error text need not match Java. Intentional differences from Java are listed in `ARCHITECTURE.md`.
5. **Boundaries.** Crates live at `crates/<group>/<crate>`; a crate depends only on its own group or the groups listed above it in `ARCHITECTURE.md`. Versions and lints are defined once in the root `Cargo.toml`. Library crates return typed errors (`thiserror`); the binary may use `anyhow`. Threads are named (`std::thread::Builder`), locks are `parking_lot` (enforced by `clippy.toml`). `java_version/` is the read-only Java reference checkout: never edit it.
6. **Names and comments say what things are now.** Self-describing names, no abbreviations or history; group many variants into named subfolders. Comments explain what and why (invariants, constraints, failure modes), never history, ticket ids, work-package or acceptance ids, or retired code. `///` docs on public items; `//!` only where a module's purpose is not obvious.
7. **Documentation is small and true.** One README per crate and per top-level folder: purpose, how to use, boundaries; no file-tree listings. Status, plans and open work live in ttm, never in docs. When a change alters a surface, command or boundary, update its README in the same commit.
8. **Tests cover core logic only.** Java serialization and wire formats (golden fixtures, descriptor lock, round trips), zsync, hashing, sync planning, downloads and resume, repository building, local store and legacy import, path safety. Never pin prose, error text, constants, file layout or `Debug`/`Display` output; no property or failpoint suites. Tests live in sibling files under `tests/`. Network tests against the live repository are `#[ignore = "network"]`; JDK cross-checks are `#[ignore = "needs JDK 25…"]`.
9. **The live repository is read-only and 93 GB.** `ftp://88.99.69.243/Repo/WOG/` (anonymous) is a third-party server: only metadata and a few small files, never a full download; the caps live inside the live tests. `https://repo.ofcra.org` is unreachable; test HTTP and zsync against local servers.
10. **CI is the law.** Every rule with a checker runs in CI (`.github/workflows/rust.yml`); a check not in CI is deleted. Gates pin their tools (`rust-toolchain.toml`); a gate that cannot run is a bug to fix, never a skip. Before committing: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and the tests of the crates you touched.
11. **Everything stays in the repository.** Nothing project-related goes to `~`, `~/.cache` or `/tmp`: machine state in the gitignored `.workstation/` (logs in `.workstation/logs/`), worktrees in `.worktrees/`, build output in `target/`. The JDK is the one exception: a toolchain at `~/.local/share/jdk/` (see `scripts/java_env.sh`).

---

## 2. Agent Environment and Workflow

- **Build:** Rust from `rust-toolchain.toml`, one `target/` in the checkout (unset any stale `CARGO_TARGET_DIR`). Agent sessions run in the `agents` distrobox (Debian), which shares the host's glibc. The repository sits on the Main_storage HDD (`/run/media/system/Disk_2/Projects/arma_3_sync_rs` is a symlink), so cold builds are slow.
- **JDK 25** (Temurin, user-local) for the Java fixture harness and the `#[ignore]`d Java cross-checks: `source scripts/java_env.sh` (override with `A3S_JAVA_HOME`). Plain `cargo test` needs no JDK.
- **Snapshot tests** (`crates/viewer/a3s_ui/tests/snapshots.rs`) render through wgpu on a CPU adapter: Mesa's lavapipe Vulkan driver must be installed (`mesa-vulkan-drivers`); egui_kittest picks it over a GPU automatically. Regenerate with `UPDATE_SNAPSHOTS=1`.
- **Java reference:** `java_version/Arma3Sync/src/main/java/fr/soe/a3s/` (gitignored upstream clone, read-only).
- **Machine-local state** (`.workstation/`, gitignored): `logs/`, scratch output.
- **Tracker:** ttm project `a3sync` (`ttm -p a3sync next`). Remaining ports and review follow-ups are tickets; their notes carry the full spec.
- **Orchestration is opt-in.** Work in the chat by default; sub-agents only for broad searches or genuinely parallel pieces; multi-agent programs only when Sam asks.
- **Bash discipline.** One build, test or gate per Bash call, each with its own log under `.workstation/logs/`; read logs and large files in ranges.

---

## 3. Repository Map

| Folder | Holds |
|---|---|
| `crates/` | The Cargo workspace by group: `foundation`, `store` (Java serialization, domain types, repository files, zsync), `net` (transports, sync, repository admin, self-updater), `game` (addons, game tools, OS integration, game launch), `viewer` (egui widgets), `application` (the `arma3sync` binary), `development` (Java harness wrapper, shared test servers). See `ARCHITECTURE.md` |
| `tests/fixtures/` | Golden fixtures written by the Java implementation (data; see its README) |
| `tools/java_fixture_harness/` | Java harness that runs the real Arma3Sync code to produce fixtures and verify Rust output |
| `scripts/java_env.sh` | Sources the JDK for the harness |
| `packaging/` | Linux AppImage build (`packaging/linux/build-appimage.sh`, run by CI) |
| `java_version/` | Gitignored Java reference checkout |
| `.github/workflows/` | CI: gates (`rust.yml`), AppImage artifact and tagged releases (`appimage.yml`) |
| `.workstation/`, `.worktrees/`, `target/` | Gitignored machine state, linked worktrees, build output |

---

## 4. Canonical Commands

```bash
cargo fmt --all -- --check                                          # Format check
cargo clippy --workspace --all-targets --all-features -- -D warnings # Lints
cargo test --workspace --all-features                               # All tests (no network, no JDK)
cargo test -p a3s_jserial -p a3s_types -p a3s_repo_files -p a3s_zsync # Golden wire-format tests
cargo doc --workspace --no-deps                                     # Docs (CI denies warnings)
cargo deny check && typos                                           # Licences/advisories, spelling
cargo run -p arma3sync                                          # The application
packaging/linux/build-appimage.sh                                 # Linux AppImage in target/appimage/

# JDK cross-checks and live network tests (opt-in)
source scripts/java_env.sh && cargo test -p a3s_dev_tools -- --ignored --test-threads=1
cargo test -p a3s_transport -- --ignored --test-threads=1           # Live WOG FTP, read-only, small files
cargo run -p a3s_dev_tools -- regen-fixtures --check                # Fixtures still match the Java harness

ttm -p a3sync next                                                  # Next tickets
```
