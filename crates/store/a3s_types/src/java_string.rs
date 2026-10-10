//! Java `String` semantics that affect on-disk ordering.

use std::cmp::Ordering;

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
