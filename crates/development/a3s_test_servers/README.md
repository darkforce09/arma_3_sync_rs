# a3s_test_servers

In-process servers for the tests of `a3s_transport`, `a3s_sync` and `a3s_admin`: FTP and FTPS
(libunftp), a scripted strict FTP server, SFTP (russh), WebDAV (dav-server in axum), a recording
TCP relay that can cut its connections, and a SOCKS4/SOCKS5 proxy. Each server binds
`127.0.0.1:0` and stops when its handle is dropped; `Fixture` bundles a server with the protocol
to reach it and the directory it serves.

Dev-dependency only: it keeps the server crates out of the libraries' dependency trees.
