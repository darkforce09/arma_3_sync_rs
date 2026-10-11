//! The configured proxy: routing per target, host/port/credential mapping, a download through a
//! local HTTP proxy, and FTP(S) and SFTP through a local SOCKS proxy.

use std::sync::Arc;

use a3s_transport::proxy::{ProxyError, proxy_for, transport_options};
use a3s_transport::{TransferCtx, Transport};
use a3s_types::local::Proxy;
use a3s_types::{Protocol, ProtocolType};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use parking_lot::Mutex;

use crate::support::AbortOnDrop;
use crate::support::socks::{SocksProxy, Tunnel};
use crate::support::{Fixture, random_bytes};

fn proxy(protocol_type: ProtocolType, url: &str, port: &str, login: &str) -> Proxy {
    Proxy {
        enable_proxy: true,
        proxy_protocol: Some(Protocol::new(
            protocol_type,
            url,
            port,
            login,
            "secret",
            false,
        )),
    }
}

/// Java's routing, except that SOCKS also serves SFTP.
#[test]
fn routing_follows_the_java_system_properties() {
    use ProtocolType::{
        A3s, Ftp, Ftps, Http, HttpWebdav, Https, HttpsWebdav, Sftp, Socks4, Socks5,
    };
    // Proxy type → (targets it serves, scheme spoken to the proxy).
    let table: [(ProtocolType, &[ProtocolType], &str); 10] = [
        (Http, &[Http, HttpWebdav], "http"),
        (Https, &[Https, HttpsWebdav], "http"),
        (Ftp, &[], ""),
        (
            Socks4,
            &[Ftp, Ftps, Sftp, Http, Https, HttpWebdav, HttpsWebdav],
            "socks4",
        ),
        (
            Socks5,
            &[Ftp, Ftps, Sftp, Http, Https, HttpWebdav, HttpsWebdav],
            "socks5",
        ),
        (Ftps, &[], ""),
        (Sftp, &[], ""),
        (A3s, &[], ""),
        (HttpWebdav, &[], ""),
        (HttpsWebdav, &[], ""),
    ];
    for (proxy_type, served, scheme) in table {
        let configured = proxy(proxy_type, "10.0.0.1", "3128", "anonymous");
        for target in ProtocolType::ALL {
            let config = proxy_for(target, &configured).unwrap();
            if served.contains(&target) {
                let config = config.unwrap_or_else(|| panic!("{proxy_type:?} for {target:?}"));
                assert_eq!(config.scheme, scheme);
                assert_eq!((config.host.as_str(), config.port), ("10.0.0.1", 3128));
            } else {
                assert_eq!(config, None, "{proxy_type:?} for {target:?}");
            }
        }
    }
}

#[test]
fn disabled_or_missing_proxy_is_none() {
    let mut disabled = proxy(ProtocolType::Http, "h", "1", "anonymous");
    disabled.enable_proxy = false;
    assert_eq!(proxy_for(ProtocolType::Http, &disabled), Ok(None));
    let missing = Proxy {
        enable_proxy: true,
        proxy_protocol: None,
    };
    assert_eq!(proxy_for(ProtocolType::Http, &missing), Ok(None));
}

