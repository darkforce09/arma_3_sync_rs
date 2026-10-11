//! Autoconfig URL parsing and formatting (`AutoConfigURLAccessMethods`).

use a3s_types::autoconfig_url::{
    AutoConfigUrlError, autoconfig_display_url, determine_autoconfig_url, parse, parse_import_url,
};
use a3s_types::{Protocol, ProtocolType};

fn url_and_port(url: &str) -> (String, String) {
    let protocol = parse(url).unwrap();
    (protocol.url, protocol.port)
}

#[test]
fn parses_repository_urls() {
    let wog = parse("ftp://88.99.69.243/Repo/WOG/.a3s/autoconfig").unwrap();
    assert_eq!(
        wog,
        Protocol::new(
            ProtocolType::Ftp,
            "88.99.69.243/Repo/WOG/.a3s",
            "21",
            "anonymous",
            "",
            true
        )
    );
    assert_eq!(wog.hostname(), "88.99.69.243");
    assert_eq!(wog.remote_path(), "/Repo/WOG/.a3s");
    assert!(wog.is_anonymous());

    let https = parse("https://repo.ofcra.org/.a3s/autoconfig").unwrap();
    assert_eq!(https.protocol_type, ProtocolType::Https);
    assert_eq!(https.url, "repo.ofcra.org/.a3s");
    assert_eq!(https.port, "443");
    assert!(https.validate_ssl_certificate);
}

/// Ported from `AutoConfigURLAccessMethodsTest`.
#[test]
fn parses_trimmed_https_and_rejects_schemes_inside_the_path() {
    let protocol = parse("  https://a3s.vpzbrig21.de/master/.a3s/autoconfig  ").unwrap();
    assert_eq!(protocol.protocol_type, ProtocolType::Https);
    assert_eq!(protocol.url, "a3s.vpzbrig21.de/master/.a3s");
    assert_eq!(protocol.port, "443");
    assert!(protocol.validate_ssl_certificate);

    assert_eq!(
        parse("repo.example/http://autoconfig"),
        Err(AutoConfigUrlError::UnsupportedScheme)
    );
}

#[test]
fn scheme_is_a_case_insensitive_prefix() {
    let upper = parse("HTTPS://H/a/autoconfig").unwrap();
    assert_eq!(upper.protocol_type, ProtocolType::Https);
    assert_eq!(upper.url, "H/a");

    let http = parse("Http://h/.a3s/autoconfig").unwrap();
    assert_eq!(http.protocol_type, ProtocolType::Http);
    assert_eq!(http.port, "80");

    // Java trims only chars <= U+0020; a no-break space stays and hides the scheme.
    assert!(parse("\t\n ftp://h/autoconfig\r").is_ok());
    assert!(parse("\u{a0}ftp://h/autoconfig").is_err());

    for rejected in [
        "ftps://h/.a3s/autoconfig",
        "sftp://h/.a3s/autoconfig",
        "a3s://h/.a3s/autoconfig",
        "http:/h",
        "",
        "   ",
    ] {
        assert_eq!(
            parse(rejected),
            Err(AutoConfigUrlError::UnsupportedScheme),
            "{rejected}"
        );
    }
}

#[test]
fn splits_host_port_and_path_like_java() {
    let cases = [
        ("https://h:8443/r/.a3s/autoconfig", "h/r/.a3s", "8443"),
        ("http://h:abc/.a3s/autoconfig", "h/.a3s", "80"),
        ("ftp://h:/x/autoconfig", "h/x", "21"),
        // `Integer.parseInt` accepts a sign and leading zeros; the port is kept as typed.
        ("ftp://h:+21/x/autoconfig", "h/x", "+21"),
        ("ftp://h:-5/x/autoconfig", "h/x", "-5"),
        ("ftp://h:0021/x/autoconfig", "h/x", "0021"),
        ("ftp://h:99999999999/x/autoconfig", "h/x", "21"),
        // User info and IPv6 literals are not understood.
        ("ftp://user:pw@h/x/autoconfig", "user/x", "21"),
        ("http://[::1]:8080/x/autoconfig", "[/x", "80"),
        ("https://h", "h", "443"),
        ("https://", "", "443"),
        ("https:///x/.a3s/autoconfig", "/x/.a3s", "443"),
    ];
    for (url, expected_url, expected_port) in cases {
        assert_eq!(
            url_and_port(url),
            (expected_url.to_owned(), expected_port.to_owned()),
            "{url}"
        );
    }
}

