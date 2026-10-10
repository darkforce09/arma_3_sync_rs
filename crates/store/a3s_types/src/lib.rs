//! Arma3Sync domain types and their exact Java wire mappings.
//!
//! The Rust types here are plain data. Their [`JavaCodec`] implementations convert them to and
//! from [`a3s_jserial::Graph`]s that are indistinguishable from what the Java implementation
//! writes: same class names, `serialVersionUID`s, field descriptors and instance sharing.
//!
//! **Never** change a class name, UID or field list in [`wire`] — Java clients would reject the
//! files.

mod auto_config;
mod changelog;
pub mod constants;
mod error;
mod event;
mod favorite_server;
pub mod java_string;
pub mod local;
mod protocol;
mod server_info;
mod sync_tree;
pub mod wire;

pub use auto_config::AutoConfig;
pub use changelog::{Changelog, Changelogs};
pub use error::{Error, Result};
pub use event::{Event, Events};
pub use favorite_server::FavoriteServer;
pub use protocol::{EncryptionMode, Protocol, ProtocolType};
pub use server_info::ServerInfo;
pub use sync_tree::{SYNC_ROOT_NAME, SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};
pub use wire::{JavaCodec, decode_stream, encode_stream};

/// Milliseconds since the Unix epoch, as stored in a `java.util.Date`.
pub type JavaDate = i64;
