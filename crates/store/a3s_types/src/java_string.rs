//! Java `String` semantics that affect on-disk ordering and parsing.

use std::cmp::Ordering;

/// `java.lang.String.trim`: strips every char `<= U+0020` at either end (not Unicode
/// whitespace).
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= '\u{20}')
}

/// `java.lang.Character.isWhitespace`.
///
/// The Unicode space, line and paragraph separators except the non-breaking ones (U+00A0,
/// U+2007, U+202F), plus `\t`, `\n`, U+000B, `\f`, `\r` and U+001C–U+001F. U+0085 is not
/// whitespace.
pub fn java_is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{0B}'
            | '\u{0C}'
            | '\r'
            | '\u{1C}'..='\u{1F}'
            | ' '
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// `java.lang.String.isBlank`: empty or only [`java_is_whitespace`] chars.
pub fn java_is_blank(s: &str) -> bool {
    s.chars().all(java_is_whitespace)
}

/// `java.lang.String.compareToIgnoreCase`.
///
/// Compares UTF-16 code units after `Character.toUpperCase` then `Character.toLowerCase`, which
/// is how Arma3Sync orders sync tree children. Approximated with Rust's per-`char` case mapping,
/// which agrees with Java for all single-unit mappings.
pub fn compare_ignore_case(a: &str, b: &str) -> Ordering {
    let fold = |c: char| -> char {
        let upper = single(c.to_uppercase()).unwrap_or(c);
        single(upper.to_lowercase()).unwrap_or(upper)
    };
    let mut ai = a.chars().map(fold).flat_map(utf16);
    let mut bi = b.chars().map(fold).flat_map(utf16);
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x != y => return x.cmp(&y),
            _ => {}
        }
    }
}

fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

fn utf16(c: char) -> impl Iterator<Item = u16> {
    let mut buf = [0u16; 2];
    let units = c.encode_utf16(&mut buf).to_vec();
    units.into_iter()
}

#[cfg(test)]
mod tests {
    use super::compare_ignore_case;
    use std::cmp::Ordering;

    #[test]
    fn orders_like_java() {
        assert_eq!(compare_ignore_case("abc", "ABC"), Ordering::Equal);
        assert_eq!(compare_ignore_case("@ace", "@CBA_A3"), Ordering::Less);
        assert_eq!(compare_ignore_case("a", "ab"), Ordering::Less);
        // Letters fold to lowercase (0x61..), so '_' (0x5F) sorts before them.
        assert_eq!(compare_ignore_case("_x", "a"), Ordering::Less);
        assert_eq!(compare_ignore_case("Z", "a"), Ordering::Greater);
    }
}
