//! Profiles (`service/ProfileService.java`).

use a3s_types::constants::DEFAULT_PROFILE_NAME;
use a3s_types::java_string::java_trim;
use a3s_types::local::protocol::parse_java_int;
use a3s_types::local::{LauncherOptions, Profile, TreeDirectory, TreeLeaf, TreeNode};

use super::{ClientData, Result, ServiceError};

impl ClientData {
    /// Every profile name, `"Default"` included, in Java `String` order (UTF-16 code units).
    // Mirrors service/ProfileService.java:57-65
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.profiles.keys().cloned().collect();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        names
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    /// The profile named by the configuration's active profile.
    pub fn active_profile(&self) -> Option<&Profile> {
        self.profiles.get(&self.configuration.profile_name)
    }

    /// Whether `name` is in use: a loaded profile, an unreadable profile file naming it, or a
    /// file at its path. Writing it would replace or duplicate a profile.
    pub(super) fn profile_name_taken(&self, name: &str) -> bool {
        self.profiles.contains_key(name)
            || self.unreadable_profiles.contains(name)
            || self.store.profile_path(name).exists()
    }

    /// Rejects a blank name and a name in use.
    fn check_new_profile_name(&self, name: &str) -> Result<()> {
        if java_trim(name).is_empty() {
            return Err(ServiceError::EmptyProfileName);
        }
        if self.profile_name_taken(name) {
            return Err(ServiceError::ProfileExists(name.to_owned()));
        }
        Ok(())
    }

    /// Creates and writes an empty profile.
    // Mirrors service/ProfileService.java:67-75
    pub fn create_profile(&mut self, name: &str) -> Result<()> {
        self.check_new_profile_name(name)?;
        self.commit_profile(Profile::new(name))
    }

    /// Creates `target` as a copy of `source`'s addon groups, search directories, additional
    /// parameters and launch settings (all but the Steam path and the mission file). Addon
    /// priorities and the last added search directory are not copied. A missing `source` does
    /// nothing.
    // Mirrors service/ProfileService.java:77-91
    pub fn duplicate_profile(&mut self, source: &str, target: &str) -> Result<()> {
        let Some(original) = self.profiles.get(source) else {
            return Ok(());
        };
        self.check_new_profile_name(target)?;
        let mut copy = Profile::new(target);
        duplicate_tree(&original.tree, &mut copy.tree);
        copy.addon_search_directories
            .clone_from(&original.addon_search_directories);
        copy.additional_parameters
            .clone_from(&original.additional_parameters);
        duplicate_launcher_options(&original.launcher_options, &mut copy.launcher_options);
        self.commit_profile(copy)
    }

    /// Renames a profile: writes it under `new`, deletes the old file and keeps it active if it
    /// was. `"Default"` cannot be renamed.
    // Mirrors service/ProfileService.java:93-104
    pub fn rename_profile(&mut self, old: &str, new: &str) -> Result<()> {
        let Some(profile) = self.profiles.get(old) else {
            return Err(ServiceError::ProfileNotFound(old.to_owned()));
        };
        if old == DEFAULT_PROFILE_NAME {
            return Err(ServiceError::DefaultProfileRename);
        }
        if java_trim(new).is_empty() {
            return Err(ServiceError::EmptyProfileName);
        }
        if new == old {
            return Ok(());
        }
        if self.profile_name_taken(new) {
            return Err(ServiceError::ProfileExists(new.to_owned()));
        }
        let mut renamed = profile.clone();
        new.clone_into(&mut renamed.name);
        // Java deletes the file of the *new* name, so the old file survives and the profile
        // reappears at the next start; here the old file is replaced.
        let previous = self
            .profile_sources
            .get(old)
            .cloned()
            .unwrap_or_else(|| self.store.profile_path(old));
        self.store
            .write_profile_replacing(&renamed, Some(&previous))?;
        self.profiles.remove(old);
        self.profile_sources.remove(old);
        self.profile_sources
            .insert(new.to_owned(), self.store.profile_path(new));
        self.profiles.insert(new.to_owned(), renamed);
        if self.configuration.profile_name == old {
            self.set_active_profile(new)?;
        }
        Ok(())
    }

