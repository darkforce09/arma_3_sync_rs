//! Autoconfig URLs: the address of a repository's `.a3s/autoconfig` file.
//!
//! A repository stores its autoconfig URL without a scheme (`host[:port]/path/.a3s/autoconfig`,
//! see [`determine_autoconfig_url`]) and shows it with the scheme of its protocol
//! ([`autoconfig_display_url`]). Importing a repository from such a URL ([`parse`]) gives a
//! [`Protocol`] that points at the `.a3s` folder; the file to fetch from it is
//! [`AUTOCONFIG_FILE_NAME`].

use crate::java_string::java_trim;
use crate::{Protocol, ProtocolType};

/// Name of the autoconfig file inside a repository's `.a3s` folder.
// Mirrors dao/DataAccessConstants.java:39
pub const AUTOCONFIG_FILE_NAME: &str = "autoconfig";

/// Metadata folder at the root of every repository.
// Mirrors dao/DataAccessConstants.java:38
const A3S_FOLDER: &str = ".a3s";

/// Timeout set on a protocol parsed for an import: no timeout.
const IMPORT_TIMEOUT: &str = "0";

/// Why an autoconfig URL was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AutoConfigUrlError {
    /// The URL does not start with `ftp://`, `http://` or `https://`.
    #[error(
        "Invalid url or unsupported protocol.\nUrl must start with ftp:// or http:// or  https://"
    )]
    UnsupportedScheme,

    /// The URL's last path segment is not exactly `autoconfig`.
    #[error("Url must ends with \"autoconfig\".")]
    MissingAutoconfigFileName,
}

/// Parses an autoconfig URL into the protocol of the `.a3s` folder that holds it.
///
/// The scheme is a case-insensitive prefix (`https://`, `http://` or `ftp://`); the host ends at
/// the first `/` and the port at the first `:` of the host. A port that is not a Java `int`
/// becomes the scheme's default port. Everything from the last `/autoconfig` (any case) of the
/// path is cut, so `ftp://h/R/.a3s/autoconfig` gives the URL `h/R/.a3s`. User info and IPv6
/// literals are not understood, as in Java. The protocol is anonymous and validates TLS
/// certificates.
// Mirrors dao/connection/AutoConfigURLAccessMethods.java:58-98
pub fn parse(url: &str) -> Result<Protocol, AutoConfigUrlError> {
    let trimmed = java_trim(url);
    let protocol_type = [ProtocolType::Https, ProtocolType::Http, ProtocolType::Ftp]
        .into_iter()
        .find(|t| starts_with_ignore_ascii_case(trimmed, t.prompt()))
        .ok_or(AutoConfigUrlError::UnsupportedScheme)?;
    Ok(parse_standard(
        &trimmed[protocol_type.prompt().len()..],
        protocol_type,
    ))
}

/// Parses the URL typed into the import dialog: its last path segment must be exactly
/// `autoconfig`, then [`parse`], with both timeouts disabled.
// Mirrors ui/repository/dialogs/connection/DescriptionPanel.java:234-262
pub fn parse_import_url(url: &str) -> Result<Protocol, AutoConfigUrlError> {
    let trimmed = java_trim(url);
    match trimmed.rsplit_once('/') {
        Some((_, last)) if last == AUTOCONFIG_FILE_NAME => {}
        _ => return Err(AutoConfigUrlError::MissingAutoconfigFileName),
    }
    let mut protocol = parse(trimmed)?;
    protocol.connection_timeout = Some(IMPORT_TIMEOUT.to_owned());
    protocol.read_timeout = Some(IMPORT_TIMEOUT.to_owned());
    Ok(protocol)
}

/// The autoconfig URL stored in a built repository, without scheme.
///
/// The form is `host[:port]/path/.a3s/autoconfig`. The port appears only when it differs from the
/// protocol's default port (exact string comparison). A trailing `/` in the repository URL gives
/// `//.a3s`, as in Java.
// Mirrors dao/connection/AutoConfigURLAccessMethods.java:24-48
pub fn determine_autoconfig_url(protocol: &Protocol) -> String {
    let base = if protocol.port.is_empty() || protocol.port == protocol.protocol_type.default_port()
    {
        protocol.url.clone()
    } else {
        format!(
            "{}:{}{}",
            protocol.hostname(),
            protocol.port,
            protocol.remote_path()
        )
    };
    format!("{base}/{A3S_FOLDER}/{AUTOCONFIG_FILE_NAME}")
}

/// The autoconfig URL as shown to the user: the protocol's scheme followed by the stored URL.
/// `None` when nothing (or an empty string) is stored.
// Mirrors ui/repository/AdminPanel.java:410-413, console/CommandConsole.java:126-130
pub fn autoconfig_display_url(protocol_type: ProtocolType, stored: Option<&str>) -> Option<String> {
    stored
        .filter(|s| !s.is_empty())
        .map(|s| format!("{}{s}", protocol_type.prompt()))
}

// Mirrors dao/connection/AutoConfigURLAccessMethods.java:146-191
fn parse_standard(rest: &str, protocol_type: ProtocolType) -> Protocol {
    let (host_and_port, relative_path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
    let (hostname, port) = host_and_port.split_once(':').unwrap_or((host_and_port, ""));
    let relative_path = rfind_ignore_ascii_case(relative_path, "/autoconfig")
        .map_or(relative_path, |i| &relative_path[..i]);
    let port = if port.parse::<i32>().is_ok() {
        port
    } else {
        protocol_type.default_port()
    };
    Protocol::new(
        protocol_type,
        format!("{hostname}{relative_path}"),
        port,
        "anonymous",
        "",
        true,
    )
}

fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// Byte index of the last ASCII case-insensitive occurrence of the ASCII `needle`.
fn rfind_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let (haystack, needle) = (haystack.as_bytes(), needle.as_bytes());
    (0..=haystack.len().checked_sub(needle.len())?)
        .rev()
        .find(|&i| haystack[i..i + needle.len()].eq_ignore_ascii_case(needle))
}
