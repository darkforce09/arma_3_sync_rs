//! `updater.toml`: where updates come from and how they are fetched.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::UpdateError;

/// Base URL of the default JSON manifest and legacy XML.
// Mirrors a3sUpdater/dao/DataAccessConstants.java:23
pub const DEFAULT_BASE_URL: &str = "https://arma3sync.vpzbrig21.de/updates";

/// Default GitHub "latest release" API endpoint.
// Mirrors a3sUpdater/config/UpdateConfig.java:80-81
pub const DEFAULT_GITHUB_API_URL: &str =
    "https://api.github.com/repos/vpzbrig21/Arma3Sync/releases/latest";

/// Default release asset name. `{target}` selects the archive built for this platform; the Java
/// default (`Arma3Sync-{version}.zip`) names the Java release, which this application cannot run.
pub const DEFAULT_ASSET_PATTERN: &str = "Arma3Sync-{version}-{target}.zip";

const CONFIG_FILE_NAME: &str = "updater.toml";
const DEFAULT_TIMEOUT_MS: i64 = 30_000;
const MAX_TIMEOUT_MS: i64 = 300_000;

/// Values that replace or force configuration file values; the Rust form of Java's
/// `-Da3s.updater.*` system properties.
///
/// URL fields replace the file value when `Some`, even when empty. The two booleans can only
/// force `true`. `base_url` only changes the default manifest and legacy XML URLs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigOverrides {
    /// Directory whose `updater.toml` takes precedence over the installation's files.
    pub user_config_dir: Option<PathBuf>,
    pub base_url: Option<String>,
    pub manifest_url: Option<String>,
    pub legacy_xml_url: Option<String>,
    pub dev_manifest_url: Option<String>,
    pub dev_legacy_xml_url: Option<String>,
    pub github_api_url: Option<String>,
    pub github_dev_api_url: Option<String>,
    pub allow_http: bool,
    pub github_enabled: bool,
}

impl ConfigOverrides {
    /// Reads the `A3S_UPDATER_*` environment variables of the current process.
    pub fn from_env() -> Self {
        Self::from_env_with(|name| std::env::var(name).ok())
    }

    /// Builds overrides from an environment lookup (`None` = unset).
    ///
    /// `A3S_UPDATER_USER_CONFIG_PATH`, `A3S_UPDATER_URL`, `A3S_UPDATER_MANIFEST_URL`,
    /// `A3S_UPDATER_LEGACY_XML_URL`, `A3S_UPDATER_MANIFEST_URL_DEV`,
    /// `A3S_UPDATER_LEGACY_XML_URL_DEV`, `A3S_UPDATER_GITHUB_API_URL`,
    /// `A3S_UPDATER_GITHUB_API_URL_DEV` map to the fields of the same meaning;
    /// `A3S_UPDATER_ALLOW_HTTP` and `A3S_UPDATER_GITHUB_ENABLED` are true when equal to `true`
    /// ignoring ASCII case.
    // Mirrors a3sUpdater/config/UpdateConfig.java:49-51,57,67-89
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Self {
        let flag = |name: &str| get(name).is_some_and(|v| v.eq_ignore_ascii_case("true"));
        Self {
            user_config_dir: get("A3S_UPDATER_USER_CONFIG_PATH")
                .filter(|v| !v.trim().is_empty())
                .map(PathBuf::from),
            base_url: get("A3S_UPDATER_URL"),
            manifest_url: get("A3S_UPDATER_MANIFEST_URL"),
            legacy_xml_url: get("A3S_UPDATER_LEGACY_XML_URL"),
            dev_manifest_url: get("A3S_UPDATER_MANIFEST_URL_DEV"),
            dev_legacy_xml_url: get("A3S_UPDATER_LEGACY_XML_URL_DEV"),
            github_api_url: get("A3S_UPDATER_GITHUB_API_URL"),
            github_dev_api_url: get("A3S_UPDATER_GITHUB_API_URL_DEV"),
            allow_http: flag("A3S_UPDATER_ALLOW_HTTP"),
            github_enabled: flag("A3S_UPDATER_GITHUB_ENABLED"),
        }
    }
}

