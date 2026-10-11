//! The `publishedid` and `name` assignments of an addon's `meta.cpp`, found exactly where
//! Java's multiline regexes find them.
//!
//! An assignment counts only at the start of a line (after any Java line terminator: LF, CRLF,
//! CR, U+0085, U+2028, U+2029), optionally indented and spread over several lines, and only
//! when it ends with `;`. The first line start that holds a complete assignment wins. A UTF-8 BOM
//! is not whitespace, so an assignment on the first line of a file with a BOM is not found, as
//! in Java.

use super::text::{is_line_terminator, is_regex_whitespace};

/// The importer's `publishedid`: a decimal number without a leading zero.
// Mirrors service/Arma3LauncherPresetImporter.java:45-46
pub fn published_id_strict(text: &str) -> Option<&str> {
    assignment(text, "publishedid", |text, pos| {
        let bytes = text.as_bytes();
        if !matches!(bytes.get(pos), Some(b'1'..=b'9')) {
            return None;
        }
        let end = digits_end(text, pos);
        Some((pos, end, end))
    })
}

/// The exporter's `publishedid`: any run of decimal digits, including `0` and leading zeros.
// Mirrors service/Arma3LauncherPresetExporter.java:35-36
pub fn published_id_any(text: &str) -> Option<&str> {
    assignment(text, "publishedid", |text, pos| {
        let end = digits_end(text, pos);
        (end > pos).then_some((pos, end, end))
    })
}

/// The raw (still escaped) value of `name = "…";`.
///
/// Inside the quotes a backslash escapes the next character unless that is a line terminator;
/// raw line breaks are accepted.
// Mirrors service/Arma3LauncherPresetImporter.java:47-48
// Mirrors service/Arma3LauncherPresetExporter.java:37-38
pub fn name_raw(text: &str) -> Option<&str> {
    assignment(text, "name", |text, pos| {
        let body_start = pos + text[pos..].strip_prefix('"').map(|_| 1)?;
        let mut chars = text[body_start..].char_indices();
        loop {
            match chars.next()? {
                (offset, '"') => {
                    let end = body_start + offset;
                    return Some((body_start, end, end + 1));
                }
                (_, '\\') => {
                    let (_, escaped) = chars.next()?;
                    if is_line_terminator(escaped) {
                        return None;
                    }
                }
                _ => {}
            }
        }
    })
}

/// Java's three sequential replacements: `\\` → `\`, then `\"` → `"`, then `\n` → LF. Because
/// they run one after the other, `a\\nb` becomes `a`, LF, `b`.
// Mirrors service/Arma3LauncherPresetImporter.java:252-254
pub fn unescape_meta_value(raw: &str) -> String {
    raw.replace("\\\\", "\\")
        .replace("\\\"", "\"")
        .replace("\\n", "\n")
}

/// The value of the first `<keyword> = <value> ;` that starts a line. `value` gets the offset
/// after `=` and its whitespace and returns the value's start, end and the offset after it.
fn assignment<'t>(
    text: &'t str,
    keyword: &str,
    value: impl Fn(&'t str, usize) -> Option<(usize, usize, usize)>,
) -> Option<&'t str> {
    line_starts(text).find_map(|start| {
        let mut pos = skip_whitespace(text, start);
        pos += text[pos..].strip_prefix(keyword).map(|_| keyword.len())?;
        pos = skip_whitespace(text, pos);
        pos += text[pos..].strip_prefix('=').map(|_| 1)?;
        pos = skip_whitespace(text, pos);
        let (value_start, value_end, after) = value(text, pos)?;
        pos = skip_whitespace(text, after);
        text[pos..]
            .starts_with(';')
            .then(|| &text[value_start..value_end])
    })
}

/// Offsets where Java's `(?m)^` matches: the start, and after every line terminator except
/// between the CR and LF of a CRLF.
fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.char_indices().filter_map(move |(offset, c)| {
        let next = offset + c.len_utf8();
        let inside_crlf = c == '\r' && text[next..].starts_with('\n');
        (is_line_terminator(c) && !inside_crlf).then_some(next)
    }))
}

fn skip_whitespace(text: &str, pos: usize) -> usize {
    pos + text[pos..]
        .chars()
        .take_while(|&c| is_regex_whitespace(c))
        .count()
}

fn digits_end(text: &str, pos: usize) -> usize {
    pos + text[pos..].bytes().take_while(u8::is_ascii_digit).count()
}
