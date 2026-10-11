use std::path::Path;

use a3s_types::local::Addon;

use crate::registry::AddonRegistry;

/// Java regex `\s` without `UNICODE_CHARACTER_CLASS`.
fn is_java_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// Whether Java's multiline `^` matches before byte `i` of `text`: at the start, or after a line
/// terminator (but not between `\r` and `\n`), never at the end.
fn is_line_start(text: &str, i: usize) -> bool {
    if i == text.len() {
        return false;
    }
    if i == 0 {
        return true;
    }
    let prev = text[..i].chars().next_back();
    match prev {
        Some('\r') => !text[i..].starts_with('\n'),
        Some('\n' | '\u{85}' | '\u{2028}' | '\u{2029}') => true,
        _ => false,
    }
}

/// Matches `\s*publishedid\s*=\s*([1-9][0-9]*)\s*;` at the start of `s`; returns the digits.
fn match_published_id(s: &str) -> Option<&str> {
    let s = s.trim_start_matches(is_java_space);
    let s = s.strip_prefix("publishedid")?;
    let s = s.trim_start_matches(is_java_space);
    let s = s.strip_prefix('=')?;
    let s = s.trim_start_matches(is_java_space);
    if !s.starts_with(|c: char| ('1'..='9').contains(&c)) {
        return None;
    }
    let digits_len = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (digits, rest) = s.split_at(digits_len);
    rest.trim_start_matches(is_java_space)
        .starts_with(';')
        .then_some(digits)
}

/// The Steam Workshop id of a mod's `meta.cpp`.
///
/// It is the digits of the first match of the Java regex
/// `(?m)^\s*publishedid\s*=\s*([1-9][0-9]*)\s*;` (ASCII `\s`, which may span lines;
/// `publishedid` is case-sensitive; a later `publishedid` line is never consulted once one
/// matches).
// Mirrors service/AddonService.java:37-38 and :581-582
pub fn parse_published_id(meta_cpp: &str) -> Option<&str> {
    meta_cpp
        .char_indices()
        .map(|(i, _)| i)
        .filter(|&i| is_line_start(meta_cpp, i))
        .find_map(|i| match_published_id(&meta_cpp[i..]))
}

/// The first addon, in registry order, whose `<path>/<name>/meta.cpp` is a regular UTF-8 file
/// with Workshop id `id`. Addons whose `meta.cpp` is missing, unreadable or not UTF-8 are
/// skipped.
// Mirrors service/AddonService.java:571-590
pub fn find_addon_by_workshop_id<'r>(registry: &'r AddonRegistry, id: &str) -> Option<&'r Addon> {
    registry.addons().find(|addon| {
        let meta = Path::new(&addon.path).join(&addon.name).join("meta.cpp");
        if !meta.is_file() {
            return false;
        }
        match std::fs::read_to_string(&meta) {
            Ok(text) => parse_published_id(&text) == Some(id),
            Err(e) => {
                tracing::debug!(path = %meta.display(), error = %e, "cannot read meta.cpp");
                false
            }
        }
    })
}
