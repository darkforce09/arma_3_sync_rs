//! Version comparison and supported-version syntax.

use a3s_updater::{is_newer, is_supported_version};

#[test]
fn numeric_dotted_comparison() {
    let newer = [
        ("2026.2.7", "2026.2.6"),
        ("2026.10.0", "2026.9.9"),
        ("0.1.0-alpha.1", "0.1.0"),
        ("1.0.0", ""),
        ("2026.3.0", "v2026.3.0"),
        (" 2026.2.7 ", "2026.2.6"),
    ];
    for (candidate, current) in newer {
        assert!(is_newer(candidate, current), "{candidate} > {current}");
        assert!(!is_newer(current, candidate), "{current} < {candidate}");
    }
    let equal = [
        ("2026.2.6", "2026.2.6.0"),
        ("2026.2.6-beta", "2026.2.6"),
        ("", ""),
        ("1.99999999999.0", "1.0.0"),
        ("1.2147483648.0", "1.0.0"),
        ("v2026.3.0", "0.3.0"),
    ];
    for (a, b) in equal {
        assert!(!is_newer(a, b), "{a} == {b}");
        assert!(!is_newer(b, a), "{b} == {a}");
    }
    assert!(is_newer("1.2147483647.0", "1.0.0"));
}

#[test]
fn supported_version_syntax() {
    for version in [
        "2026.2.1",
        "2026.2.1.4",
        "1.2.3-rc.1",
        "1.2.3+build-7",
        " 2026.2.1 ",
    ] {
        assert!(is_supported_version(version), "{version}");
    }
    for version in [
        "2026.2",
        "2026.2.1.4.5",
        "1.2.3-",
        "1.2.3-rc..1",
        "1.2.3-rc+1",
        "v1.2.3",
        "1..2.3",
        "\u{661}.\u{662}.\u{663}",
        "",
    ] {
        assert!(!is_supported_version(version), "{version}");
    }
}
