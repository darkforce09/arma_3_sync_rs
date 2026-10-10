# Contributing

1. Read [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`CODE_STYLE.md`](CODE_STYLE.md).
2. Before opening a PR, run:
   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo deny check
   typos
   ```
3. Changes to anything written to a repository (`.a3s/*`, `.zsync`, `.zip`) must keep the golden
   fixture tests green. Never change a Java class name, serialVersionUID or field descriptor.
