//! The configured proxy: routing per target, host/port/credential mapping, and a download
//! through a local HTTP proxy.

use std::sync::Arc;

use a3s_transport::proxy::{ProxyError, proxy_for, transport_options};
use a3s_transport::{TransferCtx, Transport};
use a3s_types::local::Proxy;
use a3s_types::{Protocol, ProtocolType};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use parking_lot::Mutex;

use crate::support::ftp::AbortOnDrop;

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
            &[Ftp, Ftps, Http, Https, HttpWebdav, HttpsWebdav],
            "socks4",
        ),
        (
            Socks5,
            &[Ftp, Ftps, Http, Https, HttpWebdav, HttpsWebdav],
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
