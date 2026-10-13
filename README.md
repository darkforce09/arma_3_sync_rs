# Arma3Sync (Rust)

A full Rust rewrite of [Arma3Sync](https://github.com/vpzbrig21/Arma3Sync), the Arma 3 mod
repository synchronizer and launcher.

* **Wire-compatible** with existing Arma3Sync repositories: the Rust client syncs from repositories
  built by the Java version, and repositories built by the Rust version can be used by Java clients.
* **Native egui UI** (no JVM), single binary for Linux, Windows and macOS.
* Supports HTTP(S), FTP(S), SFTP and WebDAV repositories, zsync partial updates and compressed PBOs.

[`ARCHITECTURE.md`](ARCHITECTURE.md) describes the crates, the repository wire format and the
intentional differences from Java.

## Building

Prerequisites:

* Rust: the version pinned in `rust-toolchain.toml` is installed automatically by `rustup`.
* Linux: the egui/eframe system library `libxkbcommon` and Mesa's lavapipe Vulkan driver for the
  UI snapshot tests. Folder dialogs use the XDG desktop portal and the tray icon a
  StatusNotifierItem over D-Bus, so the window needs no GTK.
* Optional, for the Java cross-checks and fixture regeneration: JDK 25 (`scripts/java_env.sh`) and
  a checkout of the Java version in `java_version/Arma3Sync` (see
  [`tools/java_fixture_harness`](tools/java_fixture_harness/README.md)).

| Purpose | Command |
| --- | --- |
| Run the app | `cargo run -p arma3sync` |
| Release build | `cargo build -p arma3sync --release` |
| All tests | `cargo test --workspace --all-features` |
| Live read-only tests against a public repository | `cargo test -p a3s_transport -- --ignored --test-threads=1` |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Formatting | `cargo fmt --all` |
| Licences / advisories | `cargo deny check` |
| Spelling | `typos` |

## Licence

GPL-3.0-or-later, same as the original Java project. See [`LICENSE`](LICENSE).
