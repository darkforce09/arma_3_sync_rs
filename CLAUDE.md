# Notes for Claude

* Rust rewrite of the Java Arma3Sync at `java_version/Arma3Sync` (gitignored, read-only reference).
* Layout and wire-format contract: `ARCHITECTURE.md`. Style: `CODE_STYLE.md`.
* Never change Java class names, serialVersionUIDs or field descriptors in `a3s_types`.
* Before committing: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && typos`.
* Live test repository (read-only): `https://repo.ofcra.org/.a3s/autoconfig`.
