# a3s_sync

The client synchronization engine: fetch a repository's `.a3s` metadata, compare it with the
local folder into a plan (downloads, zsync patches, deletions, never outside the destination),
download with resume, zsync and progress events, uncompress archives, delete extra files and
write the download report.

Use `SyncSession` to check a repository, change the selection and run downloads (a paused run
resumes on the next call), or `sync_repository` for an unattended synchronization (automatic
updates, command line). Client data is read and written only through the caller's `SyncStore`;
`MemoryStore` serves tests and dry runs.

Boundaries: depends on the store crates and `a3s_transport`; no UI and no client-data files.
Live tests against the public WOG repository are `#[ignore]`d and read only metadata and small
files.
