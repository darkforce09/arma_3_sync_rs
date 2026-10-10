use serde::{Deserialize, Serialize};

/// An addon found on disk (`fr.soe.a3s.domain.Addon`). Not persisted by Java.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Addon {
    /// Unique key (normally the lowercase name).
    pub key: String,
    pub name: String,

    /// Absolute path of the addon folder.
    pub path: String,

    /// Whether the addon lives directly in the Arma 3 installation folder.
    pub at_arma3_install_root: bool,
}

impl Addon {
    // Mirrors domain/Addon.java:10-14
    pub fn new(key: impl Into<String>, name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            path: path.into(),
            at_arma3_install_root: false,
        }
    }
}
