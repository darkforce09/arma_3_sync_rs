//! Configuration and preferences (`dao/ConfigurationDAO.java`, `dao/PreferencesDAO.java`).

use a3s_types::local::{Configuration, Preferences};

use super::{Error, LocalStore, toml_file};

impl LocalStore {
    /// Reads `config/configuration.toml`; the defaults when it does not exist.
    ///
    /// The caller logs an error and keeps the defaults, as Java does.
    // Mirrors dao/ConfigurationDAO.java:24-37
    pub fn read_configuration(&self) -> Result<Configuration, Error> {
        Ok(toml_file::read(&self.configuration_path(), "configuration")?.unwrap_or_default())
    }

    /// Writes `config/configuration.toml` atomically.
    // Mirrors dao/ConfigurationDAO.java:39-54
    pub fn write_configuration(&self, configuration: &Configuration) -> Result<(), Error> {
        let _guard = self.write_lock.lock();
        toml_file::write(&self.configuration_path(), "configuration", configuration)
    }

    /// Reads `config/preferences.toml`; the defaults when it does not exist.
    // Mirrors dao/PreferencesDAO.java:15-28
    pub fn read_preferences(&self) -> Result<Preferences, Error> {
        Ok(toml_file::read(&self.preferences_path(), "preferences")?.unwrap_or_default())
    }

    /// Writes `config/preferences.toml` atomically.
    // Mirrors dao/PreferencesDAO.java:30-45
    pub fn write_preferences(&self, preferences: &Preferences) -> Result<(), Error> {
        let _guard = self.write_lock.lock();
        toml_file::write(&self.preferences_path(), "preferences", preferences)
    }
}
