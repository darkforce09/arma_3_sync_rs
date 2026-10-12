//! Adversarial checks of the `error` module: chain formatting, classification and Java message texts.

use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::Path;

use a3s_foundation::error::messages::{self, RemoteA3sFile};
use a3s_foundation::error::{DisplayChain, ErrorKind, classify_timeout, format_chain, kind_of};

#[derive(Debug)]
struct Node {
    text: &'static str,
    source: Option<Box<Self>>,
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text)
    }
}

impl StdError for Node {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|node| node as &(dyn StdError + 'static))
    }
}

fn chain(texts: &[&'static str]) -> Node {
    let mut node: Option<Box<Node>> = None;
    for text in texts.iter().rev() {
        node = Some(Box::new(Node { text, source: node }));
    }
    *node.expect("non-empty")
}

#[test]
fn verify_format_chain_dedup_rules() {
    // Equal text is skipped; a non-suffix repeat is kept.
    assert_eq!(format_chain(&chain(&["a", "a", "b"])), "a: b");
    assert_eq!(format_chain(&chain(&["x: y", "y", "z"])), "x: y: z");
    assert_eq!(format_chain(&chain(&["y", "y z"])), "y: y z");
    // Empty sources add nothing (no trailing separator).
    assert_eq!(format_chain(&chain(&["a", "", "b"])), "a: b");
    // Unicode text survives.
    assert_eq!(
        format_chain(&chain(&["échec", "fichier « x »"])),
        "échec: fichier « x »"
    );
    assert_eq!(
        format!("{:#}", DisplayChain(&chain(&["a", "b: c", "c"]))),
        "a\nb: c"
    );
}

#[test]
fn verify_format_chain_through_io_error() {
    // `io::Error::other(inner)` displays the inner text: no duplicate.
    let io = io::Error::other(chain(&["disk full", "write failed"]));
    assert_eq!(format_chain(&io), "disk full: write failed");
}

#[test]
fn verify_kind_of_first_io_error_decides() {
    #[derive(Debug)]
    struct Wrap(io::Error);
    impl fmt::Display for Wrap {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("wrap")
        }
    }
    impl StdError for Wrap {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.0)
        }
    }
    let err = Wrap(io::Error::new(io::ErrorKind::TimedOut, "slow"));
    assert_eq!(kind_of(&err), ErrorKind::Timeout);
    let err = Wrap(io::Error::other(io::Error::from(io::ErrorKind::NotFound)));
    assert_eq!(kind_of(&err), ErrorKind::Io, "outermost io::Error decides");
}

#[test]
fn verify_classify_timeout_substrings() {
    // Java checks `contains`, so `already` (contains `read`) is a read timeout.
    assert_eq!(
        classify_timeout(Some("already closed")),
        ErrorKind::ReadTimeout
    );
    assert_eq!(
        classify_timeout(Some("disconnected")),
        ErrorKind::ConnectTimeout
    );
    assert_eq!(classify_timeout(Some("")), ErrorKind::Timeout);
    assert_eq!(classify_timeout(Some("REad")), ErrorKind::ReadTimeout);
}

#[test]
fn verify_connection_messages_edge_cases() {
    assert_eq!(
        messages::connection("core", ErrorKind::Unauthorized, Some("")),
        "core\n"
    );
    assert_eq!(
        messages::connection("core", ErrorKind::IncompleteTransfer, Some("detail")),
        "detail"
    );
    assert_eq!(
        messages::connection("", ErrorKind::ConnectionFailed, Some("ignored")),
        "\nConnection failed"
    );
    assert_eq!(
        messages::connection("c", ErrorKind::Internal, Some("d\ne")),
        "c\nd\ne"
    );
}

#[test]
fn verify_message_texts_with_odd_names() {
    assert_eq!(
        messages::repository_file_not_found(RemoteA3sFile::ServerInfo, ""),
        "File /.a3s/serverinfo not found on repository: \n"
    );
    assert_eq!(
        messages::cannot_create_directory(Path::new("C:\\Program Files\\Arma3Sync")),
        "Cannot create directory C:\\Program Files\\Arma3Sync\nPlease checkout file access permissions."
    );
    assert_eq!(
        messages::incomplete_transfer("a b.pbo", 0, u64::MAX),
        "Incompete file size transfer: a b.pbo Remote size: 18446744073709551615 Bytes, Actual size: 0 Bytes"
    );
}
