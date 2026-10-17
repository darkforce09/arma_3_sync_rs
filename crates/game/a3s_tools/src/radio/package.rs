//! The TFAR 1.0 layout: the mod ships its `TeamSpeak` plugin as `.ts3_plugin` packages in a
//! `teamspeak` folder, which `TeamSpeak`'s own package installer installs.

use std::path::{Path, PathBuf};

/// The package of the current TFAR release.
const MAIN_PACKAGE: &str = "task_force_radio.ts3_plugin";

/// The `TeamSpeak` package of a TFAR 1.0 mod folder: `teamspeak/task_force_radio.ts3_plugin`
/// when present, else the first `.ts3_plugin` of `teamspeak` by name; `None` without any.
pub fn tfar_package(mod_dir: &Path) -> Option<PathBuf> {
    let folder = mod_dir.join("teamspeak");
    let main = folder.join(MAIN_PACKAGE);
    if main.is_file() {
        return Some(main);
    }
    let mut packages: Vec<PathBuf> = std::fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("ts3_plugin"))
        })
        .collect();
    packages.sort();
    packages.into_iter().next()
}
