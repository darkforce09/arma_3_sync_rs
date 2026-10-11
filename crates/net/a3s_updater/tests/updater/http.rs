//! Request headers, status handling, scheme rules, redirects and the metadata size cap.

use a3s_updater::{CancellationToken, ConfigOverrides, HttpClient, UpdateConfig, UpdateError};

use crate::support::{Reply, Server, local_config};

fn client() -> HttpClient {
    HttpClient::new(&local_config("")).expect("client")
}

#[tokio::test]
async fn headers_and_status() {
    let server = Server::start().await;
    server.route("/text", Reply::ok("héllo"));
    server.route("/fail", Reply::status(500));
    let http = client();
    let cancel = CancellationToken::new();

    let text = http
        .read_text(
            &server.url("/text"),
            &[("X-GitHub-Api-Version", "2022-11-28")],
            &cancel,
        )
        .await
        .expect("text");
    assert_eq!(text, "héllo");
    let request = &server.requests()[0];
    assert_eq!(request.headers["user-agent"], "ArmA3Sync-Updater");
    assert_eq!(request.headers["x-github-api-version"], "2022-11-28");

    let err = http
        .read_text(&server.url("/fail"), &[], &cancel)
        .await
        .expect_err("500");
    assert!(matches!(err, UpdateError::Http { status: 500 }), "{err:?}");
}

#[tokio::test]
async fn plain_http_needs_allow_http() {
    let server = Server::start().await;
    server.route("/text", Reply::ok("x"));
    let strict = UpdateConfig::from_toml_str("", &ConfigOverrides::default()).expect("config");
    let http = HttpClient::new(&strict).expect("client");
    let err = http
        .read_text(&server.url("/text"), &[], &CancellationToken::new())
        .await
        .expect_err("http refused");
    assert!(matches!(err, UpdateError::InsecureScheme(_)), "{err:?}");
    assert_eq!(server.hits("/text"), 0);
}

#[tokio::test]
async fn metadata_size_cap() {
    let server = Server::start().await;
    let limit = 1_048_576;
    server.route("/exact", Reply::ok(vec![b'a'; limit]));
    server.route("/over", Reply::ok(vec![b'a'; limit + 1]));
    server.route(
        "/over-chunked",
        Reply::Chunked(vec![vec![b'a'; limit / 2], vec![b'a'; limit / 2 + 1]]),
    );
    server.route(
        "/exact-chunked",
        Reply::Chunked(vec![vec![b'a'; limit / 2], vec![b'a'; limit / 2]]),
    );
    let http = client();
    let cancel = CancellationToken::new();
    for path in ["/exact", "/exact-chunked"] {
        let text = http
            .read_text(&server.url(path), &[], &cancel)
            .await
            .expect(path);
        assert_eq!(text.len(), limit);
    }
    for path in ["/over", "/over-chunked"] {
        let err = http
            .read_text(&server.url(path), &[], &cancel)
            .await
            .expect_err(path);
        assert!(matches!(err, UpdateError::TooLarge), "{path}: {err:?}");
    }
}

#[tokio::test]
async fn redirects() {
    let server = Server::start().await;
    server.route("/target", Reply::ok("moved"));
    server.route("/hop", Reply::Redirect(server.url("/target").to_string()));
    server.route("/ftp", Reply::Redirect("ftp://127.0.0.1/file".to_owned()));
    server.route("/loop", Reply::Redirect(server.url("/loop").to_string()));
    let http = client();
    let cancel = CancellationToken::new();

    let text = http
        .read_text(&server.url("/hop"), &[], &cancel)
        .await
        .expect("http to http when allowed");
    assert_eq!(text, "moved");

    let err = http
        .read_text(&server.url("/ftp"), &[], &cancel)
        .await
        .expect_err("ftp redirect");
    assert!(matches!(err, UpdateError::InsecureScheme(_)), "{err:?}");

    let err = http
        .read_text(&server.url("/loop"), &[], &cancel)
        .await
        .expect_err("redirect loop");
    assert!(matches!(err, UpdateError::Http { status: 302 }), "{err:?}");
    assert_eq!(server.hits("/loop"), 11);
}

#[tokio::test]
async fn head_content_length() {
    let server = Server::start().await;
    server.route("/archive.zip", Reply::ok(vec![0u8; 4321]));
    server.route("/chunked.zip", Reply::Chunked(vec![vec![0u8; 10]]));
    let http = client();
    let cancel = CancellationToken::new();
    let length = http
        .content_length(&server.url("/archive.zip"), &cancel)
        .await
        .expect("head");
    assert_eq!(length, Some(4321));
    assert_eq!(server.requests()[0].method, "HEAD");
    let length = http
        .content_length(&server.url("/chunked.zip"), &cancel)
        .await
        .expect("head chunked");
    assert_eq!(length, None);
    let err = http
        .content_length(&server.url("/missing.zip"), &cancel)
        .await
        .expect_err("404");
    assert!(matches!(err, UpdateError::Http { status: 404 }), "{err:?}");
}

#[tokio::test]
async fn https_to_http_redirect_is_refused() {
    let (secure, certificate) = Server::start_tls().await;
    let plain = Server::start().await;
    plain.route("/target", Reply::ok("downgraded"));
    secure.route("/ok", Reply::ok("secure"));
    secure.route("/hop", Reply::Redirect(plain.url("/target").to_string()));
    // `allow_http` is on: only the downgrade itself is refused.
    let http = HttpClient::with_root_certificates(&local_config(""), certificate.as_bytes())
        .expect("client");
    let cancel = CancellationToken::new();

    let text = http
        .read_text(&secure.url("/ok"), &[], &cancel)
        .await
        .expect("https");
    assert_eq!(text, "secure");

    let err = http
        .read_text(&secure.url("/hop"), &[], &cancel)
        .await
        .expect_err("downgrade");
    assert!(matches!(err, UpdateError::InsecureScheme(_)), "{err:?}");
    assert_eq!(plain.hits("/target"), 0);
}
