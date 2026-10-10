# a3s_transport

Part of the Arma3Sync Rust rewrite (see [`ARCHITECTURE.md`](../../../ARCHITECTURE.md)).

Async transports (HTTP(S), FTP(S), SFTP, WebDAV) for Arma3Sync repositories.

Downloads work on every protocol. FTP(S), SFTP and WebDAV can also write to a repository
(`exists`, `upload_file`, `upload_bytes`, `mkdirs`, `delete`, upload sessions); HTTP supports
`exists` only. Tests run against in-process FTP/FTPS, SFTP, WebDAV and HTTP servers
(`tests/transport/support`).
