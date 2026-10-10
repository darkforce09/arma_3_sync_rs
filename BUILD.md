# Building Arma3Sync

## Prerequisites

* Rust (the version pinned in `rust-toolchain.toml` is installed automatically by `rustup`).
* Linux: the usual egui/eframe system libraries
  (`libxkbcommon`, `libgtk-3` for file dialogs and tray, `libxdo` for the tray on X11).
* Optional, for regenerating golden compatibility fixtures: JDK 25 and a checkout of the Java
  version in `java_version/Arma3Sync` (see `tools/java_fixture_harness/README.md`).

## Common commands

| Purpose | Command |
| --- | --- |
| Run the app | `cargo run -p arma3sync-cli` |
| Release build | `cargo build -p arma3sync-cli --release` |
| All tests | `cargo test --workspace` |
| Live tests against a public repository | `cargo test --workspace -- --ignored` |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| Formatting | `cargo fmt --all` |
| Licences / advisories | `cargo deny check` |
| Spelling | `typos` |
