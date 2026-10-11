use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use a3s_foundation::error::UserFacing as _;
use a3s_types::constants::DEFAULT_PROFILE_NAME;
use a3s_types::local::{Configuration, Preferences, Profile, Repository};

use super::{Result, proxy_summary};
use crate::local::LocalStore;

/// The client's configuration, preferences, profiles and repositories, kept in memory and
/// persisted through a [`LocalStore`] on every change.
///
/// `Send + Sync`; mutators take `&mut self`, so a shared instance goes behind a lock.
#[derive(Debug)]
pub struct ClientData {
    pub(super) store: Arc<LocalStore>,
    pub(super) configuration: Configuration,
    pub(super) preferences: Preferences,

    /// Always holds `"Default"` (in memory only until it is first written).
    pub(super) profiles: BTreeMap<String, Profile>,

    /// File each profile was loaded from or last written to.
    pub(super) profile_sources: BTreeMap<String, PathBuf>,

    /// Names held by profile files that could not be loaded.
    pub(super) unreadable_profiles: BTreeSet<String>,
    pub(super) repositories: BTreeMap<String, Repository>,

    /// File each repository was loaded from or last written to.
    pub(super) repository_sources: BTreeMap<String, PathBuf>,

    /// Names held by repository files that could not be loaded.
    pub(super) unreadable_repositories: BTreeSet<String>,
}

/// The result of [`ClientData::load`]: the data, and the problems to show the user.
#[derive(Debug)]
pub struct LoadOutcome {
    pub data: ClientData,

    /// One message per failed file group, duplicate and failed start-up write.
    pub warnings: Vec<String>,
}

impl ClientData {
    /// Loads everything at start-up, in Java's order: configuration, preferences, profiles,
    /// repositories, then the proxy log line and the active-profile fix-up.
    ///
    /// Nothing fails: unreadable files become warnings and defaults. The only write is the
    /// configuration when its active profile does not exist (it is reset to `"Default"`).
    // Mirrors ui/main/MainPanel.java:457-490
    pub fn load(store: Arc<LocalStore>) -> LoadOutcome {
        let mut warnings = Vec::new();
        let mut warn = |message: String| {
            tracing::warn!("{message}");
            warnings.push(message);
        };

        // Mirrors service/ConfigurationService.java:33
        tracing::info!("Loading configuration...");
        let mut configuration = store.read_configuration().unwrap_or_else(|err| {
            warn(err.user_message());
            Configuration::default()
        });
        // Java never reads the preferences at start-up and overwrites them with the defaults
        // on exit; they are loaded here.
        let preferences = store.read_preferences().unwrap_or_else(|err| {
            warn(err.user_message());
            Preferences::default()
        });

        // Mirrors service/ProfileService.java:28
        tracing::info!("Loading profiles...");
        let (profiles, profile_sources, unreadable_profiles) = match store.load_profiles() {
            Ok(load) => {
                if let Some(message) = load.error_message() {
                    warn(message);
                }
                load.warnings.into_iter().for_each(&mut warn);
                (
                    load.profiles,
                    load.sources,
                    load.unreadable_names.into_values().collect(),
                )
            }
            Err(err) => {
                warn(err.user_message());
                let default = Profile::new(DEFAULT_PROFILE_NAME);
                (
                    BTreeMap::from([(DEFAULT_PROFILE_NAME.to_owned(), default)]),
                    BTreeMap::new(),
                    BTreeSet::new(),
                )
            }
        };

        // Mirrors service/RepositoryService.java:68
        tracing::info!("Loading repositories...");
        let mut repositories = BTreeMap::new();
        let mut repository_sources = BTreeMap::new();
        let mut unreadable_repositories = BTreeSet::new();
        match store.load_repositories() {
            Ok(load) => {
                if let Some(message) = load.error_message() {
                    warn(message);
                }
                load.duplicate_warnings.into_iter().for_each(&mut warn);
                unreadable_repositories = load.unreadable_names.into_values().collect();
                for (name, loaded) in load.repositories {
                    repository_sources.insert(name.clone(), loaded.source);
                    repositories.insert(name, loaded.repository);
                }
            }
            Err(err) => warn(err.user_message()),
        }

        // Mirrors service/ConfigurationService.java:358
        tracing::info!("Loading proxy configuration...");
        tracing::info!("{}", proxy_summary(&configuration.proxy));

        // Mirrors ui/main/MainPanel.java:481-490
        if !profiles.contains_key(&configuration.profile_name) {
            DEFAULT_PROFILE_NAME.clone_into(&mut configuration.profile_name);
            if let Err(err) = store.write_configuration(&configuration) {
                warn(err.user_message());
            }
        }
        configuration.sort_favorite_servers();

        LoadOutcome {
            data: Self {
                store,
                configuration,
                preferences,
                profiles,
                profile_sources,
                unreadable_profiles,
                repositories,
                repository_sources,
                unreadable_repositories,
            },
            warnings,
        }
    }

