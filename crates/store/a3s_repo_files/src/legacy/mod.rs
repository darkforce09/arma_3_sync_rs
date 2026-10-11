//! Read-only access to the Java client's data, and the one-time import into the [`crate::local`]
//! store.
//!
//! The Java client keeps `a3s.cfg`, `a3s.prefs` and `*.a3s.profile` as
//! `GZIP(ObjectOutputStream(object))`, filtered by a class allowlist on reading, and each
//! repository as `GZIP(ObjectOutputStream(SealedObject))` where the sealed content is the
//! AES-128/ECB/PKCS5 encryption (hard-coded key) of an uncompressed `ObjectOutputStream` of the
//! `Repository` (`dao/A3SFilesAccessor.java`, `dao/EncryptionProvider.java`). The Workshop cache
//! is a `java.util.Properties` file.
//!
//! [`import`] copies everything [`a3s_paths::legacy`] discovers into the Rust store. It never
//! touches the Java files and never overwrites Rust data.

mod allowlist;
pub mod crypto;
mod error;
mod files;
mod import;
pub mod naming;
pub mod properties;
pub mod sealed;
mod workshop;

pub use allowlist::{Rejected, check_java_allowlist};
pub use error::{DeserializeError, Error, FILE_CORRUPTED};
pub use files::{
    LegacyFile, read_configuration_file, read_object_file, read_preferences_file,
    read_profile_file, read_repository_file,
};
pub use import::{
    ImportError, ImportOptions, ImportReport, ItemOutcome, NamedOutcome, SourceReport, import,
    should_offer_import,
};
pub use workshop::read_workshop_file;
