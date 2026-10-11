# a3s_foundation

The base every other crate builds on, one module per concern:

- `command_line`: Windows command-line (`CommandLineToArgvW`) and PowerShell quoting, and
  PowerShell `-EncodedCommand` encoding.
- `error`: the shared error taxonomy (`ErrorKind`, `UserFacing`), error-chain formatting and the
  error report shown for unexpected failures.
- `format`: sizes, speeds, durations and dates for display (the size/speed/duration rules follow
  Java's `UnitConverter`).
- `log`: the `tracing` setup of the binaries: stderr layer (`A3S_LOG`), opt-in rotating debug file,
  credential redaction.
- `paths`: the Rust client's own configuration, data and cache directories (`arma3sync-rs`,
  portable mode) and read-only discovery of the Java client's data for the importer.

Boundaries: no network, no wire format, no UI. Depends only on external crates.
