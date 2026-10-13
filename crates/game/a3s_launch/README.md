# a3s_launch

Starting Arma 3 and the external applications that go with it, as the Java client's launch
button does. This is the one crate that spawns the game.

- `params`: the profile's command-line parameters in Java's exact order (additional parameters,
  launcher options, server join, `-mod=` groups), plus the Launcher Options preview and the
  desktop-shortcut argument string. Arma 3 sees these, so Java's quirks are kept.
- `executable`: the "Executable location" check, the executable kind, the BattlEye names and the
  "Enable BattlEye" path switch.
- `cfg`: removes the Arma 3 Launcher's `class ModLauncherList` from `Arma3.cfg` before each launch.
- `command`, `elevation`, `steam`: pure planning of the exact command for every OS: BattlEye's
  `2 1` prefix, UAC elevation through PowerShell `-EncodedCommand`, `.bat` through
  `cmd /C start ""`, `.sh` through bash, and on Linux the Steam/Proton launch
  (`steam -applaunch 107410 -noLauncher …`, mod paths in Wine `Z:` form), chosen by the
  `LinuxLaunchMode` a profile's `GameStart` setting converts into.
- `runner`: `spawn_plan` runs a plan on the `a3s-launch-arma3` thread, sends `LaunchEvent`s to an
  `EventSink` and restarts the game when auto-restart is on, until `stop_auto_restart` or the
  fast-exit guard stops it. Nothing kills the game.
- `external`: starts the enabled external applications that are not already running.

`launch_arma3` ties it together: parameters, plan, `Arma3.cfg` clean-up, launch thread. Planning
errors (no executable, an executable this OS cannot run, no Steam) come back before any thread
starts; everything after the spawn arrives as events.

Boundaries: depends on `a3s_types` (profile and configuration), `a3s_addons` (the addon registry),
`a3s_platform` (process probe, Steam app id) and `a3s_foundation` (`Os`, `PlatformEnv`, Windows
argument and PowerShell quoting). The caller supplies the profile, the registry, the Arma 3 user
folder and the Steam command; no UI, no persistence. Tests never start the game: planning is checked as
argument vectors, and process tests run small shell scripts.

## Manual checks

CI cannot start the real game. After a change to planning or the runner, check by hand:

- Windows: an elevated launch ("launch as administrator") with a mod path containing spaces loads
  the mod; refusing the UAC prompt ends with exit code 1.
- Linux: a launch through Steam starts the game under Proton with `-window` and a `-mod=Z:\…` mod
  loaded, without the Arma 3 Launcher and without Steam's argument prompt blocking it.
