# a3s_sync

The client synchronization engine: fetch a repository's `.a3s` metadata, compare it with the
local folder into a plan (downloads, zsync patches, deletions, never outside the destination),
and run the download with resume, zsync and progress events.

Boundaries: depends on the store crates and `a3s_transport`; no UI. Live tests against the public
WOG repository are `#[ignore]`d and read only metadata and small files.
