//! The configured proxy (`configration.Proxy`) applied per connection.
//!
//! Java sets JVM-wide system properties and a default `Authenticator`; which connections use
//! them depends on the Java networking stack of each protocol. [`proxy_for`] reproduces that
//! effective routing for one target, except for SFTP:
//!
//! * an HTTP proxy serves `http://` targets (HTTP, HTTP WebDAV) only, an HTTPS proxy `https://`
//!   targets only (Java's `HttpURLConnection` and Apache `HttpClient` with system properties);
//! * an FTP proxy serves nothing (only `java.net.URL` FTP connections read it; commons-net does
//!   not);
//! * a SOCKS proxy serves every target: FTP(S), HTTP(S) and WebDAV like every `java.net.Socket`
//!   in Java, and SFTP too, which Java's MINA NIO channels never routed through it.
//!
//! The credentials go to the proxy only; Java's default `Authenticator` also answered the
//! repository server's challenges with them. A SOCKS4 proxy speaks SOCKS4 here, where Java's
//! default `socksProxyVersion` made it speak SOCKS5.
//!
//! HTTP and WebDAV hand the proxy to `reqwest`; FTP(S) and SFTP open their sockets through a
//! dialer, which tunnels the control and data connections through a SOCKS proxy.

use std::net::SocketAddr;

use a3s_types::local::Proxy;
use a3s_types::local::protocol::parse_java_int;
use a3s_types::{Protocol, ProtocolType};
use tokio::net::TcpStream;
use tokio_socks::tcp::{Socks4Stream, Socks5Stream};

use crate::{Error, ProxyConfig, Result, TransportOptions};

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
        ProtocolType::Sftp,
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

/// SOCKS protocol version of a [`Dialer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SocksVersion {
    V4,
    V5,
}

/// Opens the TCP connections of the FTP(S) and SFTP transports: directly, or through the
/// SOCKS proxy of the [`TransportOptions`]. Targets are resolved locally, as Java does before
/// connecting a socket.
#[derive(Clone, Debug, Default)]
pub(crate) struct Dialer {
    socks: Option<(SocksVersion, ProxyConfig)>,
}

impl Dialer {
    /// The dialer for `options`; `protocol` names the transport in the error for a proxy that
    /// cannot carry raw TCP connections (an HTTP proxy).
    pub(crate) fn new(options: &TransportOptions, protocol: &'static str) -> Result<Self> {
        let Some(proxy) = &options.proxy else {
            return Ok(Self::default());
        };
        let version = match proxy.scheme.as_str() {
            "socks4" => SocksVersion::V4,
            "socks5" => SocksVersion::V5,
            other => {
                return Err(Error::InvalidProtocol(format!(
                    "a {other} proxy cannot carry {protocol} connections"
                )));
            }
        };
        Ok(Self {
            socks: Some((version, proxy.clone())),
        })
    }

    /// Connects to the first reachable address of `host`, through the proxy if there is one.
    pub(crate) async fn connect_host(&self, host: &str, port: u16) -> std::io::Result<TcpStream> {
        let mut last_error = None;
        for target in tokio::net::lookup_host((host, port)).await? {
            match self.connect(target).await {
                Ok(stream) => return Ok(stream),
                Err(err) => last_error = Some(err),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("unknown host '{host}'"),
            )
        }))
    }

    /// Connects to `target`, through the proxy if there is one.
    pub(crate) async fn connect(&self, target: SocketAddr) -> std::io::Result<TcpStream> {
        let Some((version, proxy)) = &self.socks else {
            return TcpStream::connect(target).await;
        };
        let socket = TcpStream::connect((proxy.host.as_str(), proxy.port)).await?;
        let login = proxy.login.as_deref();
        let tunnel = match (version, login) {
            (SocksVersion::V4, None) => Socks4Stream::connect_with_socket(socket, target)
                .await
                .map(Socks4Stream::into_inner),
            // SOCKS4 has no password: the login is the user id.
            (SocksVersion::V4, Some(login)) => {
                Socks4Stream::connect_with_userid_and_socket(socket, target, login)
                    .await
                    .map(Socks4Stream::into_inner)
            }
            (SocksVersion::V5, None) => Socks5Stream::connect_with_socket(socket, target)
                .await
                .map(Socks5Stream::into_inner),
            (SocksVersion::V5, Some(login)) => Socks5Stream::connect_with_password_and_socket(
                socket,
                target,
                login,
                proxy.password.as_deref().unwrap_or_default(),
            )
            .await
            .map(Socks5Stream::into_inner),
        };
        tunnel.map_err(|err| match err {
            tokio_socks::Error::Io(err) => err,
            other => std::io::Error::other(format!(
                "SOCKS proxy {}:{}: {other}",
                proxy.host, proxy.port
            )),
        })
    }
}
