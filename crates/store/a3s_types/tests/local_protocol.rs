//! Protocol helpers.
//!
//! Ports `src/test/java/fr/soe/a3s/domain/ProtocolSupportTest.java`; also covers
//! `AbstractProtocole.checkData` and `getHostUrl`.

use a3s_types::constants::protocol_type_from_description;
use a3s_types::local::ProtocolCheckError;
use a3s_types::local::protocol::{
    check_protocol, host_url, java_protocol_class, new_protocol, parse_java_int,
};
use a3s_types::{Protocol, ProtocolType, encode_stream, wire};

fn class_of_encoded(protocol: &Protocol) -> String {
    let bytes = encode_stream(protocol).unwrap();
    let graph = a3s_jserial::read(&bytes).unwrap();
    let root = graph.root_value().unwrap().as_ref().unwrap();
    graph.class_name_of(root).unwrap().to_owned()
}

#[test]
fn resolves_secure_upload_protocols() {
    assert_eq!(
        protocol_type_from_description("FTPS"),
        Some(ProtocolType::Ftps)
    );
    assert_eq!(
        protocol_type_from_description("SFTP"),
        Some(ProtocolType::Sftp)
    );
    assert_eq!(ProtocolType::Ftps.default_port(), "21");
    assert_eq!(ProtocolType::Sftp.default_port(), "22");
}

#[test]
fn creates_dedicated_transport_models() {
    let ftps = new_protocol(
        ProtocolType::Ftps,
        "ftps.example/updates",
        "21",
        "user",
        "password",
        true,
    )
    .unwrap();
    let sftp = new_protocol(
        ProtocolType::Sftp,
        "sftp.example/updates",
        "22",
        "user",
        "password",
        true,
    )
    .unwrap();

    assert_eq!(class_of_encoded(&ftps), "fr.soe.a3s.domain.Ftp");
    assert!(ftps.validate_ssl_certificate);
    assert_eq!(class_of_encoded(&sftp), "fr.soe.a3s.domain.Sftp");
    assert_eq!(sftp.protocol_type, ProtocolType::Sftp);
    assert!(sftp.validate_ssl_certificate);

    assert!(new_protocol(ProtocolType::A3s, "host", "", "user", "password", true).is_none());
}

#[test]
fn factory_class_mapping() {
    let cases = [
        (ProtocolType::Ftp, Some(wire::FTP)),
        (ProtocolType::Ftps, Some(wire::FTP)),
        (ProtocolType::Sftp, Some(wire::SFTP)),
        (ProtocolType::Http, Some(wire::HTTP)),
        (ProtocolType::Https, Some(wire::HTTP)),
        (ProtocolType::HttpWebdav, Some(wire::WEBDAV)),
        (ProtocolType::HttpsWebdav, Some(wire::WEBDAV)),
        (ProtocolType::Socks4, Some(wire::SOCKS)),
        (ProtocolType::Socks5, Some(wire::SOCKS)),
        (ProtocolType::A3s, None),
    ];
    for (protocol_type, class) in cases {
        assert_eq!(
            java_protocol_class(protocol_type),
            class,
            "{protocol_type:?}"
        );
    }
    // SFTP always validates, SOCKS never; the others take the argument.
    let socks = new_protocol(ProtocolType::Socks5, "proxy", "1080", "", "", true).unwrap();
    assert!(!socks.validate_ssl_certificate);
    let http = new_protocol(ProtocolType::Https, "host", "443", "", "", false).unwrap();
    assert!(!http.validate_ssl_certificate);
    let sftp = new_protocol(ProtocolType::Sftp, "host", "22", "", "", false).unwrap();
    assert!(sftp.validate_ssl_certificate);
}

fn valid() -> Protocol {
    Protocol::new(
        ProtocolType::Https,
        "host/path",
        "443",
        "user",
        "secret",
        true,
    )
}

fn check(edit: impl FnOnce(&mut Protocol)) -> Result<(), ProtocolCheckError> {
    let mut p = valid();
    edit(&mut p);
    check_protocol(&p)
}