    /// Removes a profile and its file; when it was active, `"Default"` becomes active.
    /// `"Default"` cannot be removed.
    // Mirrors service/ProfileService.java:106-114
    pub fn remove_profile(&mut self, name: &str) -> Result<()> {
        if !self.profiles.contains_key(name) {
            return Err(ServiceError::ProfileNotFound(name.to_owned()));
        }
        if name == DEFAULT_PROFILE_NAME {
            return Err(ServiceError::DefaultProfileRemoval);
        }
        self.store.delete_profile(name)?;
        if let Some(source) = self.profile_sources.get(name)
            && *source != self.store.profile_path(name)
        {
            self.store.delete_profile_file(source)?;
        }
        self.profiles.remove(name);
        self.profile_sources.remove(name);
        if self.configuration.profile_name == name {
            self.set_active_profile(DEFAULT_PROFILE_NAME)?;
        }
        Ok(())
    }

    /// Runs `f` on a copy of the active profile and, when it returns `true` ("changed"),
    /// writes and keeps it. `false` writes nothing; `f` must not change the name.
    pub fn update_active_profile(&mut self, f: impl FnOnce(&mut Profile) -> bool) -> Result<bool> {
        let active = self.configuration.profile_name.clone();
        self.update_profile(&active, f)
    }

    /// Runs `f` on a copy of the profile `name` and, when it returns `true` ("changed"),
    /// writes and keeps it. `false` writes nothing; `f` must not change the name.
    pub fn update_profile(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Profile) -> bool,
    ) -> Result<bool> {
        let Some(profile) = self.profiles.get(name) else {
            return Err(ServiceError::ProfileNotFound(name.to_owned()));
        };
        let mut next = profile.clone();
        if !f(&mut next) {
            return Ok(false);
        }
        if next.name != name {
            return Err(ServiceError::NameChange);
        }
        self.commit_profile(next)?;
        Ok(true)
    }

    /// Runs `f` on the active profile and always writes it.
    fn set_active_profile_field(&mut self, f: impl FnOnce(&mut Profile)) -> Result<()> {
        self.update_active_profile(|p| {
            f(p);
            true
        })
        .map(drop)
    }

    // Mirrors service/ProfileService.java:32-43
    pub fn set_additional_parameters(&mut self, value: Option<String>) -> Result<()> {
        self.set_active_profile_field(|p| p.additional_parameters = value)
    }

    /// Appends a search directory (no de-duplication).
    // Mirrors service/ProfileService.java:197-209
    pub fn add_addon_search_directory(&mut self, path: String) -> Result<()> {
        self.set_active_profile_field(|p| p.addon_search_directories.push(path))
    }

    /// Removes the first entry equal to `path`; `false` (nothing written) when absent.
    // Mirrors service/ProfileService.java:211-223
    pub fn remove_addon_search_directory(&mut self, path: &str) -> Result<bool> {
        self.update_active_profile(|p| {
            let index = p.addon_search_directories.iter().position(|d| d == path);
            if let Some(index) = index {
                p.addon_search_directories.remove(index);
            }
            index.is_some()
        })
    }

    // Mirrors service/ProfileService.java:237-249
    pub fn set_last_added_addon_search_directory(&mut self, value: Option<String>) -> Result<()> {
        self.set_active_profile_field(|p| p.last_added_addon_search_directory = value)
    }

    /// Changes the active profile's launch settings and writes the profile.
    // Mirrors service/ProfileService.java:403-709
    pub fn update_launcher_options(&mut self, f: impl FnOnce(&mut LauncherOptions)) -> Result<()> {
        self.set_active_profile_field(|p| f(&mut p.launcher_options))
    }

    /// Sets the CPU count from the dialog's text: `None` is 0, otherwise Java
    /// `Integer.parseInt`.
    // Mirrors service/ProfileService.java:555-571
    pub fn set_cpu_count(&mut self, value: Option<&str>) -> Result<()> {
        let count = match value {
            None => 0,
            Some(text) => {
                parse_java_int(text).ok_or_else(|| ServiceError::InvalidNumber(text.to_owned()))?
            }
        };
        self.update_launcher_options(|o| o.cpu_count_selection = count)
    }

    /// `None` without an active profile.
    // Mirrors service/ProfileService.java:45-55
    pub fn additional_parameters(&self) -> Option<&str> {
        self.active_profile()?.additional_parameters.as_deref()
    }

    /// Empty without an active profile.
    // Mirrors service/ProfileService.java:185-195
    pub fn addon_search_directories(&self) -> &[String] {
        self.active_profile()
            .map_or(&[], |p| p.addon_search_directories.as_slice())
    }

    /// `None` without an active profile.
    // Mirrors service/ProfileService.java:225-235
    pub fn last_added_addon_search_directory(&self) -> Option<&str> {
        self.active_profile()?
            .last_added_addon_search_directory
            .as_deref()
    }

    /// `None` without an active profile.
    // Mirrors service/ProfileService.java:171-183
    pub fn launcher_options(&self) -> Option<&LauncherOptions> {
        self.active_profile().map(|p| &p.launcher_options)
    }
}

