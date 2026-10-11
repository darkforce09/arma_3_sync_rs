//! The Java client's file names, for diagnostics and tests.

use crate::layout::{PROFILE_EXTENSION, REPOSITORY_EXTENSION};
use crate::local::file_stem_java;

/// Current Java name of a repository file: `sanitize(name) + "__" + hash + ".a3s.repository"`,
/// without the Rust store's additions; `"repository__000000.a3s.repository"` for a null name.
// Mirrors dao/repository/RepositoryDAO.java:313-320
pub fn java_preferred_repository_file_name(name: Option<&str>) -> String {
    format!("{}{REPOSITORY_EXTENSION}", file_stem_java(name))
}

/// Pre-hash Java name of a repository file: the name without its U+0020 spaces (other
/// whitespace is kept); `"repository.a3s.repository"` for a null name.
// Mirrors dao/repository/RepositoryDAO.java:322-328
pub fn java_legacy_repository_file_name(name: Option<&str>) -> String {
    let base = name.map_or_else(|| "repository".to_owned(), |n| n.replace(' ', ""));
    format!("{base}{REPOSITORY_EXTENSION}")
}

/// Java name of a profile file: the unsanitized name plus `".a3s.profile"`.
// Mirrors dao/ProfileDAO.java:72
pub fn java_profile_file_name(name: &str) -> String {
    format!("{name}{PROFILE_EXTENSION}")
}
