//! `java.util.Properties.load(InputStream)`: ISO-8859-1 key/value files.

/// A malformed properties file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PropertiesError {
    /// A `\u` escape without four hex digits (Java `IllegalArgumentException`).
    #[error("Malformed \\uxxxx encoding.")]
    MalformedUnicodeEscape,
}

/// Whitespace that separates keys and starts lines: space, tab, form feed.
fn is_blank(c: u16) -> bool {
    c == u16::from(b' ') || c == u16::from(b'\t') || c == 0x0C
}

const BACKSLASH: u16 = b'\\' as u16;
const CR: u16 = b'\r' as u16;
const LF: u16 = b'\n' as u16;

/// Splits ISO-8859-1 input into logical lines: comments and blank lines skipped, continuation
/// lines (odd number of trailing backslashes) joined without their leading whitespace.
// Mirrors java.util.Properties.LineReader.readLine
fn logical_lines(bytes: &[u8]) -> Vec<Vec<u16>> {
    let mut lines = Vec::new();
    let mut line: Vec<u16> = Vec::new();
    let mut skip_white_space = true;
    let mut in_comment = false;
    let mut appended_line_begin = false;
    let mut preceding_backslash = false;
    let mut skip_lf = false;

    for &byte in bytes {
        let c = u16::from(byte);
        if skip_lf {
            skip_lf = false;
            if c == LF {
                continue;
            }
        }
        if in_comment {
            // A comment ends at the end of its physical line; it never continues.
            if c == LF || c == CR {
                in_comment = false;
                skip_white_space = true;
            }
            continue;
        }
        if skip_white_space {
            if is_blank(c) {
                continue;
            }
            if !appended_line_begin && (c == CR || c == LF) {
                continue;
            }
            skip_white_space = false;
            appended_line_begin = false;
        }
        if line.is_empty() && (c == u16::from(b'#') || c == u16::from(b'!')) {
            in_comment = true;
            continue;
        }
        if c != LF && c != CR {
            line.push(c);
            preceding_backslash = c == BACKSLASH && !preceding_backslash;
            continue;
        }
        // End of a physical line.
        if line.is_empty() {
            skip_white_space = true;
            continue;
        }
        if preceding_backslash {
            // Continuation: drop the backslash and the next line's leading whitespace.
            line.pop();
            skip_white_space = true;
            appended_line_begin = true;
            preceding_backslash = false;
            if c == CR {
                skip_lf = true;
            }
        } else {
            lines.push(std::mem::take(&mut line));
            skip_white_space = true;
        }
    }
    if !line.is_empty() {
        if preceding_backslash {
            line.pop();
        }
        lines.push(line);
    }
    lines
}

/// Resolves escapes: `\t \n \r \f`, `\uXXXX`, and `\x` for any other `x`.
// Mirrors java.util.Properties.loadConvert
fn unescape(chars: &[u16]) -> Result<String, PropertiesError> {
    let mut out: Vec<u16> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c != BACKSLASH {
            out.push(c);
            continue;
        }
        let Some(&escaped) = chars.get(i) else {
            break;
        };
        i += 1;
        let resolved = match u8::try_from(escaped).map(char::from) {
            Ok('u') => {
                let digits = chars
                    .get(i..i + 4)
                    .ok_or(PropertiesError::MalformedUnicodeEscape)?;
                let mut value: u16 = 0;
                for &d in digits {
                    let digit = u8::try_from(d)
                        .ok()
                        .and_then(|d| char::from(d).to_digit(16))
                        .ok_or(PropertiesError::MalformedUnicodeEscape)?;
                    // Four hex digits fit in a u16.
                    value = (value << 4) | digit as u16;
                }
                i += 4;
                value
            }
            Ok('t') => u16::from(b'\t'),
            Ok('r') => CR,
            Ok('n') => LF,
            Ok('f') => 0x0C,
            _ => escaped,
        };
        out.push(resolved);
    }
    Ok(String::from_utf16_lossy(&out))
}

/// Parses a properties file like Java `Properties.load(InputStream)`, in file order.
///
/// Repeated keys appear repeatedly; the caller keeps the last one, as `Properties` does.
// Mirrors java.util.Properties.load0
pub fn parse(bytes: &[u8]) -> Result<Vec<(String, String)>, PropertiesError> {
    let mut out = Vec::new();
    for line in logical_lines(bytes) {
        let limit = line.len();
        let mut key_len = 0;
        let mut value_start = limit;
        let mut has_separator = false;
        let mut preceding_backslash = false;
        while key_len < limit {
            let c = line[key_len];
            if (c == u16::from(b'=') || c == u16::from(b':')) && !preceding_backslash {
                value_start = key_len + 1;
                has_separator = true;
                break;
            } else if is_blank(c) && !preceding_backslash {
                value_start = key_len + 1;
                break;
            }
            preceding_backslash = c == BACKSLASH && !preceding_backslash;
            key_len += 1;
        }
        while value_start < limit {
            let c = line[value_start];
            if !is_blank(c) {
                if !has_separator && (c == u16::from(b'=') || c == u16::from(b':')) {
                    has_separator = true;
                } else {
                    break;
                }
            }
            value_start += 1;
        }
        let key = unescape(&line[..key_len])?;
        let value = unescape(&line[value_start..])?;
        out.push((key, value));
    }
    Ok(out)
}
