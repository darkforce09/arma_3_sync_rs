# a3s_repo_files

Files on disk: the gzip-wrapped `.a3s` repository metadata files (size limits, atomic writes
through `<name>.<random>.a3s.tmp` temporary files, which repository builds never scan), the
Rust client's own TOML store (configuration, preferences, profiles, repositories, SHA-1 caches,
Workshop metadata), the one-time importer of the Java client's files (including the AES-sealed
`*.a3s.repository`), the `export.a3s.autoconfig` file exchanged between clients
(`autoconfig_export`, refused when a Java client could not read it back), and `hash`: parallel
SHA-1 of repository files with the modification-time cache.

`services::ClientData` is the client's state in memory (configuration, preferences, profiles,
repositories) for the CLI and the viewer: load it once with `ClientData::load`, change it only
through its methods, and every change is written to the TOML store before the call returns (on
failure memory is left unchanged). Connections apply the stored proxy through
`a3s_transport::proxy::transport_options`.

Boundaries: depends on `a3s_types`, `a3s_jserial` and `a3s_foundation`; no network.
