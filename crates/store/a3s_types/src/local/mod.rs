//! The client's local state: repositories, profiles, preferences and configuration.
//!
//! These are plain data types for the new TOML store (all derive serde; missing keys take the
//! Java field initializer values). Java getter semantics (connection clamps, the partial-transfer
//! inversion, null normalisation) are methods.
//!
//! [`legacy`] decodes the Java client's files (`a3s.cfg`, `a3s.prefs`, `*.a3s.profile`, the
//! decrypted `*.a3s.repository`, `export.a3s.autoconfig`) leniently, and [`AutoConfigExport`]
//! encodes `export.a3s.autoconfig` exactly as Java writes it (descriptors in [`wire`]).

mod addon;
mod configuration;
mod export;
pub mod legacy;
mod preferences;
mod profile;
pub mod protocol;
mod repository;
pub mod wire;

pub use addon::Addon;
pub use configuration::{
    Acre2Options, AcreOptions, BikeyExtractOptions, Configuration, ExternalApplication,
    LauncherOptions, Proxy, RptOptions, TfarOptions, favorite_server_java_cmp,
};
pub use export::AutoConfigExport;
pub use legacy::{
    Legacy, LegacyConfiguration, LegacyDecode, LegacyRepository, decode_legacy_stream,
};
pub use preferences::Preferences;
pub use profile::{PROFILE_TREE_ROOT_NAME, Profile, TreeDirectory, TreeLeaf, TreeNode};
pub use protocol::ProtocolCheckError;
pub use repository::{FileAttributes, Repository};

/// A setter rejected a value; `message` is the exact Java exception message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InvalidSetting {
    pub message: &'static str,
}
