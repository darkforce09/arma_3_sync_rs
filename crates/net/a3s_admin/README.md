# a3s_admin

Repository administration. `build` turns a local folder into an Arma3Sync repository like the Java
client: scan, SHA-1 with the modification-time cache, `.zsync` and `.pbo.zip` generation, and the
five `.a3s` files (sync tree, server info, changelogs, autoconfig, events), written only after every
stream is checked against Java's reader limits. Check and upload are open ttm tickets.

Boundaries: depends on the store crates and `a3s_transport`; no UI.