#[test]
fn cuts_the_path_at_the_last_autoconfig_segment() {
    let cases = [
        ("ftp://h/R/.a3s/AutoConfig", "h/R/.a3s"),
        ("ftp://h/a/autoconfig/b/autoconfig", "h/a/autoconfig/b"),
        ("ftp://h/x/autoconfigfoo/bar", "h/x"),
        ("ftp://h/R/.a3s/", "h/R/.a3s/"),
        ("ftp://h/p/.a3s/autoconfig?x=1", "h/p/.a3s"),
        // The match is on the path only, never on the host.
        ("ftp://autoconfig/x", "autoconfig/x"),
    ];
    for (url, expected) in cases {
        assert_eq!(parse(url).unwrap().url, expected, "{url}");
    }
}

#[test]
fn import_url_must_end_with_the_file_name() {
    let protocol = parse_import_url(" ftp://88.99.69.243/Repo/WOG/.a3s/autoconfig ").unwrap();
    assert_eq!(protocol.connection_timeout.as_deref(), Some("0"));
    assert_eq!(protocol.read_timeout.as_deref(), Some("0"));
    assert_eq!(protocol.connection_timeout_ms(), 0);
    let mut expected = parse("ftp://88.99.69.243/Repo/WOG/.a3s/autoconfig").unwrap();
    expected.connection_timeout = Some("0".to_owned());
    expected.read_timeout = Some("0".to_owned());
    assert_eq!(protocol, expected);

    for rejected in [
        "ftp://h/.a3s/AUTOCONFIG",
        "ftp://h/.a3s/autoconfig/",
        "autoconfig",
    ] {
        assert_eq!(
            parse_import_url(rejected),
            Err(AutoConfigUrlError::MissingAutoconfigFileName),
            "{rejected}"
        );
    }
    // The file name check runs first; the scheme check after it.
    assert_eq!(
        parse_import_url("sftp://h/.a3s/autoconfig"),
        Err(AutoConfigUrlError::UnsupportedScheme)
    );
}

fn protocol(protocol_type: ProtocolType, url: &str, port: &str) -> Protocol {
    Protocol::new(protocol_type, url, port, "anonymous", "", true)
}

#[test]
fn stored_url_has_no_scheme_and_only_a_non_default_port() {
    let cases = [
        (
            protocol(ProtocolType::Https, "h/r", "443"),
            "h/r/.a3s/autoconfig",
        ),
        (
            protocol(ProtocolType::Https, "h/r", "8443"),
            "h:8443/r/.a3s/autoconfig",
        ),
        (
            protocol(ProtocolType::Ftp, "h", "2121"),
            "h:2121/.a3s/autoconfig",
        ),
        (
            protocol(ProtocolType::Ftp, "h/r/", "21"),
            "h/r//.a3s/autoconfig",
        ),
        (
            protocol(ProtocolType::Ftp, "h/r", "021"),
            "h:021/r/.a3s/autoconfig",
        ),
        (
            protocol(ProtocolType::Sftp, "h/r", "22"),
            "h/r/.a3s/autoconfig",
        ),
        (
            protocol(ProtocolType::Ftp, "h/r", ""),
            "h/r/.a3s/autoconfig",
        ),
    ];
    for (protocol, expected) in cases {
        assert_eq!(
            determine_autoconfig_url(&protocol),
            expected,
            "{protocol:?}"
        );
    }

    let mut no_login = protocol(ProtocolType::Http, "h", "80");
    no_login.login = None;
    assert_eq!(determine_autoconfig_url(&no_login), "h/.a3s/autoconfig");
}

#[test]
fn display_url_adds_the_scheme() {
    assert_eq!(
        autoconfig_display_url(ProtocolType::HttpWebdav, Some("h/.a3s/autoconfig")).as_deref(),
        Some("http://h/.a3s/autoconfig")
    );
    assert_eq!(autoconfig_display_url(ProtocolType::Ftp, Some("")), None);
    assert_eq!(autoconfig_display_url(ProtocolType::Ftp, None), None);
}

#[test]
fn display_url_parses_back_to_the_folder_of_the_repository() {
    for (protocol_type, url, port) in [
        (ProtocolType::Ftp, "88.99.69.243/Repo/WOG", "21"),
        (ProtocolType::Ftp, "h", "2121"),
        (ProtocolType::Http, "h/a/b", "80"),
        (ProtocolType::Https, "h/r", "8443"),
    ] {
        let repository = protocol(protocol_type, url, port);
        let stored = determine_autoconfig_url(&repository);
        let display = autoconfig_display_url(protocol_type, Some(&stored)).unwrap();
        let parsed = parse_import_url(&display).unwrap();
        assert_eq!(parsed.protocol_type, protocol_type);
        assert_eq!(parsed.url, format!("{url}/.a3s"), "{display}");
        assert_eq!(parsed.port, port, "{display}");
    }
}
