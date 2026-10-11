//! The configured proxy (`configration.Proxy`) applied per connection.
//!
//! Java sets JVM-wide system properties and a default `Authenticator`; which connections use
//! them depends on the Java networking stack of each protocol. [`proxy_for`] reproduces that
//! effective routing for one target:
//!
//! * an HTTP proxy serves `http://` targets (HTTP, HTTP WebDAV) only, an HTTPS proxy `https://`
//!   targets only (Java's `HttpURLConnection` and Apache `HttpClient` with system properties);
//! * an FTP proxy serves nothing (only `java.net.URL` FTP connections read it; commons-net does
//!   not);
//! * a SOCKS proxy serves every target except SFTP (every `java.net.Socket`, but not MINA's NIO
//!   channels).
//!
//! The credentials go to the proxy only; Java's default `Authenticator` also answered the
//! repository server's challenges with them. A SOCKS4 proxy speaks SOCKS4 here, where Java's
//! default `socksProxyVersion` made it speak SOCKS5.

use a3s_types::local::Proxy;
use a3s_types::local::protocol::parse_java_int;
use a3s_types::{Protocol, ProtocolType};

use crate::{ProxyConfig, TransportOptions};

/// The stored proxy cannot be used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProxyError {
    #[error("Invalid proxy host: {0}")]
    InvalidHost(String),

    #[error("Invalid proxy port: {0}")]
    InvalidPort(String),
}

impl From<ProxyError> for crate::Error {
    fn from(err: ProxyError) -> Self {
        Self::InvalidProtocol(err.to_string())
    }
}

/// The proxy a connection to a `target` repository goes through, or `None` (disabled, no
/// protocol, or a proxy type that does not serve this target).
// Mirrors service/ConfigurationService.java:356-409
pub fn proxy_for(target: ProtocolType, proxy: &Proxy) -> Result<Option<ProxyConfig>, ProxyError> {
    let Some(protocol) = proxy.proxy_protocol.as_ref().filter(|_| proxy.enable_proxy) else {
        return Ok(None);
    };
    let http_targets = [ProtocolType::Http, ProtocolType::HttpWebdav];
    let https_targets = [ProtocolType::Https, ProtocolType::HttpsWebdav];
    let socket_targets = [
        ProtocolType::Ftp,
        ProtocolType::Ftps,
        ProtocolType::Http,
        ProtocolType::Https,
        ProtocolType::HttpWebdav,
        ProtocolType::HttpsWebdav,
    ];
    let scheme = match protocol.protocol_type {
        ProtocolType::Http if http_targets.contains(&target) => "http",
        // Java speaks plain HTTP `CONNECT` to an `https.proxyHost`: no TLS to the proxy.
        ProtocolType::Https if https_targets.contains(&target) => "http",
        ProtocolType::Socks4 if socket_targets.contains(&target) => "socks4",
        // Java resolves the target host itself before connecting: local DNS.
        ProtocolType::Socks5 if socket_targets.contains(&target) => "socks5",
        _ => return Ok(None),
    };
    proxy_config(scheme, protocol).map(Some)
}

/// The proxy's host, port and credentials.
// Mirrors service/ConfigurationService.java:375-401
fn proxy_config(scheme: &str, protocol: &Protocol) -> Result<ProxyConfig, ProxyError> {
    let host = &protocol.url;
    if host.is_empty() || host.contains(['/', '@']) || host.contains(char::is_whitespace) {
        return Err(ProxyError::InvalidHost(host.clone()));
    }
    let port = parse_java_int(&protocol.port)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or_else(|| ProxyError::InvalidPort(protocol.port.clone()))?;
    let (login, password) = match protocol.login.as_deref() {
        None | Some("anonymous") => (None, None),
        Some(login) => (
            Some(login.to_owned()),
            Some(protocol.password.clone().unwrap_or_default()),
        ),
    };
    Ok(ProxyConfig {
        scheme: scheme.to_owned(),
        host: host.clone(),
        port,
        login,
        password,
    })
}

/// [`TransportOptions::from_protocol`] plus the proxy that serves `target`. Every connection to
/// a repository builds its options with this.
pub fn transport_options(target: &Protocol, proxy: &Proxy) -> Result<TransportOptions, ProxyError> {
    let mut options = TransportOptions::from_protocol(target);
    options.proxy = proxy_for(target.protocol_type, proxy)?;
    Ok(options)
}
