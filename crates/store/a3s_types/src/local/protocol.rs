//! `AbstractProtocole` / `AbstractProtocoleFactory` semantics over [`Protocol`].

use crate::{Protocol, ProtocolType, wire};

/// The Java class `AbstractProtocoleFactory.getProtocol` instantiates for `protocol_type`, or
/// `None` for `A3s` (the factory returns `null`).
// Mirrors domain/AbstractProtocoleFactory.java:7-25
pub fn java_protocol_class(protocol_type: ProtocolType) -> Option<&'static str> {
    Some(match protocol_type {
        ProtocolType::Ftp | ProtocolType::Ftps => wire::FTP,
        ProtocolType::Sftp => wire::SFTP,
        ProtocolType::Http | ProtocolType::Https => wire::HTTP,
        ProtocolType::HttpWebdav | ProtocolType::HttpsWebdav => wire::WEBDAV,
        ProtocolType::Socks4 | ProtocolType::Socks5 => wire::SOCKS,
        ProtocolType::A3s => return None,
    })
}

/// `AbstractProtocoleFactory.getProtocol`: `None` for `A3s`, otherwise [`Protocol::new`]
/// (`validate_ssl` is forced to `true` for SFTP and `false` for SOCKS).
// Mirrors domain/AbstractProtocoleFactory.java:7-25
pub fn new_protocol(
    protocol_type: ProtocolType,
    url: &str,
    port: &str,
    login: &str,
    password: &str,
    validate_ssl: bool,
) -> Option<Protocol> {
    java_protocol_class(protocol_type)?;
    Some(Protocol::new(
        protocol_type,
        url,
        port,
        login,
        password,
        validate_ssl,
    ))
}

/// `scheme://host:port/path` (`AbstractProtocole.getHostUrl`), e.g. `https://host:443/path`.
// Mirrors domain/AbstractProtocole.java:67-89
pub fn host_url(protocol: &Protocol) -> String {
    format!(
        "{}{}:{}{}",
        protocol.protocol_type.prompt(),
        protocol.hostname(),
        protocol.port,
        protocol.remote_path()
    )
}

/// A failed [`check_protocol`]; `Display` is the exact Java message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolCheckError {
    #[error("URL field is empty!")]
    UrlEmpty,

    #[error("Login field is empty!")]
    LoginEmpty,

    #[error("Port field is empty!")]
    PortEmpty,

    #[error("Port is invalid!")]
    PortInvalid,

    #[error("Port number is out of range!\nMaximum port number is 65535.")]
    PortOutOfRange,

    #[error("Connection timeout field is empty!")]
    ConnectionTimeoutEmpty,

    #[error("Connection timeout is invalid!")]
    ConnectionTimeoutInvalid,

    #[error("Read timeout field is empty!")]
    ReadTimeoutEmpty,

    #[error("Read timeout is invalid!")]
    ReadTimeoutInvalid,
}

/// Validates the user-entered fields (`AbstractProtocole.checkData`), in Java's order.
///
/// A missing timeout reads as the default (`300000`) and passes. Ports are only checked with
/// `Integer.parseInt` and `> 65535`, so negative ports pass, as in Java.
// Mirrors domain/AbstractProtocole.java:99-135
pub fn check_protocol(protocol: &Protocol) -> Result<(), ProtocolCheckError> {
    if protocol.url.is_empty() {
        return Err(ProtocolCheckError::UrlEmpty);
    }
    if protocol.login.as_deref().is_none_or(str::is_empty) {
        return Err(ProtocolCheckError::LoginEmpty);
    }
    if protocol.port.is_empty() {
        return Err(ProtocolCheckError::PortEmpty);
    }
    let port = parse_java_int(&protocol.port).ok_or(ProtocolCheckError::PortInvalid)?;
    if port > 65_535 {
        return Err(ProtocolCheckError::PortOutOfRange);
    }
    // A null timeout reads as the default (AbstractProtocole.java:45-65).
    if let Some(timeout) = protocol.connection_timeout.as_deref() {
        if timeout.is_empty() {
            return Err(ProtocolCheckError::ConnectionTimeoutEmpty);
        }
        parse_java_int(timeout).ok_or(ProtocolCheckError::ConnectionTimeoutInvalid)?;
    }
    if let Some(timeout) = protocol.read_timeout.as_deref() {
        if timeout.is_empty() {
            return Err(ProtocolCheckError::ReadTimeoutEmpty);
        }
        parse_java_int(timeout).ok_or(ProtocolCheckError::ReadTimeoutInvalid)?;
    }
    Ok(())
}

/// `Integer.parseInt(s)` (radix 10): an optional leading `+` or `-`, then at least one ASCII
/// digit, no whitespace, and the value must fit in an `i32`.
pub fn parse_java_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // `i32::from_str` accepts the same syntax once the characters are validated.
    s.parse().ok()
}
