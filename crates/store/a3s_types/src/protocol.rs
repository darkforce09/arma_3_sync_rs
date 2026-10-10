use a3s_jserial::jdk::Builder;
use a3s_jserial::{ClassData, Graph, Value};
use serde::{Deserialize, Serialize};

use crate::wire::{self, JavaCodec};
use crate::{Error, Result};

/// `fr.soe.a3s.constant.ProtocolType`. Constant names are part of the wire format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtocolType {
    Ftp,
    Ftps,
    Sftp,
    Http,
    Https,
    A3s,
    Socks4,
    Socks5,
    HttpWebdav,
    HttpsWebdav,
}

impl ProtocolType {
    pub const ALL: [Self; 10] = [
        Self::Ftp,
        Self::Ftps,
        Self::Sftp,
        Self::Http,
        Self::Https,
        Self::A3s,
        Self::Socks4,
        Self::Socks5,
        Self::HttpWebdav,
        Self::HttpsWebdav,
    ];

    /// Java enum constant name.
    pub fn java_name(self) -> &'static str {
        match self {
            Self::Ftp => "FTP",
            Self::Ftps => "FTPS",
            Self::Sftp => "SFTP",
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::A3s => "A3S",
            Self::Socks4 => "SOCKS4",
            Self::Socks5 => "SOCKS5",
            Self::HttpWebdav => "HTTP_WEBDAV",
            Self::HttpsWebdav => "HTTPS_WEBDAV",
        }
    }

    pub fn from_java_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.java_name() == name)
    }

    /// Human-readable description, as shown by the Java UI.
    pub fn description(self) -> &'static str {
        match self {
            Self::HttpWebdav => "HTTP/WEBDAV",
            Self::HttpsWebdav => "HTTPS/WEBDAV",
            other => other.java_name(),
        }
    }

    /// URL scheme prefix, e.g. `https://`.
    pub fn prompt(self) -> &'static str {
        match self {
            Self::Ftp => "ftp://",
            Self::Ftps => "ftps://",
            Self::Sftp => "sftp://",
            Self::Http | Self::HttpWebdav => "http://",
            Self::Https | Self::HttpsWebdav => "https://",
            Self::A3s => "a3s://",
            Self::Socks4 => "socks4://",
            Self::Socks5 => "socks5://",
        }
    }

    pub fn default_port(self) -> &'static str {
        match self {
            Self::Ftp | Self::Ftps => "21",
            Self::Sftp => "22",
            Self::Http | Self::HttpWebdav => "80",
            Self::Https | Self::HttpsWebdav => "443",
            Self::A3s => "",
            Self::Socks4 | Self::Socks5 => "1080",
        }
    }

    /// Whether the transport is HTTP-based (the only kind supporting zsync partial transfer).
    pub fn is_http(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }

    /// The concrete Java class used for this protocol (`AbstractProtocoleFactory`).
    fn java_class(self) -> &'static str {
        match self {
            Self::Ftp | Self::Ftps => wire::FTP,
            Self::Sftp => wire::SFTP,
            Self::Http | Self::Https | Self::A3s => wire::HTTP,
            Self::HttpWebdav | Self::HttpsWebdav => wire::WEBDAV,
            Self::Socks4 | Self::Socks5 => wire::SOCKS,
        }
    }

    /// Protocol type implied by a Java class when the `protocolType` field is null.
    fn default_for_class(class: &str) -> Option<Self> {
        Some(match class {
            wire::FTP => Self::Ftp,
            wire::SFTP => Self::Sftp,
            wire::HTTP => Self::Http,
            wire::WEBDAV => Self::HttpWebdav,
            wire::SOCKS => Self::Socks4, // Mirrors domain/Socks.java:20-22
            _ => return None,
        })
    }
}

/// `fr.soe.a3s.constant.EncryptionMode` (deprecated, normally null).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EncryptionMode {
    NoEncryption,
    ExplicitSsl,
    ImplicitSsl,
}

impl EncryptionMode {
    pub fn java_name(self) -> &'static str {
        match self {
            Self::NoEncryption => "NO_ENCRYPTION",
            Self::ExplicitSsl => "EXPLICIT_SSL",
            Self::ImplicitSsl => "IMPLICIT_SSL",
        }
    }

    pub fn from_java_name(name: &str) -> Option<Self> {
        [Self::NoEncryption, Self::ExplicitSsl, Self::ImplicitSsl]
            .into_iter()
            .find(|m| m.java_name() == name)
    }
}

/// Connection settings of a repository (`fr.soe.a3s.domain.AbstractProtocole` and subclasses).
///
/// String-typed fields mirror Java exactly (Java stores ports and timeouts as strings).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    pub protocol_type: ProtocolType,

    /// `host[/path]`, without scheme.
    pub url: String,
    pub port: String,
    pub login: Option<String>,
    pub password: Option<String>,

    /// Milliseconds as a decimal string; `None` means the Java default (300000).
    pub connection_timeout: Option<String>,

    /// Milliseconds as a decimal string; `None` means the Java default (300000).
    pub read_timeout: Option<String>,
    pub validate_ssl_certificate: bool,

    /// Deprecated field, kept for round-tripping.
    pub encryption_mode: Option<EncryptionMode>,
}

