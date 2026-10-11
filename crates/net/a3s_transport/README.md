# a3s_transport

Async transports (HTTP(S), FTP(S), SFTP, WebDAV) for Arma3Sync repositories, with proxies,
timeouts, throttling and cancellation.

Downloads work on every protocol. FTP(S), SFTP and WebDAV can also write to a repository
(`exists`, `upload_file`, `upload_bytes`, `mkdirs`, `delete`, upload sessions); HTTP supports
`exists` only. Tests run against the in-process FTP/FTPS, SFTP, WebDAV, HTTP and SOCKS servers of
`a3s_test_servers`.

`proxy::transport_options` builds a connection's options from the repository protocol and the
configured proxy, routed per target the way Java's system properties route traffic (HTTP proxy
for `http://`, HTTPS proxy for `https://`, SOCKS for every protocol, FTP proxy for nothing).
HTTP and WebDAV hand the proxy to `reqwest`; FTP(S) and SFTP tunnel their control and data
connections through the SOCKS proxy.

Every FTP command races the cancellation token; a command cancelled before its reply arrived
poisons the connection (`Error::ConnectionPoisoned`), which must then be reconnected.

`HttpTransport::probe_range` returns the status and headers of a `Range: bytes=0-1` request,
for the partial-transfer check.

Boundaries: knows URLs and bytes, not the repository format.
