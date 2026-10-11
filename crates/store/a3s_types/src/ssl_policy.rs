//! When TLS certificate validation may be turned off.
//!
//! A protocol with `validate_ssl_certificate == false` (HTTPS, HTTPS-WebDAV, FTPS) is allowed
//! only for a local test host, or anywhere when the user sets [`ALLOW_INSECURE_ENV`] (Java uses
//! the system property `a3s.allowInsecureSsl` instead). Transports enforce this at connect time;
//! these functions give the UI and the command line the same predicate and texts.

use crate::java_string::java_trim;

/// Environment variable that allows disabled certificate validation for any host: `1` or
/// `true` (any case).
pub const ALLOW_INSECURE_ENV: &str = "A3S_ALLOW_INSECURE_SSL";

/// The host of a `host[:port][/path]` string: trimmed like Java, cut at the first `/`. The port
/// is kept.
// Mirrors service/SslValidationPolicy.java:28-33
pub fn host_part(url_or_host: &str) -> &str {
    let value = java_trim(url_or_host);
    value.split_once('/').map_or(value, |(host, _)| host)
}

/// Whether `host` (see [`host_part`]) is `localhost`, `localhost.`, the IPv6 loopback (with or
/// without brackets) or in `127.0.0.0/8`. A host with a port (`localhost:8443`) is not.
// Mirrors service/SslValidationPolicy.java:13-22
pub fn is_local_test_host(host: &str) -> bool {
    let normalized = host_part(host).to_ascii_lowercase();
    let normalized = normalized
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(&normalized);
    matches!(
        normalized,
        "localhost" | "localhost." | "::1" | "0:0:0:0:0:0:0:1"
    ) || normalized.starts_with("127.")
}

/// Whether certificate validation may be disabled for `host`. `allow_override` is the user's
/// explicit opt-in, normally [`insecure_override_from_env`].
// Mirrors service/SslValidationPolicy.java:24-26
pub fn is_allowed_for(host: &str, allow_override: bool) -> bool {
    is_local_test_host(host) || allow_override
}

/// Whether [`ALLOW_INSECURE_ENV`] is set to `1` or `true` (any case).
pub fn insecure_override_from_env() -> bool {
    std::env::var_os(ALLOW_INSECURE_ENV).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// Why a connection with disabled certificate validation to a non-local host was refused.
// Mirrors service/SslValidationPolicy.java:35-41
pub fn refusal_text() -> String {
    format!(
        "SSL certificate validation is disabled for a non-local repository. Enable certificate \
         validation or explicitly set the environment variable {ALLOW_INSECURE_ENV}=1 for a \
         controlled development test."
    )
}

/// Warning shown when the user disables certificate validation for `host`.
// Mirrors service/SslValidationPolicy.java:43-48
pub fn warning_text(host: &str) -> String {
    format!(
        "SSL certificate validation is disabled for '{}'.\n\n\
         This accepts forged certificates and hostnames. This setting is intended only for local \
         tests.\n\
         For a remote development server, set the environment variable {ALLOW_INSECURE_ENV}=1 \
         before starting Arma3Sync.\n\n\
         Enable certificate validation whenever possible.",
        host_part(host)
    )
}