/// Default connection and read timeout of the Java implementation, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u32 = 300_000;

impl Protocol {
    /// Creates a protocol the way `AbstractProtocoleFactory.getProtocol` does.
    pub fn new(
        protocol_type: ProtocolType,
        url: impl Into<String>,
        port: impl Into<String>,
        login: impl Into<String>,
        password: impl Into<String>,
        validate_ssl_certificate: bool,
    ) -> Self {
        let validate_ssl_certificate = match protocol_type {
            // `Ftp`'s constructor defaults it, but the factory overrides it with the argument.
            ProtocolType::Sftp => true,
            ProtocolType::Socks4 | ProtocolType::Socks5 => false,
            _ => validate_ssl_certificate,
        };
        Self {
            protocol_type,
            url: url.into(),
            port: port.into(),
            login: Some(login.into()),
            password: Some(password.into()),
            connection_timeout: Some(DEFAULT_TIMEOUT_MS.to_string()),
            read_timeout: Some(DEFAULT_TIMEOUT_MS.to_string()),
            validate_ssl_certificate,
            encryption_mode: None,
        }
    }

    /// Host part of [`Self::url`] (`AbstractProtocole.getHostname`).
    pub fn hostname(&self) -> &str {
        self.url.split_once('/').map_or(&self.url, |(host, _)| host)
    }

    /// Path part of [`Self::url`], starting with `/`, or empty (`getRemotePath`).
    pub fn remote_path(&self) -> &str {
        self.url.find('/').map_or("", |i| &self.url[i..])
    }

    pub fn connection_timeout_ms(&self) -> u32 {
        parse_timeout(self.connection_timeout.as_deref())
    }

    pub fn read_timeout_ms(&self) -> u32 {
        parse_timeout(self.read_timeout.as_deref())
    }

    pub fn port_number(&self) -> Option<u16> {
        self.port.trim().parse().ok()
    }

    /// Whether the login means "no authentication" (`anonymous`, case-insensitive).
    pub fn is_anonymous(&self) -> bool {
        self.login
            .as_deref()
            .is_none_or(|l| l.is_empty() || l.eq_ignore_ascii_case("anonymous"))
    }
}

fn parse_timeout(s: Option<&str>) -> u32 {
    s.and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS)
}

impl JavaCodec for Protocol {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = value.as_ref().ok_or(Error::NullValue("protocole"))?;
        let class = graph.class_name_of(id)?;
        let protocol_type = match graph.enum_name(graph.field(id, "protocolType")?)? {
            Some(name) => {
                ProtocolType::from_java_name(name).ok_or_else(|| Error::UnknownEnumConstant {
                    kind: "ProtocolType",
                    name: name.to_owned(),
                })?
            }
            None => {
                ProtocolType::default_for_class(class).ok_or_else(|| Error::UnexpectedClass {
                    expected: wire::ABSTRACT_PROTOCOLE,
                    found: class.to_owned(),
                })?
            }
        };
        let encryption_mode = match graph.enum_name(graph.field(id, "encryptionMode")?)? {
            Some(name) => Some(EncryptionMode::from_java_name(name).ok_or_else(|| {
                Error::UnknownEnumConstant {
                    kind: "EncryptionMode",
                    name: name.to_owned(),
                }
            })?),
            None => None,
        };
        Ok(Self {
            protocol_type,
            url: graph.field_string(id, "url")?.unwrap_or_default(),
            port: graph.field_string(id, "port")?.unwrap_or_default(),
            login: graph.field_string(id, "login")?,
            password: graph.field_string(id, "password")?,
            connection_timeout: graph.field_string(id, "connectionTimeOut")?,
            read_timeout: graph.field_string(id, "readTimeOut")?,
            validate_ssl_certificate: graph.field_bool(id, "validateSSLCertificate")?,
            encryption_mode,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::protocol_class_desc(b, self.protocol_type.java_class());
        let id = b.reserve_object(class);
        // Field order: validateSSLCertificate, connectionTimeOut, encryptionMode, login,
        // password, port, protocolType, readTimeOut, url.
        let connection_timeout = b.opt_string(self.connection_timeout.as_deref());
        let encryption_mode = self.encryption_mode.map_or(Value::Null, |m| {
            b.enum_value(wire::ENCRYPTION_MODE, m.java_name())
        });
        let login = b.opt_string(self.login.as_deref());
        let password = b.opt_string(self.password.as_deref());
        let port = b.string(&self.port);
        let protocol_type = b.enum_value(wire::PROTOCOL_TYPE, self.protocol_type.java_name());
        let read_timeout = b.opt_string(self.read_timeout.as_deref());
        let url = b.string(&self.url);
        b.set_object_data(
            id,
            vec![
                ClassData {
                    values: vec![
                        Value::Boolean(self.validate_ssl_certificate),
                        connection_timeout,
                        encryption_mode,
                        login,
                        password,
                        port,
                        protocol_type,
                        read_timeout,
                        url,
                    ],
                    annotations: Vec::new(),
                },
                // The concrete subclass has no fields.
                ClassData::default(),
            ],
        );
        Value::Ref(id)
    }
}
