# a3s_admin

Repository administration, like the Java client's admin panel.

- `build` turns a local folder into an Arma3Sync repository: scan, SHA-1 with the
  modification-time cache, `.zsync` and `.pbo.zip` generation, and the five `.a3s` files (sync
  tree, server info, changelogs, autoconfig, events), written only after every stream is checked
  against Java's reader limits.
- `check` tests that the server holds every file the remote sync tree lists, on up to four
  connections.
- `upload` uploads a built repository: it plans against the remote sync tree, re-uploads files the
  server lost, uploads new and changed files (in parallel for FTP and SFTP), then the metadata
  exactly as Java uploads it (its deserialize-then-serialize round trip of the local files), then
  deletes what the repository no longer holds. `UploadResume` lets the caller resume a failed
  upload.
- `events` edits a repository's events (`EventsDoc`) and writes `.a3s/events` byte-identically to
  Java, locally or by upload; `fetch_admin_metadata` loads the admin panel's remote metadata.

Boundaries: depends on the store crates, `a3s_transport` and `a3s_sync` (connections, events,
remote errors); no UI. Tests use in-process FTP(S), SFTP, WebDAV and HTTP servers
(`tests/net_support`, which includes `a3s_transport`'s test servers).
