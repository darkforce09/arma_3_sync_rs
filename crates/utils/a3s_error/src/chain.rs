use std::error::Error;
use std::fmt;

/// Joins the `Display` text of `err` and of each of its `source()`s with `": "`.
///
/// A source whose text equals, or is a suffix of, the previously emitted text is skipped, so an
/// error declared as `#[error("HTTP: {0}")] Http(#[source] Inner)` is not printed as
/// `HTTP: x: x`.
pub fn format_chain(err: &dyn Error) -> String {
    join_chain(err.to_string(), err.source(), ": ")
}

/// Like [`format_chain`], but one error per line (Java's `core + "\n" + detail` style).
pub fn format_chain_lines(err: &dyn Error) -> String {
    join_chain(err.to_string(), err.source(), "\n")
}

/// Shared implementation: `first` is the text of the outermost error, `source` its first source.
pub(crate) fn join_chain(
    first: String,
    mut source: Option<&(dyn Error + 'static)>,
    separator: &str,
) -> String {
    let mut out = first;
    // Byte offset of the last emitted part, which is always at the end of `out`.
    let mut last_start = 0;
    while let Some(err) = source {
        let text = err.to_string();
        if !text.is_empty() && !out[last_start..].ends_with(text.as_str()) {
            out.push_str(separator);
            last_start = out.len();
            out.push_str(&text);
        }
        source = err.source();
    }
    out
}

/// `Display` adapter for an error chain: `{}` joins with `": "`, `{:#}` with newlines.
#[derive(Clone, Copy)]
pub struct DisplayChain<'a>(pub &'a dyn Error);

impl fmt::Display for DisplayChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            f.write_str(&format_chain_lines(self.0))
        } else {
            f.write_str(&format_chain(self.0))
        }
    }
}

impl fmt::Debug for DisplayChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("c")]
    struct C;

    #[derive(Debug, thiserror::Error)]
    #[error("b")]
    struct B(#[source] C);

    #[derive(Debug, thiserror::Error)]
    #[error("a")]
    struct A(#[source] B);

    #[derive(Debug, thiserror::Error)]
    #[error("inner failure")]
    struct Inner;

    #[derive(Debug, thiserror::Error)]
    enum Transport {
        #[error("HTTP: {0}")]
        Http(#[source] Inner),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("download failed")]
    struct Download(#[source] Transport);

    #[test]
    fn a_e5_three_level_chain() {
        let err = A(B(C));
        assert_eq!(format_chain(&err), "a: b: c");
        assert_eq!(format_chain_lines(&err), "a\nb\nc");
    }

    #[test]
    fn a_e5_no_duplicated_inner_text() {
        let err = Transport::Http(Inner);
        assert_eq!(format_chain(&err), "HTTP: inner failure");
        let err = Download(Transport::Http(Inner));
        assert_eq!(format_chain(&err), "download failed: HTTP: inner failure");
        assert_eq!(
            format_chain_lines(&err),
            "download failed\nHTTP: inner failure"
        );
    }

    #[test]
    fn a_e5_display_chain_alternate_uses_newlines() {
        let err = A(B(C));
        assert_eq!(DisplayChain(&err).to_string(), "a: b: c");
        assert_eq!(format!("{:#}", DisplayChain(&err)), "a\nb\nc");
    }
}
