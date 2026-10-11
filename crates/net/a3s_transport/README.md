# a3s_transport

Async transports (HTTP(S), FTP(S), SFTP, WebDAV) for Arma3Sync repositories, with proxies,
timeouts, throttling and cancellation.

Downloads work on every protocol. FTP(S), SFTP and WebDAV can also write to a repository
(`exists`, `upload_file`, `upload_bytes`, `mkdirs`, `delete`, upload sessions); HTTP supports
`exists` only. Tests run against in-process FTP/FTPS, SFTP, WebDAV and HTTP servers
(`tests/transport/support`).

`proxy::transport_options` builds a connection's options from the repository protocol and the
configured proxy, routed per target the way Java's system properties route traffic (HTTP proxy
for `http://`, HTTPS proxy for `https://`, SOCKS for everything but SFTP, FTP proxy for
nothing). Only the HTTP and WebDAV transports apply a proxy; FTP(S) and SFTP ignore it.

Boundaries: knows URLs and bytes, not the repository format.
