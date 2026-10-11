//! File-name stems of profiles and repositories (`dao/repository/RepositoryDAO.java:313-363`).

use sha1::{Digest as _, Sha1};

/// Bytes of the name's SHA-1 used in a file stem (`HASH_PREFIX_BYTES`).
// Mirrors dao/repository/RepositoryDAO.java:41
const HASH_PREFIX_BYTES: usize = 6;

/// Maximum length in bytes of the sanitized part of a Rust file stem.
const MAX_SANITIZED_BYTES: usize = 64;

/// Name used when sanitizing leaves nothing (or the name is null in Java).
const FALLBACK_NAME: &str = "repository";

/// Lowercase hex of the first 6 bytes of `SHA-1(UTF-8(name))`: 12 characters.
///
/// The untrimmed name is hashed.
// Mirrors dao/repository/RepositoryDAO.java:344-359
pub fn name_hash(name: &str) -> String {
    let digest = Sha1::digest(name.as_bytes());
    crate::hash::to_hex(&digest[..HASH_PREFIX_BYTES])
}

pub(crate) use a3s_types::java_string::{java_is_blank, java_trim};

/// Java's `sanitizeRepositoryName`, without the Rust additions of [`sanitize`].
// Mirrors dao/repository/RepositoryDAO.java:330-342
pub(crate) fn sanitize_java(name: &str) -> String {
    let trimmed = java_trim(name);
    let mut out = String::with_capacity(trimmed.len());
    let mut in_whitespace = false;
    for c in trimmed.chars() {
        // `\s+` → `_` (Java `\s` is ASCII only).
        if matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r') {
            if !in_whitespace {
                out.push('_');
            }
            in_whitespace = true;
            continue;
        }
        in_whitespace = false;
        // `[\\/:*?"<>|]` → `_`.
        if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        out
    }
}

/// Sanitized, file-system-safe form of `name`.
///
/// Java's rules (trim, whitespace runs and `\ / : * ? " < > |` become `_`, empty becomes
/// `"repository"`), plus two local-only additions: remaining control
/// characters (`< U+0020`, `U+007F`) become `_`, and the result is truncated to 64 bytes on a
/// char boundary.
pub fn sanitize(name: &str) -> String {
    let java = sanitize_java(name);
    let mut out = String::with_capacity(java.len().min(MAX_SANITIZED_BYTES));
    for c in java.chars() {
        let c = if c < '\u{20}' || c == '\u{7F}' {
            '_'
        } else {
            c
        };
        if out.len() + c.len_utf8() > MAX_SANITIZED_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

/// File stem of the Rust store: `sanitize(name) + "__" + name_hash(name)`.
pub fn file_stem(name: &str) -> String {
    format!("{}__{}", sanitize(name), name_hash(name))
}

/// Java's preferred stem, without the Rust additions.
// Mirrors dao/repository/RepositoryDAO.java:313-320
pub(crate) fn file_stem_java(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{}__{}", sanitize_java(name), name_hash(name)),
        None => format!("{FALLBACK_NAME}__000000"),
    }
}