/// The effective updater configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateConfig {
    manifest_url: String,
    legacy_xml_url: String,
    dev_manifest_url: String,
    dev_legacy_xml_url: String,
    allow_http: bool,
    github_enabled: bool,
    github_api_url: String,
    github_dev_api_url: String,
    github_asset_pattern: String,
    github_dev_asset_pattern: String,
    connect_timeout: Duration,
    read_timeout: Duration,
}

impl UpdateConfig {
    /// Location of the configuration shipped with an installation.
    pub const RELATIVE_PATH: &str = "resources/configuration/updater.toml";

    /// Loads exactly one configuration file (no merging): `<user_config_dir>/updater.toml`, else
    /// `<install_dir>/updater.toml`, else `<install_dir>/resources/configuration/updater.toml`,
    /// the first that is a regular file; with none, every value is the default.
    // Mirrors a3sUpdater/config/UpdateConfig.java:46-55
    pub fn load(install_dir: &Path, overrides: &ConfigOverrides) -> Result<Self, UpdateError> {
        let candidates = overrides
            .user_config_dir
            .iter()
            .map(|dir| dir.join(CONFIG_FILE_NAME))
            .chain([
                install_dir.join(CONFIG_FILE_NAME),
                install_dir.join(Self::RELATIVE_PATH),
            ]);
        let Some(file) = candidates.into_iter().find(|path| path.is_file()) else {
            return Self::from_table(&toml::Table::new(), overrides);
        };
        let text = std::fs::read_to_string(&file).map_err(|e| UpdateError::Config {
            path: Some(file.clone()),
            message: e.to_string(),
        })?;
        Self::from_toml_str(&text, overrides).map_err(|err| match err {
            UpdateError::Config { message, .. } => UpdateError::Config {
                path: Some(file),
                message,
            },
            other => other,
        })
    }

    /// Builds the configuration from the text of an `updater.toml`.
    pub fn from_toml_str(text: &str, overrides: &ConfigOverrides) -> Result<Self, UpdateError> {
        let table: toml::Table = toml::from_str(text).map_err(|e| UpdateError::Config {
            path: None,
            message: e.to_string(),
        })?;
        Self::from_table(&table, overrides)
    }

    // Mirrors a3sUpdater/config/UpdateConfig.java:57-94
    fn from_table(table: &toml::Table, overrides: &ConfigOverrides) -> Result<Self, UpdateError> {
        let base = trim_slash(overrides.base_url.as_deref());
        let file = Values(table);

        let mut manifest_url =
            file.string("update", "manifest_url", &format!("{base}/a3s.json"))?;
        let mut legacy_xml_url =
            file.string("update", "legacy_xml_url", &format!("{base}/a3s.xml"))?;
        // Dev values fall back to the file values as read, before any override.
        let mut dev_manifest_url = file.string("update", "dev_manifest_url", &manifest_url)?;
        let mut dev_legacy_xml_url =
            file.string("update", "dev_legacy_xml_url", &legacy_xml_url)?;
        if dev_manifest_url.trim().is_empty() {
            dev_manifest_url.clone_from(&manifest_url);
        }
        if dev_legacy_xml_url.trim().is_empty() {
            dev_legacy_xml_url.clone_from(&legacy_xml_url);
        }
        replace_if_some(&mut manifest_url, overrides.manifest_url.as_ref());
        replace_if_some(&mut legacy_xml_url, overrides.legacy_xml_url.as_ref());
        replace_if_some(&mut dev_manifest_url, overrides.dev_manifest_url.as_ref());
        replace_if_some(
            &mut dev_legacy_xml_url,
            overrides.dev_legacy_xml_url.as_ref(),
        );

        let allow_http = overrides.allow_http || file.boolean("update", "allow_http", false)?;
        let github_enabled = overrides.github_enabled || file.boolean("github", "enabled", true)?;
        let mut github_api_url = file.string("github", "api_url", DEFAULT_GITHUB_API_URL)?;
        let mut github_dev_api_url = file.string("github", "dev_api_url", &github_api_url)?;
        let github_asset_pattern = file.string("github", "asset_pattern", DEFAULT_ASSET_PATTERN)?;
        let github_dev_asset_pattern =
            file.string("github", "dev_asset_pattern", &github_asset_pattern)?;
        replace_if_some(&mut github_api_url, overrides.github_api_url.as_ref());
        replace_if_some(
            &mut github_dev_api_url,
            overrides.github_dev_api_url.as_ref(),
        );

        Ok(Self {
            manifest_url,
            legacy_xml_url,
            dev_manifest_url,
            dev_legacy_xml_url,
            allow_http,
            github_enabled,
            github_api_url,
            github_dev_api_url,
            github_asset_pattern,
            github_dev_asset_pattern,
            connect_timeout: file.timeout("update", "connect_timeout_ms")?,
            read_timeout: file.timeout("update", "read_timeout_ms")?,
        })
    }

