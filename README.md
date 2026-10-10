# Arma3Sync (Rust)

A full Rust rewrite of [Arma3Sync](https://github.com/vpzbrig21/Arma3Sync), the Arma 3 mod
repository synchronizer and launcher.

* **Wire-compatible** with existing Arma3Sync repositories: the Rust client syncs from repositories
  built by the Java version, and repositories built by the Rust version can be used by Java clients.
* **Native egui UI** (no JVM), single binary for Linux, Windows and macOS.
* Supports HTTP(S), FTP(S), SFTP and WebDAV repositories, zsync partial updates and compressed PBOs.

> Status: early development. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the crate layout and the
> roadmap.

## Building

```sh
cargo run -p arma3sync-cli --release
```

See [`BUILD.md`](BUILD.md) for details.

## Licence

GPL-3.0-or-later, same as the original Java project. See [`LICENSE`](LICENSE).

The Java reference implementation is expected (optionally) under `java_version/Arma3Sync/`; it is
not part of this repository.
