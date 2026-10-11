# a3s_zsync

The zsync format as Arma3Sync uses it (jazsync): `.zsync` writer byte-identical to Java's, reader,
block matching against a local file and patch assembly with SHA-1 verification.

Boundaries: pure format and algorithm; the download side lives in `a3s_sync`.
