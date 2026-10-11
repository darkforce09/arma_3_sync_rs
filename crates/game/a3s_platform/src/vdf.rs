//! A parser for the subset of Valve's `KeyValues` text format (VDF) used by Steam's
//! `libraryfolders.vdf` and `appmanifest_*.acf`.

use std::fmt;

/// Largest accepted input.
const MAX_INPUT_BYTES: usize = 1 << 20;

/// Deepest accepted object nesting.
const MAX_DEPTH: usize = 32;

/// A value: a string or an object of ordered key/value pairs (duplicate keys kept).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Str(String),
    Obj(Vec<(String, Self)>),
}

/// Why a document was rejected.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParseError(&'static str);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The first value under `key`, compared case-insensitively.
pub(crate) fn get<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

/// The first string under `key`, compared case-insensitively.
pub(crate) fn get_str<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a str> {
    match get(entries, key) {
        Some(Value::Str(text)) => Some(text),
        _ => None,
    }
}

/// The first object under `key`, compared case-insensitively.
pub(crate) fn get_obj<'a>(
    entries: &'a [(String, Value)],
    key: &str,
) -> Option<&'a [(String, Value)]> {
    match get(entries, key) {
        Some(Value::Obj(children)) => Some(children),
        _ => None,
    }
}

/// Parses a whole document into its top-level entries.
pub(crate) fn parse(text: &str) -> Result<Vec<(String, Value)>, ParseError> {
    if text.len() > MAX_INPUT_BYTES {
        return Err(ParseError("document larger than 1 MiB"));
    }
    parse_entries(&mut Tokenizer { rest: text }, 0)
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

/// The entries of one object: the top level (`depth == 0`, ends at the end of input) or a nested
/// object (ends at, and consumes, its closing `}`).
fn parse_entries(
    tokens: &mut Tokenizer<'_>,
    depth: usize,
) -> Result<Vec<(String, Value)>, ParseError> {
    let mut entries = Vec::new();
    loop {
        let key = match tokens.next()? {
            None if depth == 0 => return Ok(entries),
            None => return Err(ParseError("unclosed '{'")),
            Some(Token::Close) if depth > 0 => return Ok(entries),
            Some(Token::Close) => return Err(ParseError("unbalanced '}'")),
            Some(Token::Open) => return Err(ParseError("object without a key")),
            Some(Token::Text(key)) => key,
        };
        let value = match tokens.next()? {
            Some(Token::Text(text)) => Value::Str(text),
            Some(Token::Open) if depth < MAX_DEPTH => Value::Obj(parse_entries(tokens, depth + 1)?),
            Some(Token::Open) => return Err(ParseError("objects nested too deeply")),
            Some(Token::Close) | None => return Err(ParseError("key without a value")),
        };
        entries.push((key, value));
    }
}

struct Tokenizer<'a> {
    rest: &'a str,
}

impl Tokenizer<'_> {
    /// The next token. Whitespace, `//` comments and `[$…]` conditionals are skipped.
    fn next(&mut self) -> Result<Option<Token>, ParseError> {
        loop {
            self.rest = self.rest.trim_start();
            if let Some(after) = self.rest.strip_prefix("//") {
                self.rest = after.find('\n').map_or("", |end| &after[end..]);
                continue;
            }
            if self.rest.starts_with('[') {
                let end = self
                    .rest
                    .find(']')
                    .ok_or(ParseError("unterminated conditional"))?;
                self.rest = &self.rest[end + 1..];
                continue;
            }
            break;
        }
        let Some(first) = self.rest.chars().next() else {
            return Ok(None);
        };
        match first {
            '{' => {
                self.rest = &self.rest[1..];
                Ok(Some(Token::Open))
            }
            '}' => {
                self.rest = &self.rest[1..];
                Ok(Some(Token::Close))
            }
            '"' => self.quoted().map(|text| Some(Token::Text(text))),
            _ => {
                let end = self
                    .rest
                    .find(|c: char| c.is_whitespace() || matches!(c, '{' | '}' | '"'))
                    .unwrap_or(self.rest.len());
                let text = self.rest[..end].to_owned();
                self.rest = &self.rest[end..];
                Ok(Some(Token::Text(text)))
            }
        }
    }

    /// A quoted string with the escapes `\\`, `\"`, `\n` and `\t`; any other `\x` stays as is.
    fn quoted(&mut self) -> Result<String, ParseError> {
        let mut out = String::new();
        let mut chars = self.rest[1..].char_indices();
        while let Some((index, c)) = chars.next() {
            match c {
                '"' => {
                    self.rest = &self.rest[1 + index + 1..];
                    return Ok(out);
                }
                '\\' => match chars.next() {
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, '"')) => out.push('"'),
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, other)) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => break,
                },
                _ => out.push(c),
            }
        }
        Err(ParseError("unterminated string"))
    }
}