    /// Data built in memory; nothing is read or written. `"Default"` is added when `profiles`
    /// lacks it and the favorite servers are sorted, as after [`Self::load`].
    pub fn from_parts(
        store: Arc<LocalStore>,
        mut configuration: Configuration,
        preferences: Preferences,
        profiles: Vec<Profile>,
        repositories: Vec<Repository>,
    ) -> Self {
        let mut profiles: BTreeMap<String, Profile> =
            profiles.into_iter().map(|p| (p.name.clone(), p)).collect();
        profiles
            .entry(DEFAULT_PROFILE_NAME.to_owned())
            .or_insert_with(|| Profile::new(DEFAULT_PROFILE_NAME));
        configuration.sort_favorite_servers();
        Self {
            store,
            configuration,
            preferences,
            profiles,
            profile_sources: BTreeMap::new(),
            unreadable_profiles: BTreeSet::new(),
            repositories: repositories
                .into_iter()
                .map(|r| (r.name.clone(), r))
                .collect(),
            repository_sources: BTreeMap::new(),
            unreadable_repositories: BTreeSet::new(),
        }
    }

    /// The store the data is persisted to.
    pub fn store(&self) -> &Arc<LocalStore> {
        &self.store
    }

    /// Stores the window size, then writes the configuration, the preferences and the active
    /// profile, stopping at the first error.
    // Mirrors service/CommonService.java:41-52
    pub fn save_on_exit(&mut self, height: i32, width: i32) -> Result<()> {
        self.configuration.height = height;
        self.configuration.width = width;
        self.save_configuration()?;
        self.save_preferences()?;
        let active = self.configuration.profile_name.clone();
        if self.profiles.contains_key(&active) {
            self.save_profile(&active)?;
        }
        Ok(())
    }

    /// Writes the configuration as it is in memory.
    // Mirrors service/ConfigurationService.java:37-39
    pub fn save_configuration(&self) -> Result<()> {
        Ok(self.store.write_configuration(&self.configuration)?)
    }

    /// Writes the preferences as they are in memory.
    // Mirrors service/PreferencesService.java:17-19
    pub fn save_preferences(&self) -> Result<()> {
        Ok(self.store.write_preferences(&self.preferences)?)
    }

    /// Writes the profile `name` as it is in memory; an unknown name writes nothing.
    pub fn save_profile(&mut self, name: &str) -> Result<()> {
        if let Some(profile) = self.profiles.get(name).cloned() {
            self.commit_profile(profile)?;
        }
        Ok(())
    }

    /// Writes the repository `name` as it is in memory; an unknown name writes nothing.
    // Mirrors service/RepositoryService.java:72-78
    pub fn save_repository(&mut self, name: &str) -> Result<()> {
        if let Some(repository) = self.repositories.get(name).cloned() {
            self.commit_repository(repository)?;
        }
        Ok(())
    }

    /// Writes `profile` (replacing the file it came from), then keeps it in memory.
    pub(super) fn commit_profile(&mut self, profile: Profile) -> Result<()> {
        let source = self.profile_sources.get(&profile.name);
        self.store
            .write_profile_replacing(&profile, source.map(PathBuf::as_path))?;
        let name = profile.name.clone();
        self.profile_sources
            .insert(name.clone(), self.store.profile_path(&name));
        self.profiles.insert(name, profile);
        Ok(())
    }

    /// Writes `repository` (replacing the file it came from), then keeps it in memory.
    pub(super) fn commit_repository(&mut self, repository: Repository) -> Result<()> {
        let source = self.repository_sources.get(&repository.name);
        self.store
            .write_repository_replacing(&repository, source.map(PathBuf::as_path))?;
        let name = repository.name.clone();
        self.repository_sources
            .insert(name.clone(), self.store.repository_path(&name));
        self.repositories.insert(name, repository);
        Ok(())
    }

    /// Writes `configuration` with its favorite servers sorted, then keeps it in memory.
    pub(super) fn commit_configuration(&mut self, mut configuration: Configuration) -> Result<()> {
        configuration.sort_favorite_servers();
        self.store.write_configuration(&configuration)?;
        self.configuration = configuration;
        Ok(())
    }
}
