use std::time::Duration;

use a3s_types::Protocol;

/// A proxy for outgoing connections (`domain.configration.Proxy`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyConfig {
    /// `http`, `https`, `socks4` or `socks5`.
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub login: Option<String>,
    pub password: Option<String>,
}

impl ProxyConfig {
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }
}

/// Connection options derived from a [`Protocol`] and global settings.
#[derive(Clone, Debug)]
pub struct TransportOptions {
    pub connect_timeout: Option<Duration>,
    pub read_timeout: Option<Duration>,
    pub proxy: Option<ProxyConfig>,

    /// Allows disabled certificate validation for non-local hosts
    /// (Java: `-Da3s.allowInsecureSsl=true`).
    pub allow_insecure_tls: bool,
}

impl TransportOptions {
    /// Options from the protocol's timeouts; `0` means no timeout, as in Java.
    pub fn from_protocol(protocol: &Protocol) -> Self {
        let to_duration = |ms: u32| (ms > 0).then(|| Duration::from_millis(u64::from(ms)));
        Self {
            connect_timeout: to_duration(protocol.connection_timeout_ms()),
            read_timeout: to_duration(protocol.read_timeout_ms()),
            proxy: None,
            allow_insecure_tls: std::env::var_os("A3S_ALLOW_INSECURE_SSL")
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
        }
    }
}

/// `SslValidationPolicy.isLocalTestHost`.
pub fn is_local_test_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let host = host.trim_start_matches('[').trim_end_matches(']');
    matches!(host, "localhost" | "localhost." | "::1" | "0:0:0:0:0:0:0:1")
        || host.starts_with("127.")
}