#[test]
fn credentials_go_to_the_proxy_unless_anonymous() {
    let anonymous = proxy_for(
        ProtocolType::Http,
        &proxy(ProtocolType::Http, "h", "1", "anonymous"),
    )
    .unwrap()
    .unwrap();
    assert_eq!((anonymous.login, anonymous.password), (None, None));

    let mut no_password = proxy(ProtocolType::Http, "h", "1", "u");
    no_password.proxy_protocol.as_mut().unwrap().password = None;
    let config = proxy_for(ProtocolType::Http, &no_password)
        .unwrap()
        .unwrap();
    assert_eq!(config.login.as_deref(), Some("u"));
    assert_eq!(config.password.as_deref(), Some(""));

    // Only the exact login "anonymous" means no authentication, as in Java.
    let upper = proxy_for(
        ProtocolType::Http,
        &proxy(ProtocolType::Http, "h", "1", "Anonymous"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(upper.login.as_deref(), Some("Anonymous"));
}

#[test]
fn ports_and_hosts_are_validated() {
    let port = |p: &str| proxy_for(ProtocolType::Http, &proxy(ProtocolType::Http, "h", p, "a"));
    assert_eq!(port("+3128").unwrap().unwrap().port, 3128);
    for bad in ["0", "-1", "70000", "abc", "", " 80"] {
        assert_eq!(port(bad), Err(ProxyError::InvalidPort(bad.to_owned())));
    }
    let host = |h: &str| proxy_for(ProtocolType::Http, &proxy(ProtocolType::Http, h, "1", "a"));
    for bad in ["", "h/x", "u@h", "h x"] {
        assert_eq!(host(bad), Err(ProxyError::InvalidHost(bad.to_owned())));
    }
    assert_eq!(host("::1").unwrap().unwrap().url(), "http://[::1]:1");
}

/// What the proxy saw: request target, `Host` and `Proxy-Authorization`.
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

#[tokio::test]
async fn http_download_goes_through_the_proxy() {
    let seen: Seen = Arc::default();
    let recorder = seen.clone();
    let router = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let recorder = recorder.clone();
        async move {
            let header = |name: &str| {
                req.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned()
            };
            recorder.lock().push((
                req.uri().to_string(),
                header("host"),
                header("proxy-authorization"),
            ));
            (StatusCode::OK, "via proxy").into_response()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = AbortOnDrop(tokio::spawn(async move {
        let _ignored = axum::serve(listener, router).await;
    }));

    let target = Protocol::new(
        ProtocolType::Http,
        "repo.example.invalid/r",
        "80",
        "anonymous",
        "",
        true,
    );
    let mut configured = proxy(ProtocolType::Http, "127.0.0.1", &port.to_string(), "u");
    configured.proxy_protocol.as_mut().unwrap().password = Some("p".to_owned());
    let options = transport_options(&target, &configured).unwrap();
    let mut transport = Transport::connect(&target, &options).await.unwrap();
    let bytes = transport
        .fetch(".a3s/serverinfo", &TransferCtx::new())
        .await
        .unwrap();
    assert_eq!(bytes, b"via proxy");
    let seen = seen.lock().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let (uri, host, authorization) = &seen[0];
    assert!(uri.ends_with("/r/.a3s/serverinfo"), "{uri}");
    assert_eq!(host, "repo.example.invalid");
    assert_eq!(authorization, "Basic dTpw");

    // An HTTPS-type proxy does not serve an `http://` repository.
    configured.proxy_protocol.as_mut().unwrap().protocol_type = ProtocolType::Https;
    assert_eq!(transport_options(&target, &configured).unwrap().proxy, None);
}

/// Connects to `fixture` through `socks`, configured as a `proxy_type` proxy with `login`.
async fn connect_through(
    fixture: &Fixture,
    socks: &SocksProxy,
    proxy_type: ProtocolType,
    login: &str,
) -> Transport {
    let configured = proxy(proxy_type, "127.0.0.1", &socks.port.to_string(), login);
    let options = transport_options(&fixture.protocol, &configured).unwrap();
    assert!(options.proxy.is_some());
    Transport::connect(&fixture.protocol, &options)
        .await
        .unwrap()
}

/// Download, resumed download (a data connection opened by hand) and upload over FTP and FTPS
/// through SOCKS4 and SOCKS5: the control and every data connection are tunnels.
#[tokio::test]
async fn ftp_goes_through_a_socks_proxy() {
    let cases = [
        (ProtocolType::Socks5, Some(("u", "secret")), false),
        (ProtocolType::Socks5, None, true),
        (ProtocolType::Socks4, None, false),
        (ProtocolType::Socks4, Some(("u", "")), true),
    ];
    for (proxy_type, credentials, ftps) in cases {
        let case = format!("{proxy_type:?} {credentials:?} ftps={ftps}");
        // SOCKS4 has no password: the proxy only sees the user id.
        let required = credentials.filter(|_| proxy_type == ProtocolType::Socks5);
        let socks = SocksProxy::start(required).await;
        let fx = crate::support::ftp::fixture(ftps).await;
        let data = random_bytes(200_000, 5);
        fx.put("d/a.bin", &data);
        let login = credentials.map_or("anonymous", |(user, _)| user);
        let mut t = connect_through(&fx, &socks, proxy_type, login).await;
        let ctx = TransferCtx::new();
        assert!(t.fetch("d/a.bin", &ctx).await.unwrap() == data, "{case}");
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("a.bin");
        std::fs::write(&dest, &data[..1000]).unwrap();
        let len = t.download_file("d/a.bin", &dest, 1000, &ctx).await.unwrap();
        assert_eq!(len, data.len() as u64, "{case}");
        assert!(std::fs::read(&dest).unwrap() == data, "{case}");
        assert_eq!(ctx.start_offset.get(), Some(1000), "{case}");
        t.upload_bytes(b"up", "e/b.bin", &ctx).await.unwrap();
        assert_eq!(
            std::fs::read(fx.base.join("e/b.bin")).unwrap(),
            b"up",
            "{case}"
        );
        t.close().await;

        let tunnels = socks.tunnels();
        let version = if proxy_type == ProtocolType::Socks5 {
            5
        } else {
            4
        };
        let user = credentials.map(|(user, _)| user.to_owned());
        assert!(
            tunnels
                .iter()
                .all(|tunnel| tunnel.version == version && tunnel.user == user),
            "{case}: {tunnels:?}"
        );
        let control = format!("127.0.0.1:{}", fx.protocol.port);
        let (controls, data_connections): (Vec<&Tunnel>, Vec<&Tunnel>) =
            tunnels.iter().partition(|tunnel| tunnel.target == control);
        assert_eq!(controls.len(), 1, "{case}: {tunnels:?}");
        // fetch, resumed download, upload (and the listings of `mkdirs`, if any).
        assert!(data_connections.len() >= 3, "{case}: {tunnels:?}");
    }
}

/// SFTP through SOCKS5 with credentials.
#[tokio::test]
async fn sftp_goes_through_a_socks_proxy() {
    let socks = SocksProxy::start(Some(("u", "secret"))).await;
    let fx = crate::support::sftp::fixture().await;
    fx.put("a.bin", b"over socks");
    let mut t = connect_through(&fx, &socks, ProtocolType::Socks5, "u").await;
    assert_eq!(
        t.fetch("a.bin", &TransferCtx::new()).await.unwrap(),
        b"over socks"
    );
    t.close().await;
    assert_eq!(
        socks.tunnels(),
        vec![Tunnel {
            version: 5,
            user: Some("u".to_owned()),
            target: format!("127.0.0.1:{}", fx.protocol.port),
        }]
    );
}

/// Wrong SOCKS5 credentials fail the connection; an HTTP proxy cannot carry FTP.
#[tokio::test]
async fn socks_refusals() {
    let socks = SocksProxy::start(Some(("u", "other"))).await;
    let fx = crate::support::ftp::fixture(false).await;
    let configured = proxy(
        ProtocolType::Socks5,
        "127.0.0.1",
        &socks.port.to_string(),
        "u",
    );
    let options = transport_options(&fx.protocol, &configured).unwrap();
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(result.is_err());
    assert!(socks.tunnels().is_empty());

    let mut options = a3s_transport::TransportOptions::from_protocol(&fx.protocol);
    options.proxy = Some(a3s_transport::ProxyConfig {
        scheme: "http".to_owned(),
        host: "127.0.0.1".to_owned(),
        port: socks.port,
        login: None,
        password: None,
    });
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(
        matches!(result, Err(a3s_transport::Error::InvalidProtocol(_))),
        "{:?}",
        result.err()
    );
}
