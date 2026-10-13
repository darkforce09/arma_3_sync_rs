# arma3sync

The `arma3sync` binary: the window start, the command line, the interactive console, and the
build information baked into it (version, git hash, target, profile; `build.rs` falls back to
`unknown` without git).

```text
arma3sync build NAME                          -BUILD NAME
arma3sync check NAME                          -CHECK NAME
arma3sync sync NAME FOLDER [--exact-match]    -SYNC NAME FOLDER true|false
arma3sync extract SOURCE TARGET               -EXTRACT SOURCE TARGET
arma3sync update [--dev]                      -UPDATE
arma3sync console [--dev]                     -CONSOLE, -DEV -CONSOLE
--debug (anywhere)                            -DEBUG (anywhere), A3S_DEBUG=true
```

The Java client's single-dash flags keep working (case-insensitive, Java's argument counts).
Exit codes: 0 success, 1 failure (including a check that finds missing files), 2 invalid
arguments; `update` returns the updater's 0 (updated), 2 (already up to date) or 1 (failed).
With no command, or `-dev`/`-run`/`-unlock`, it opens the window (`a3s_viewer`): `-dev`
development mode, `-run` hidden in the tray, `-unlock` without the single-instance lock. A second
start shows the first window and exits with 1, and so does a start without a display.

`console` reads commands (`NEW`, `BUILD`, `CHECK`, `DELETE`, `LIST`, `SYNC`, `EXTRACT`, `UPDATE`,
`COMMANDS`, `VERSION`, `QUIT`) and their answers line by line from standard input, so scripts can
pipe them in. It ends on `QUIT` or at the end of the input, and exits with 1 when a command
failed or was unknown, or the input ended inside a command.

Data lives in the application directories of `a3s_foundation::paths` (`A3S_CONFIG_DIR`,
`A3S_DATA_DIR`, `A3S_CACHE_DIR` override them); every change goes through
`a3s_repo_files::services::ClientData`, which writes it at once. The synchronization stores its
revision and caches through the same data (`ClientStore`). Warnings go to stderr (`A3S_LOG`
filters them); `--debug` adds the rotating log file in the data folder's `logs`. At start-up,
leftovers of interrupted self-updates next to the executable are cleaned up; the hidden elevated
updater subcommand is dispatched before anything else.

Tests (`tests/cli`) run the real binary against a private data folder and a local HTTP server.

Boundaries: the only application crate; nothing depends on it. The window itself lives in
`a3s_viewer`.
