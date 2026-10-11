# a3s_addons

The Arma 3 addons on the player's machine and the profile's addon groups, as the Java client
handles them.

- `scan` walks the profile's search directories and the Arma 3 folder: mods (a folder with an
  `addons` folder holding `.pbo`/`.ebo` files) and CDLCs go into an `AddonRegistry` under their
  symbolic keys (`@ace`, `@ace*`, …), and the available-addons tree and list are built for display.
- `priority` keeps `Profile::addon_names_by_priority` in line with the registry and moves entries
  of it and of the search-directory list.
- `group_from_repository`, `add_groups_from_repository`, `add_groups_from_events` and
  `update_group_modsets` build and refresh modset groups from a repository's `.a3s/sync` tree and
  events, keeping the user's selection.
- `refresh_group_status` resolves group leaves against the registry (download locations,
  Workshop ids from `meta.cpp`, DLC app ids) and flags missing, duplicate and doubly selected
  addons; `check_missing_selected_addons` and `check_duplicate_addons_selection` are the launch
  checks.

Leaf names written into profiles are registry keys, so key allocation follows Java exactly.

Boundaries: synchronous functions over owned values, no threads, no persistence (the caller saves
the profile) and no UI. Depends only on `a3s_types`.
