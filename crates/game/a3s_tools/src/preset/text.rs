//! Java string and regex semantics the preset code depends on.

use std::sync::LazyLock;

use a3s_types::java_string::java_trim;
use regex::{Captures, Regex};

/// Java regex `\s` without `UNICODE_CHARACTER_CLASS`: exactly `[ \t\n\x0B\f\r]`.
pub(crate) fn is_regex_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// A Java line terminator, which `(?m)^` follows and `.` (without `DOTALL`) never matches.
pub(crate) fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// Compiles one of the fixed preset patterns (constants, compiled by every import test).
pub(crate) fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("preset pattern compiles")
}

// Mirrors service/Arma3LauncherPresetImporter.java:42
static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?s)<[^>]+>"));

// Mirrors service/Arma3LauncherPresetImporter.java:43-44
static HTML_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)&(#x?[0-9a-f]+|amp|quot|apos|lt|gt);"));

/// Text of an HTML fragment: tags removed, the five named and the numeric entities decoded,
/// whitespace runs collapsed to one space, then Java-trimmed.
///
/// Tags are removed before entities are decoded, so `&lt;b&gt;` survives as `<b>`, and decoded
/// text is not scanned again (`&amp;lt;` gives `&lt;`). A numeric entity decodes to its code
/// point; two adjacent surrogate entities combine into one character, a lone surrogate becomes
/// U+FFFD, and one that is not a number or above U+10FFFF stays as written.
// Mirrors service/Arma3LauncherPresetImporter.java:225-244
pub(crate) fn clean_text(value: &str) -> String {
    let text = HTML_TAG.replace_all(value, "");
    let decoded = decode_entities(&text);
    let mut collapsed = String::with_capacity(decoded.len());
    let mut in_whitespace = false;
    for c in decoded.chars() {
        if is_regex_whitespace(c) {
            if !in_whitespace {
                collapsed.push(' ');
            }
            in_whitespace = true;
        } else {
            collapsed.push(c);
            in_whitespace = false;
        }
    }
    java_trim(&collapsed).to_owned()
}

/// What one entity stands for.
enum Entity {
    Text(&'static str),
    Code(u32),
    Literal,
}

fn entity(captures: &Captures<'_>) -> Entity {
    let body = captures[1].to_ascii_lowercase();
    match body.as_str() {
        "amp" => Entity::Text("&"),
        "quot" => Entity::Text("\""),
        "apos" => Entity::Text("'"),
        "lt" => Entity::Text("<"),
        "gt" => Entity::Text(">"),
        _ => {
            let code = if let Some(hex) = body.strip_prefix("#x") {
                u32::from_str_radix(hex, 16).ok()
            } else if let Some(decimal) = body.strip_prefix('#') {
                decimal.parse::<u32>().ok()
            } else {
                None
            };
            match code {
                Some(code) if code <= 0x10_FFFF => Entity::Code(code),
                _ => Entity::Literal,
            }
        }
    }
}

fn decode_entities(text: &str) -> String {
    let matches: Vec<Captures<'_>> = HTML_ENTITY.captures_iter(text).collect();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut index = 0;
    while let Some(captures) = matches.get(index) {
        let whole = captures.get(0).map_or(0..0, |m| m.range());
        out.push_str(&text[last..whole.start]);
        last = whole.end;
        index += 1;
        match entity(captures) {
            Entity::Text(replacement) => out.push_str(replacement),
            Entity::Literal => out.push_str(&text[whole]),
            Entity::Code(high @ 0xD800..=0xDBFF) => {
                let low = matches.get(index).and_then(|next| {
                    let next_range = next.get(0).map_or(0..0, |m| m.range());
                    match entity(next) {
                        Entity::Code(low @ 0xDC00..=0xDFFF) if next_range.start == last => {
                            Some((low, next_range.end))
                        }
                        _ => None,
                    }
                });
                match low {
                    Some((low, end)) => {
                        let combined = 0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                        out.push(char::from_u32(combined).unwrap_or(char::REPLACEMENT_CHARACTER));
                        last = end;
                        index += 1;
                    }
                    None => out.push(char::REPLACEMENT_CHARACTER),
                }
            }
            Entity::Code(code) => {
                out.push(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
    }
    out.push_str(&text[last..]);
    out
}
