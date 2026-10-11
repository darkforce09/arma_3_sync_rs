//! The client's local state in memory, [`ClientData`], shared by the CLI and the viewer.
//!
//! It holds the configuration, preferences, profiles and repositories, with the operations of
//! Java's `ConfigurationService`, `PreferencesService`, `ProfileService`, `CommonService` and the
//! CRUD part of `RepositoryService`.
//!
//! Every mutator persists the files it changes through the [`LocalStore`] before it returns
//! `Ok`. When validation, a closure or a write fails, memory is left as it was (multi-file
//! operations stop at the first failed write, and memory then matches what reached the disk).
//! Java instead mutates memory first, swallows write errors and leaves many changes to the
//! save on exit.
//!
//! [`LocalStore`]: crate::local::LocalStore

mod autoconfig;
mod client;
mod configuration;
mod error;
mod profiles;
mod repositories;

use a3s_types::ProtocolType;
use a3s_types::local::Proxy;

pub use client::{ClientData, LoadOutcome};
pub use error::ServiceError;

/// Result of the service operations.
pub type Result<T, E = ServiceError> = std::result::Result<T, E>;

/// The Arma3Sync wiki (help menu).
// Mirrors service/CommonService.java:35
pub const WIKI_URL: &str = "https://wiki.luna.vpzbrig21.de";

/// The Arma3Sync thread of the Bohemia Interactive forums (help menu).
// Mirrors service/CommonService.java:36-37
pub const BIS_FORUM_URL: &str = "https://forums.bistudio.com/showthread.php?162236-ArmA3Sync-launcher-and-addons-synchronization-software-for-ArmA-3&p=2477805#post2477805";

/// The Arma Discord server (help menu).
// Mirrors service/CommonService.java:38
pub const BIS_DISCORD_URL: &str = "https://discord.gg/arma";

/// The donation page (help menu).
// Mirrors service/CommonService.java:39
pub const PAYPAL_URL: &str = "https://paypal.me/sorotech";

/// The fields of a connection dialog (repository download or upload protocol, proxy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolInput {
    pub protocol_type: ProtocolType,

    /// `host[/path]`, without scheme.
    pub url: String,
    pub port: String,
    pub login: String,
    pub password: String,
    pub validate_ssl_certificate: bool,
}

/// The fields a favorite server edit overwrites; `selected` and `repository_name` are kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FavoriteServerEdit {
    pub name: Option<String>,
    pub ip_address: Option<String>,
    pub port: i32,
    pub password: Option<String>,
    pub modset_name: Option<String>,
}

/// What [`ClientData::import_autoconfig`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoConfigImportSummary {
    /// Profiles written, in file order (new and replaced).
    pub profiles_imported: Vec<String>,

    /// The subset of [`Self::profiles_imported`] that replaced an existing profile; its launcher
    /// options are now the defaults, as an export carries none.
    pub profiles_replaced: Vec<String>,
    pub favorite_servers_added: usize,

    /// Repositories added and written.
    pub repositories_added: Vec<String>,

    /// Repositories of the file whose name already exists; the existing one is kept untouched.
    pub repositories_skipped: Vec<String>,

    /// Decoder fallbacks and skipped elements.
    pub warnings: Vec<String>,
}

/// The name the profile dialog proposes for a duplicate of `name`, or `None` when `name` already
/// contains `"Duplicate"`.
// Mirrors ui/profiles/ProfileSelectionDialog.java:218-223
pub fn duplicate_profile_name(name: &str) -> Option<String> {
    (!name.contains("Duplicate")).then(|| format!("{name} - Duplicate"))
}

/// The proxy line logged at start-up.
///
/// `Proxy loaded: <scheme>://<host>:<port>` when a proxy is enabled (also for an FTP proxy,
/// which Java reports although it has no effect), otherwise `No proxy available.`.
// Mirrors service/ConfigurationService.java:368-407
pub fn proxy_summary(proxy: &Proxy) -> String {
    match &proxy.proxy_protocol {
        Some(protocol) if proxy.enable_proxy => format!(
            "Proxy loaded: {}{}:{}",
            protocol.protocol_type.prompt(),
            protocol.url,
            protocol.port
        ),
        _ => "No proxy available.".to_owned(),
    }
}