/// Copies `source`'s groups into `target`: leaves with their name, selection, optional flag and
/// Workshop/DLC ids; groups with their name, selection and modset fields only (their `optional`
/// and `marked` flags are not copied).
// Mirrors service/ProfileService.java:116-147
fn duplicate_tree(source: &TreeDirectory, target: &mut TreeDirectory) {
    for node in &source.children {
        match node {
            TreeNode::Leaf(leaf) => target.add_child(TreeNode::Leaf(TreeLeaf {
                name: leaf.name.clone(),
                selected: leaf.selected,
                optional: leaf.optional,
                workshop_id: leaf.workshop_id.clone(),
                dlc_app_id: leaf.dlc_app_id.clone(),
                missing: false,
            })),
            TreeNode::Directory(directory) => {
                let mut copy = TreeDirectory::new(directory.name.clone());
                copy.selected = directory.selected;
                copy.modset_type = directory.modset_type;
                copy.modset_repository_name
                    .clone_from(&directory.modset_repository_name);
                duplicate_tree(directory, &mut copy);
                target.add_child(TreeNode::Directory(copy));
            }
        }
    }
}

/// Copies the launch settings a duplicate inherits; the Steam path and the mission file stay
/// at their defaults. Java copies `no_pause` into `no_splash_screen`; the real value is copied.
// Mirrors service/ProfileService.java:149-169
fn duplicate_launcher_options(source: &LauncherOptions, target: &mut LauncherOptions) {
    target.cpu_count_selection = source.cpu_count_selection;
    target
        .ex_threads_selection
        .clone_from(&source.ex_threads_selection);
    target.malloc_selection.clone_from(&source.malloc_selection);
    target.default_world = source.default_world;
    target.game_profile.clone_from(&source.game_profile);
    target
        .max_memory_selection
        .clone_from(&source.max_memory_selection);
    target.file_patching = source.file_patching;
    target.nologs = source.nologs;
    target.no_pause = source.no_pause;
    target.enable_ht = source.enable_ht;
    target.huge_pages = source.huge_pages;
    target.no_splash_screen = source.no_splash_screen;
    target.show_script_errors = source.show_script_errors;
    target.window_mode = source.window_mode;
    target.check_signatures = source.check_signatures;
    target.auto_restart = source.auto_restart;
    target.launch_arma3_as_administrator = source.launch_arma3_as_administrator;
    target.arma3_exe_path.clone_from(&source.arma3_exe_path);
    target.game_start = source.game_start;
}
