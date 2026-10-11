# a3s_repo_files

Files on disk: the gzip-wrapped `.a3s` repository metadata files (size limits, atomic writes), the
Rust client's own TOML store (configuration, preferences, profiles, repositories, SHA-1 caches,
Workshop metadata), the one-time importer of the Java client's files (including the AES-sealed
`*.a3s.repository`), and `hash`: parallel SHA-1 of repository files with the modification-time
cache.

Boundaries: depends on `a3s_types`, `a3s_jserial` and `a3s_foundation`; no network.
