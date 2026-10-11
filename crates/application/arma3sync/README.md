# arma3sync

The `arma3sync` binary (GUI by default, command line subcommands) and the build information baked
into it (version, git hash, target, profile; `build.rs` falls back to `unknown` without git).

Run it with `cargo run -p arma3sync`. The entry point is still a placeholder: the command line and
the viewer are ttm tickets (`command-line-interface`, `viewer-shell`).

Boundaries: the only application crate; nothing depends on it.
