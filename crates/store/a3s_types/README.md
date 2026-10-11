# a3s_types

Arma3Sync's domain types (sync tree, server info, changelogs, events, autoconfig, protocols, local
client data) and their exact Java wire mappings: class names, `serialVersionUID`s and field
descriptors, locked by `tests/descriptor_lock.rs`.

Boundaries: builds on `a3s_jserial`; no file or network access.
