//! Profiles (`dao/ProfileDAO.java`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use a3s_types::constants::DEFAULT_PROFILE_NAME;
use a3s_types::local::Profile;

use super::toml_file::{self, file_name, modified_millis};
use super::{Error, LocalStore};

/// Result of [`LocalStore::load_profiles`].
#[derive(Debug, Default)]
pub struct ProfilesLoad {
    /// Profiles by name. Always holds `"Default"` (in memory only if no file has it).
    pub profiles: BTreeMap<String, Profile>,

    /// File each profile was loaded from; the in-memory `"Default"` has no entry.
    pub sources: BTreeMap<String, PathBuf>,

    /// Files that could not be loaded, by file name.
    pub failures: BTreeMap<String, Error>,

    /// Duplicate profile names, in the repository warning format.
    pub warnings: Vec<String>,

    /// The profile name of each file that failed to decode but still names its profile (e.g. a
    /// file of a newer store format), by file name. Such a name is taken: writing it would
    /// duplicate or overwrite that file.
    pub unreadable_names: BTreeMap<String, String>,
}

impl ProfilesLoad {
    /// Java's `LoadingException` text, `None` without failures.
    // Mirrors dao/ProfileDAO.java:50-63
    pub fn error_message(&self) -> Option<String> {
        summary("Failed to load profiles:", &self.failures)
    }
}

/// `<header>` plus one `"\n - <file>: <message>"` line per failure.
pub(crate) fn summary(header: &str, failures: &BTreeMap<String, Error>) -> Option<String> {
    if failures.is_empty() {
        return None;
    }
    let mut message = header.to_owned();
    for (file, err) in failures {
        let detail = err.detail();
        if detail.is_empty() {
            message.push_str(&format!("\n - {file}"));
        } else {
            message.push_str(&format!("\n - {file}: {detail}"));
        }
    }
    Some(message)
}

/// Warning text of a duplicate whose newer copy replaced the kept one.
// Mirrors dao/repository/RepositoryDAO.java:183-184
pub(crate) fn replaced_warning(name: &str, old_file: &str, new_file: &str) -> String {
    format!("{name}: replaced \"{old_file}\" with \"{new_file}\" because it is newer.")
}

/// Warning text of a duplicate that was ignored.
// Mirrors dao/repository/RepositoryDAO.java:186-187
pub(crate) fn ignored_warning(name: &str, new_file: &str, old_file: &str) -> String {
    format!("{name}: ignored \"{new_file}\" because \"{old_file}\" is newer.")
}

impl LocalStore {
    /// Loads every `profiles/*.toml`, in file-name order; a profile's identity is its `name`.
    ///
    /// Failures are collected per file. Of two files with the same name the newer one wins (the
    /// first in file-name order on a tie). Only an unreadable directory is an error.
    // Mirrors dao/ProfileDAO.java:24-65
    pub fn load_profiles(&self) -> Result<ProfilesLoad, Error> {
        let dir = &self.dirs.profiles;
        let files = toml_file::list_toml_files(dir).map_err(|source| Error::Read {
            path: dir.clone(),
            source,
        })?;
        let mut load = ProfilesLoad::default();
        let mut mtimes: BTreeMap<String, i64> = BTreeMap::new();
        for path in files {
            let file = file_name(&path);
            let profile: Profile = match std::fs::read_to_string(&path) {
                Ok(text) => match toml_file::decode(&path, &text, "profile") {
                    Ok(profile) => profile,
                    Err(err) => {
                        if let Some(name) = toml_file::peek_name(&text, "profile") {
                            load.unreadable_names.insert(file.clone(), name);
                        }
                        load.failures.insert(file, err);
                        continue;
                    }
                },
                Err(source) => {
                    load.failures.insert(file, Error::Read { path, source });
                    continue;
                }
            };
            let mtime = modified_millis(&path);
            let name = profile.name.clone();
            if let Some(&kept_mtime) = mtimes.get(&name) {
                let kept_file = load
                    .sources
                    .get(&name)
                    .map(|p| file_name(p))
                    .unwrap_or_default();
                if mtime > kept_mtime {
                    load.warnings
                        .push(replaced_warning(&name, &kept_file, &file));
                } else {
                    load.warnings
                        .push(ignored_warning(&name, &file, &kept_file));
                    continue;
                }
            }
            mtimes.insert(name.clone(), mtime);
            load.sources.insert(name.clone(), path);
            load.profiles.insert(name, profile);
        }
        // Mirrors dao/ProfileDAO.java:44-48
        if !load.profiles.contains_key(DEFAULT_PROFILE_NAME) {
            load.profiles.insert(
                DEFAULT_PROFILE_NAME.to_owned(),
                Profile::new(DEFAULT_PROFILE_NAME),
            );
        }
        Ok(load)
    }

    /// Writes `profiles/<file_stem(name)>.toml` atomically (no `.backup` file is made).
    // Mirrors dao/ProfileDAO.java:67-98
    pub fn write_profile(&self, profile: &Profile) -> Result<(), Error> {
        self.write_profile_replacing(profile, None)
    }

    /// Like [`Self::write_profile`], then deletes `previous` (best effort) when it is another
    /// file, e.g. the file of a renamed profile or the file it was loaded from.
    ///
    /// `previous` is only deleted when it is a `.toml` file directly inside the profiles
    /// directory; anything else is logged and kept.
    pub fn write_profile_replacing(
        &self,
        profile: &Profile,
        previous: Option<&Path>,
    ) -> Result<(), Error> {
        let _guard = self.write_lock.lock();
        let target = self.profile_path(&profile.name);
        toml_file::write(&target, "profile", profile).map_err(|err| match err {
            Error::Write { path, source } => Error::WriteProfile { path, source },
            other => other,
        })?;
        if let Some(previous) = previous {
            Self::delete_previous(&self.dirs.profiles, previous, &target);
        }
        Ok(())
    }

    /// Deletes a profile file outside the canonical name, e.g. the file a profile was loaded
    /// from. Only a `.toml` file directly inside the profiles directory is deleted; whether a
    /// file was removed.
    pub fn delete_profile_file(&self, path: &Path) -> Result<bool, Error> {
        self.delete_file_in(&self.dirs.profiles, path)
    }

    /// Deletes the profile's file; whether a file was removed.
    // Mirrors dao/ProfileDAO.java:100-108
    pub fn delete_profile(&self, name: &str) -> Result<bool, Error> {
        let _guard = self.write_lock.lock();
        Self::delete_file(&self.profile_path(name))
    }
}
