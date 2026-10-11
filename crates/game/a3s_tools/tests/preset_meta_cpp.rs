//! The `meta.cpp` scanner against Java's multiline regex semantics.

use a3s_tools::preset::meta_cpp::{
    name_raw, published_id_any, published_id_strict, unescape_meta_value,
};

#[test]
fn assignments_start_after_every_java_line_terminator() {
    for separator in ["\n", "\r\n", "\r", "\u{2028}", "\u{85}", "\u{2029}"] {
        let text = format!("protocol = 1;{separator}publishedid = 42;{separator}");
        assert_eq!(published_id_strict(&text), Some("42"), "{separator:?}");
        assert_eq!(published_id_any(&text), Some("42"), "{separator:?}");
    }
}

#[test]
fn whitespace_may_span_lines() {
    assert_eq!(published_id_strict("\n\n   \tpublishedid = 7;"), Some("7"));
    assert_eq!(published_id_strict("publishedid =\n 123\n;"), Some("123"));
    assert_eq!(name_raw("\n  name\n=\n\"x\"\n;"), Some("x"));
}

#[test]
fn keyword_is_case_sensitive_and_must_start_the_line() {
    assert_eq!(published_id_any("Publishedid = 1;"), None);
    assert_eq!(published_id_any("x publishedid = 1;"), None);
    assert_eq!(published_id_any("publishedids = 1;"), None);
}

#[test]
fn strict_and_any_differ_on_zero_and_leading_zeros() {
    let text = "publishedid = 0;\npublishedid = 5;";
    assert_eq!(published_id_strict(text), Some("5"));
    assert_eq!(published_id_any(text), Some("0"));
    assert_eq!(published_id_strict("publishedid = 0123;"), None);
    assert_eq!(published_id_any("publishedid = 0123;"), Some("0123"));
}

#[test]
fn value_must_be_followed_by_a_semicolon() {
    assert_eq!(published_id_any("publishedid = 12x;"), None);
    assert_eq!(published_id_any("publishedid = 12"), None);
    assert_eq!(
        published_id_any("publishedid = 12 x;\npublishedid = 13;"),
        Some("13")
    );
}

#[test]
fn byte_order_mark_hides_the_first_line() {
    assert_eq!(published_id_any("\u{FEFF}publishedid = 1;"), None);
    assert_eq!(
        published_id_any("\u{FEFF}publishedid = 1;\npublishedid = 2;"),
        Some("2")
    );
}

#[test]
fn name_escapes() {
    let raw = name_raw(r#"name = "a\"b";"#).unwrap();
    assert_eq!(raw, r#"a\"b"#);
    assert_eq!(unescape_meta_value(raw), "a\"b");

    let raw = name_raw(r#"name = "a\\nb";"#).unwrap();
    assert_eq!(unescape_meta_value(raw), "a\nb");

    assert_eq!(name_raw("name = \"x\\\ny\";"), None);
    assert_eq!(name_raw("name = \"x\\"), None);
    assert_eq!(name_raw("name = \"x\ny\";"), Some("x\ny"));
}

#[test]
fn incomplete_name_falls_through_to_a_later_line() {
    assert_eq!(name_raw("name = \"x\"\nname = \"y\";"), Some("y"));
    assert_eq!(name_raw("name = x;"), None);
}

#[test]
fn unescape_runs_three_sequential_replacements() {
    assert_eq!(unescape_meta_value(r"a\\b"), r"a\b");
    assert_eq!(unescape_meta_value(r#"q\"q"#), "q\"q");
    assert_eq!(unescape_meta_value(r"l\nl"), "l\nl");
    assert_eq!(unescape_meta_value(r"t\t"), r"t\t");
}
