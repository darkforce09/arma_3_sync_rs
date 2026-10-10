/// Placeholder for an absent or empty value.
const EMPTY: &str = "<empty>";

/// Keywords whose values are redacted, in the alternation order of the Java regex.
const KEYWORDS: [&str; 5] = ["password", "passwd", "pwd", "secret", "token"];

/// Makes a value safe for the diagnostic log: no line breaks, no URL credentials, no
/// `password=…`-style secrets.
///
/// 1. `None` or `""` → `<empty>`.
/// 2. `\r` and `\n` become spaces.
/// 3. If the last `@` comes after the first `://` (or there is no `://`), everything before it
///    becomes `<credentials-redacted>` (so a bare `user@host` is redacted too).
/// 4. Every case-insensitive `(password|passwd|pwd|secret|token)\s*[=:]\s*[^&\s]+` becomes
///    `<keyword as written>=<redacted>`.
// Mirrors utils/DebugLogger.java:117-128
pub fn redact(value: Option<&str>) -> String {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return EMPTY.to_owned();
    };
    let mut sanitized = value.replace(['\r', '\n'], " ");
    if let Some(at) = sanitized.rfind('@') {
        let credentials = match sanitized.find("://") {
            Some(scheme) => at > scheme,
            None => true,
        };
        if credentials {
            sanitized = format!("<credentials-redacted>@{}", &sanitized[at + 1..]);
        }
    }
    redact_keywords(&sanitized)
}

/// Hand-written equivalent of Java's
/// `replaceAll("(?i)(password|passwd|pwd|secret|token)\\s*[=:]\\s*[^&\\s]+", "$1=<redacted>")`.
fn redact_keywords(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0; // bytes of `text` already copied to `out`
    let mut i = 0;
    while i < bytes.len() {
        if let Some((keyword_end, match_end)) = match_at(bytes, i) {
            out.push_str(&text[copied..keyword_end]);
            out.push_str("=<redacted>");
            copied = match_end;
            i = match_end;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Tries the regex at byte `start`; returns the end of the keyword and of the whole match.
///
/// Keywords, separators and Java's `\s` are ASCII, so every returned index is a char boundary.
fn match_at(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    KEYWORDS.iter().find_map(|keyword| {
        let keyword_end = start + keyword.len();
        let candidate = bytes.get(start..keyword_end)?;
        if !candidate.eq_ignore_ascii_case(keyword.as_bytes()) {
            return None;
        }
        let mut j = skip_spaces(bytes, keyword_end);
        if !matches!(bytes.get(j), Some(b'=' | b':')) {
            return None;
        }
        j = skip_spaces(bytes, j + 1);
        let value_start = j;
        while j < bytes.len() && bytes[j] != b'&' && !is_java_space(bytes[j]) {
            j += 1;
        }
        (j > value_start).then_some((keyword_end, j))
    })
}

fn skip_spaces(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && is_java_space(bytes[i]) {
        i += 1;
    }
    i
}

/// Java regex `\s`: `[ \t\n\x0B\f\r]`.
fn is_java_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// Safe endpoint description:
/// `protocol={protocol or unknown}, host={host}, port={port}, remotePath={path}` with every value
/// but the protocol passed through [`redact`].
// Mirrors utils/DebugLogger.java:106-115
pub fn describe_endpoint(
    protocol: Option<&str>,
    host: Option<&str>,
    port: Option<&str>,
    remote_path: Option<&str>,
) -> String {
    format!(
        "protocol={}, host={}, port={}, remotePath={}",
        protocol.unwrap_or("unknown"),
        redact(host),
        redact(port),
        redact(remote_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_l1_redact_table() {
        let table: &[(Option<&str>, &str)] = &[
            (None, "<empty>"),
            (Some(""), "<empty>"),
            (Some("a\r\nb"), "a  b"),
            (
                Some("ftp://user:pw@host/path"),
                "<credentials-redacted>@host/path",
            ),
            (Some("user@host"), "<credentials-redacted>@host"),
            (Some("http://host/a@b"), "<credentials-redacted>@b"),
            (Some("user@ftp://host"), "user@ftp://host"),
            (Some("x?Password = s3cret&y=1"), "x?Password=<redacted>&y=1"),
            (Some("mypwd:abc def"), "mypwd=<redacted> def"),
            (Some("token="), "token="),
            (
                Some("PASSWD=1 secret:2"),
                "PASSWD=<redacted> secret=<redacted>",
            ),
            (Some("passwordless"), "passwordless"),
            (Some("TOKEN\t:\tÄbc&x"), "TOKEN=<redacted>&x"),
            (Some("ftp.example.org"), "ftp.example.org"),
            (Some("été pwd=ü"), "été pwd=<redacted>"),
        ];
        for &(input, expected) in table {
            assert_eq!(redact(input), expected, "redact({input:?})");
        }
    }

    #[test]
    fn a_l2_describe_endpoint() {
        assert_eq!(
            describe_endpoint(Some("FTP"), Some("88.99.69.243"), Some("21"), None),
            "protocol=FTP, host=88.99.69.243, port=21, remotePath=<empty>"
        );
        assert_eq!(
            describe_endpoint(None, Some("u:p@host"), Some(""), Some("/Repo/WOG")),
            "protocol=unknown, host=<credentials-redacted>@host, port=<empty>, remotePath=/Repo/WOG"
        );
    }
}
