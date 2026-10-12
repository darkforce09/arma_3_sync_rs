//! Insecure-TLS policy (`SslValidationPolicy`).

use a3s_types::ssl_policy::{host_part, is_allowed_for, is_local_test_host};

/// Ported from `SslValidationPolicyTest`.
#[test]
fn allows_only_local_hosts_without_explicit_override() {
    assert!(is_allowed_for("localhost/repository", false));
    assert!(is_allowed_for("127.0.0.1", false));
    assert!(!is_allowed_for("updates.example/repository", false));
    assert!(is_allowed_for("updates.example/repository", true));
}

#[test]
fn local_test_hosts() {
    for local in [
        "LOCALHOST",
        "localhost.",
        " localhost ",
        "[::1]",
        "::1",
        "0:0:0:0:0:0:0:1",
        "[0:0:0:0:0:0:0:1]",
        "127.8.9.10/x",
        "localhost/repository",
    ] {
        assert!(is_local_test_host(local), "{local}");
    }
    // A port is part of the host; brackets are stripped only in pairs.
    for remote in [
        "localhost:8443",
        "[::1]:443",
        "[::1",
        "::1]",
        "128.0.0.1",
        "localhost.example",
        "",
        "/localhost",
    ] {
        assert!(!is_local_test_host(remote), "{remote}");
    }
}

#[test]
fn host_part_is_trimmed_and_cut_at_the_first_slash() {
    assert_eq!(host_part(" h/x/y "), "h");
    assert_eq!(host_part("h:8443/x"), "h:8443");
    assert_eq!(host_part("h"), "h");
    assert_eq!(host_part("/x"), "");
}
