use std::time::Duration;

use a3s_types::{Protocol, ssl_policy};

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
    /// `scheme://host:port`, an IPv6 host in brackets.
    pub fn url(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("{}://[{}]:{}", self.scheme, self.host, self.port)
        } else {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        }
    }
}

/// Connection options derived from a [`Protocol`] and global settings; with the configured proxy
/// from [`crate::proxy::transport_options`].
#[derive(Clone, Debug)]
pub struct TransportOptions {
    pub connect_timeout: Option<Duration>,
    pub read_timeout: Option<Duration>,
    pub proxy: Option<ProxyConfig>,

    /// Allows disabled certificate validation for non-local hosts
    /// ([`ssl_policy::is_allowed_for`]; Java: `-Da3s.allowInsecureSsl=true`).
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
            allow_insecure_tls: ssl_policy::insecure_override_from_env(),
        }
    }
}