    /// URL of the JSON manifest (`a3s.json`); blank disables that source.
    pub fn manifest_url(&self, dev: bool) -> &str {
        if dev {
            &self.dev_manifest_url
        } else {
            &self.manifest_url
        }
    }

    /// URL of the legacy `a3s.xml`; blank disables that source.
    pub fn legacy_xml_url(&self, dev: bool) -> &str {
        if dev {
            &self.dev_legacy_xml_url
        } else {
            &self.legacy_xml_url
        }
    }

    /// Whether the GitHub Releases source is consulted.
    pub fn github_enabled(&self) -> bool {
        self.github_enabled
    }

    /// GitHub "latest release" API URL.
    pub fn github_api_url(&self, dev: bool) -> &str {
        if dev {
            &self.github_dev_api_url
        } else {
            &self.github_api_url
        }
    }

    /// Release asset name pattern with `{version}`, `{tag}` and `{target}` placeholders.
    pub fn github_asset_pattern(&self, dev: bool) -> &str {
        if dev {
            &self.github_dev_asset_pattern
        } else {
            &self.github_asset_pattern
        }
    }

    /// Whether plain `http://` URLs are accepted (local tests only).
    pub fn allow_http(&self) -> bool {
        self.allow_http
    }

    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// Inactivity timeout while reading a response.
    pub fn read_timeout(&self) -> Duration {
        self.read_timeout
    }
}

/// Typed getters over the parsed file; unknown keys and sections are ignored.
struct Values<'a>(&'a toml::Table);

impl Values<'_> {
    fn get(&self, section: &str, key: &str) -> Option<&toml::Value> {
        self.0.get(section)?.as_table()?.get(key)
    }

    fn error(section: &str, key: &str, what: &str) -> UpdateError {
        UpdateError::Config {
            path: None,
            message: format!("{what}: {section}.{key}"),
        }
    }

    // Mirrors a3sUpdater/config/TomlConfigDAO.java:37-45
    fn string(&self, section: &str, key: &str, fallback: &str) -> Result<String, UpdateError> {
        match self.get(section, key) {
            None => Ok(fallback.to_owned()),
            Some(toml::Value::String(value)) => Ok(value.clone()),
            Some(_) => Err(Self::error(
                section,
                key,
                "TOML value must be a quoted string",
            )),
        }
    }

    // Mirrors a3sUpdater/config/TomlConfigDAO.java:47-53
    fn boolean(&self, section: &str, key: &str, fallback: bool) -> Result<bool, UpdateError> {
        match self.get(section, key) {
            None => Ok(fallback),
            Some(toml::Value::Boolean(value)) => Ok(*value),
            Some(_) => Err(Self::error(
                section,
                key,
                "TOML value must be true or false",
            )),
        }
    }

    // Mirrors a3sUpdater/config/TomlConfigDAO.java:55-65
    fn timeout(&self, section: &str, key: &str) -> Result<Duration, UpdateError> {
        let millis = match self.get(section, key) {
            None => DEFAULT_TIMEOUT_MS,
            Some(toml::Value::Integer(value)) if (1..=MAX_TIMEOUT_MS).contains(value) => *value,
            Some(_) => return Err(Self::error(section, key, "Invalid positive TOML integer")),
        };
        Ok(Duration::from_millis(millis.unsigned_abs()))
    }
}

/// The base URL without one trailing `/`; blank means the default.
// Mirrors a3sUpdater/config/UpdateConfig.java:108-111
fn trim_slash(base: Option<&str>) -> &str {
    match base {
        Some(value) if !value.trim().is_empty() => value.strip_suffix('/').unwrap_or(value),
        _ => DEFAULT_BASE_URL,
    }
}

fn replace_if_some(value: &mut String, replacement: Option<&String>) {
    if let Some(replacement) = replacement {
        value.clone_from(replacement);
    }
}
