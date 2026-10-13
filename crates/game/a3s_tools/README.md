# a3s_tools

The logic behind the Java client's game tools, without any UI:

- `bikey`: the bikey extractor. `validate` checks the source and target fields, `extract` walks
  the source for `*.bikey` files and copies them flat into the target, with a `CancelToken`. The
  wizard and the command line's `-extract` both use it.
- `rpt`: the RPT viewer engine. `LogTailer` is a pollable tail that survives appends,
  truncation and rotation; `RptTail` runs it on the `rpt-tail` thread, sends `RptEvent`s to a
  sink and stops on `stop` or drop.
- `radio`: the TFAR and ACRE 2 installer wizards: TeamSpeak 3 detection, the system
  specifications labels, first-page validation (`validate_tfar`, `validate_acre2`), the copy steps
  (`install_tfar`, `install_acre2`), the plugin dll version (`plugin_version`, from its Windows
  version resource) and the wizard texts (`radio::strings`).
- `preset`: Arma 3 Launcher HTML presets for the addon-groups panel. `compare` / `import_preset`
  read a preset and match its Workshop mods and CDLCs against the local addons;
  `add_imported_group` turns the result into a checked addon group. `generate` collects a
  modset's selected addons (reading `meta.cpp`, falling back to the Workshop metadata store) and
  renders the HTML byte-identical to Java's; `write` replaces the target file atomically. The
  remaining helpers pick the modset to export, propose file names and check the metadata
  dialog's input.

Messages and labels keep the Java client's wording. Blocking calls; the caller picks the thread,
except `RptTail`, which owns its own.

Boundaries: depends on `a3s_types` (addons, addon-group trees, CDLCs) and `a3s_repo_files`
(the Workshop metadata store, read only); no UI, no configuration storage. Addons come in from
the caller as plain values, so addon discovery stays outside. Platform lookups
(the TeamSpeak 3 registry entry, the default RPT folder, the process list) come in as plain values
or through the `Ts3ProcessProbe` trait.
