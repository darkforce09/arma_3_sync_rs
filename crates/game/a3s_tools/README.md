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
  (`install_tfar`, `install_acre2`) and the wizard texts (`radio::strings`).

Messages and labels keep the Java client's wording. Blocking calls; the caller picks the thread,
except `RptTail`, which owns its own.

Boundaries: depends only on external crates; no UI, no configuration storage. Platform lookups
(the TeamSpeak 3 registry entry, the default RPT folder, the process list) come in as plain values
or through the `Ts3ProcessProbe` trait.