#[test]
fn check_protocol_messages_in_priority_order() {
    assert_eq!(check(|_| {}), Ok(()));

    // Every field broken at once: the URL wins, then login, port, ... in Java order.
    let broken = |p: &mut Protocol| {
        p.url = String::new();
        p.login = None;
        p.port = String::new();
        p.connection_timeout = Some(String::new());
        p.read_timeout = Some(String::new());
    };
    let err = check(broken).unwrap_err();
    assert_eq!(err, ProtocolCheckError::UrlEmpty);
    assert_eq!(err, ProtocolCheckError::UrlEmpty);

    let err = check(|p| {
        broken(p);
        p.url = "host".to_owned();
    })
    .unwrap_err();
    assert_eq!(err, ProtocolCheckError::LoginEmpty);
    assert_eq!(
        check(|p| p.login = Some(String::new())),
        Err(ProtocolCheckError::LoginEmpty)
    );

    let err = check(|p| {
        broken(p);
        p.url = "host".to_owned();
        p.login = Some("user".to_owned());
    })
    .unwrap_err();
    assert_eq!(err, ProtocolCheckError::PortEmpty);

    let err = check(|p| p.port = "abc".to_owned()).unwrap_err();
    assert_eq!(err, ProtocolCheckError::PortInvalid);

    let err = check(|p| p.port = "65536".to_owned()).unwrap_err();
    assert_eq!(err, ProtocolCheckError::PortOutOfRange);

    let err = check(|p| p.connection_timeout = Some(String::new())).unwrap_err();
    assert_eq!(err, ProtocolCheckError::ConnectionTimeoutEmpty);
    let err = check(|p| p.connection_timeout = Some("10s".to_owned())).unwrap_err();
    assert_eq!(err, ProtocolCheckError::ConnectionTimeoutInvalid);

    let err = check(|p| p.read_timeout = Some(String::new())).unwrap_err();
    assert_eq!(err, ProtocolCheckError::ReadTimeoutEmpty);
    let err = check(|p| p.read_timeout = Some("1e3".to_owned())).unwrap_err();
    assert_eq!(err, ProtocolCheckError::ReadTimeoutInvalid);

    // The connection timeout is checked before the read timeout.
    let err = check(|p| {
        p.connection_timeout = Some("x".to_owned());
        p.read_timeout = Some(String::new());
    })
    .unwrap_err();
    assert_eq!(err, ProtocolCheckError::ConnectionTimeoutInvalid);
}

#[test]
fn check_protocol_java_quirks() {
    // A null timeout reads as 300000 and passes.
    assert_eq!(
        check(|p| {
            p.connection_timeout = None;
            p.read_timeout = None;
        }),
        Ok(())
    );
    // Negative ports pass `Integer.parseInt` and `> 65535`.
    assert_eq!(check(|p| p.port = "-1".to_owned()), Ok(()));
    assert_eq!(check(|p| p.port = "+21".to_owned()), Ok(()));
    assert_eq!(check(|p| p.port = "65535".to_owned()), Ok(()));
    assert_eq!(
        check(|p| p.port = " 21".to_owned()),
        Err(ProtocolCheckError::PortInvalid)
    );
    assert_eq!(
        check(|p| p.port = "2147483648".to_owned()),
        Err(ProtocolCheckError::PortInvalid)
    );
    assert_eq!(
        check(|p| p.port = "2147483647".to_owned()),
        Err(ProtocolCheckError::PortOutOfRange)
    );
}

#[test]
fn parse_java_int_cases() {
    assert_eq!(parse_java_int("0"), Some(0));
    assert_eq!(parse_java_int("-2147483648"), Some(i32::MIN));
    assert_eq!(parse_java_int("+7"), Some(7));
    assert_eq!(parse_java_int("007"), Some(7));
    for bad in [
        "",
        "+",
        "-",
        "+-1",
        "1 ",
        " 1",
        "1_000",
        "0x10",
        "2147483648",
    ] {
        assert_eq!(parse_java_int(bad), None, "{bad:?}");
    }
}

#[test]
fn host_url_cases() {
    let p = Protocol::new(
        ProtocolType::Https,
        "repo.example/path/to",
        "443",
        "",
        "",
        true,
    );
    assert_eq!(host_url(&p), "https://repo.example:443/path/to");
    let p = Protocol::new(ProtocolType::Https, "repo.example", "8443", "", "", true);
    assert_eq!(host_url(&p), "https://repo.example:8443");
    let p = Protocol::new(
        ProtocolType::HttpWebdav,
        "dav.example/r",
        "80",
        "",
        "",
        false,
    );
    assert_eq!(host_url(&p), "http://dav.example:80/r");
}
