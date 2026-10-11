//! File-name stems of the local store and Java's file names (A1).

use a3s_repo_files::legacy::naming::{
    java_legacy_repository_file_name, java_preferred_repository_file_name, java_profile_file_name,
};
use a3s_repo_files::local::{file_stem, name_hash, sanitize};

#[test]
fn a1_name_hash_is_six_sha1_bytes() {
    assert_eq!(name_hash("WOG"), "fe52247a11ac");
    assert_eq!(name_hash("Default"), "808d7dca8a74");
    // The untrimmed name is hashed.
    assert_ne!(name_hash(" WOG"), name_hash("WOG"));
}

#[test]
fn a1_file_stem_examples() {
    assert_eq!(file_stem("WOG"), "WOG__fe52247a11ac");
    assert_eq!(file_stem("My Repo"), "My_Repo__339879ebfb9b");
    assert_eq!(file_stem("  a/b:c*?  "), "a_b_c____b3a64f4e4ad1");
    assert_eq!(file_stem("Default"), "Default__808d7dca8a74");
    assert!(file_stem("   ").starts_with("repository__"));
    assert!(file_stem("").starts_with("repository__"));
}

#[test]
fn a1_whitespace_runs_collapse() {
    assert_eq!(sanitize("a\t\n b"), "a_b");
    assert_eq!(sanitize("a\u{0B}\u{0C}\rb"), "a_b");
    assert_eq!(sanitize("\t a b \n"), "a_b");
    assert_eq!(sanitize(r#"x\y|z<>"#), "x_y_z__");
}

#[test]
fn a1_rust_additions() {
    // Control characters (not Java `\s`) become `_`.
    assert_eq!(sanitize("a\u{1}b"), "a_b");
    assert_eq!(sanitize("a\u{7F}b"), "a_b");

    // The sanitized part is truncated to 64 bytes on a char boundary; the hash is kept.
    let long = "x".repeat(300);
    let stem = file_stem(&long);
    let (sanitized, hash) = stem.rsplit_once("__").unwrap();
    assert_eq!(sanitized.len(), 64);
    assert_eq!(hash, name_hash(&long));

    let wide = "é".repeat(100);
    let sanitized = sanitize(&wide);
    assert!(sanitized.len() <= 64);
    assert_eq!(sanitized, "é".repeat(32));
}

#[test]
fn a1_java_file_names() {
    assert_eq!(
        java_legacy_repository_file_name(Some("My  Repo")),
        "MyRepo.a3s.repository"
    );
    // Only U+0020 is removed.
    assert_eq!(
        java_legacy_repository_file_name(Some("a\tb c")),
        "a\tbc.a3s.repository"
    );
    assert_eq!(
        java_legacy_repository_file_name(None),
        "repository.a3s.repository"
    );
    assert_eq!(
        java_preferred_repository_file_name(None),
        "repository__000000.a3s.repository"
    );
    assert_eq!(
        java_preferred_repository_file_name(Some("Sample HTTP")),
        "Sample_HTTP__5ab39964a7ab.a3s.repository"
    );
    // No Rust additions in Java's name.
    assert_eq!(
        java_preferred_repository_file_name(Some("a\u{1}b")),
        format!("a\u{1}b__{}.a3s.repository", name_hash("a\u{1}b"))
    );
    assert_eq!(
        java_profile_file_name("Event Night"),
        "Event Night.a3s.profile"
    );
}

#[test]
fn a1_java_names_match_the_fixture_files() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/java_local/peruser/data/Arma3Sync/repositories");
    for name in ["Sample HTTP", "Sample FTP", "Sample SFTP"] {
        assert!(
            dir.join(java_preferred_repository_file_name(Some(name)))
                .is_file(),
            "{name}"
        );
    }
    assert!(
        dir.join(java_legacy_repository_file_name(Some("Sample FTP")))
            .is_file()
    );
}
