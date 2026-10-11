# a3s_types

Arma3Sync's domain types (sync tree, server info, changelogs, events, autoconfig, protocols, local
client data) and their exact Java wire mappings: class names, `serialVersionUID`s and field
descriptors, locked by `tests/descriptor_lock.rs`. Also the pure rules around them: autoconfig URL
parsing and formatting (`autoconfig_url`) and when TLS certificate validation may be disabled
(`ssl_policy`).

Boundaries: builds on `a3s_jserial`; no file or network access.
