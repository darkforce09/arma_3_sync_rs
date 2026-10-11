//! Test helpers: a minimal JSON reader for the harness dumps, and identity-dump navigation.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use std::collections::HashMap;
use std::path::Path;

/// A parsed JSON value. Numbers keep their text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Self>),
    Obj(Vec<(String, Self)>),
}

impl Json {
    pub fn parse(text: &str) -> Self {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            pos: 0,
        };
        let value = parser.value();
        parser.ws();
        assert_eq!(parser.pos, parser.bytes.len(), "trailing data in JSON");
        value
    }

    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Num(n) => n.parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Self] {
        match self {
            Self::Arr(items) => items,
            other => panic!("not an array: {other:?}"),
        }
    }

    /// Calls `f` on this value and every nested value.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Self)) {
        f(self);
        match self {
            Self::Arr(items) => items.iter().for_each(|item| item.walk(f)),
            Self::Obj(entries) => entries.iter().for_each(|(_, v)| v.walk(f)),
            _ => {}
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) {
        self.ws();
        assert_eq!(
            self.bytes.get(self.pos),
            Some(&byte),
            "expected {}",
            byte as char
        );
        self.pos += 1;
    }

    fn peek(&mut self) -> u8 {
        self.ws();
        self.bytes[self.pos]
    }

    fn value(&mut self) -> Json {
        match self.peek() {
            b'{' => {
                self.eat(b'{');
                let mut entries = Vec::new();
                if self.peek() == b'}' {
                    self.pos += 1;
                    return Json::Obj(entries);
                }
                loop {
                    let key = self.string();
                    self.eat(b':');
                    entries.push((key, self.value()));
                    if self.peek() == b',' {
                        self.pos += 1;
                    } else {
                        self.eat(b'}');
                        return Json::Obj(entries);
                    }
                }
            }
            b'[' => {
                self.eat(b'[');
                let mut items = Vec::new();
                if self.peek() == b']' {
                    self.pos += 1;
                    return Json::Arr(items);
                }
                loop {
                    items.push(self.value());
                    if self.peek() == b',' {
                        self.pos += 1;
                    } else {
                        self.eat(b']');
                        return Json::Arr(items);
                    }
                }
            }
            b'"' => Json::Str(self.string()),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'n' => self.literal("null", Json::Null),
            _ => {
                let start = self.pos;
                while self.pos < self.bytes.len()
                    && matches!(
                        self.bytes[self.pos],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.pos += 1;
                }
                assert!(self.pos > start, "unexpected byte at {start}");
                Json::Num(String::from_utf8(self.bytes[start..self.pos].to_vec()).unwrap())
            }
        }
    }

    fn literal(&mut self, text: &str, value: Json) -> Json {
        assert!(self.bytes[self.pos..].starts_with(text.as_bytes()));
        self.pos += text.len();
        value
    }

    fn string(&mut self) -> String {
        self.eat(b'"');
        let mut out = Vec::new();
        loop {
            let byte = self.bytes[self.pos];
            self.pos += 1;
            match byte {
                b'"' => return String::from_utf8(out).unwrap(),
                b'\\' => {
                    let escape = self.bytes[self.pos];
                    self.pos += 1;
                    let ch = match escape {
                        b'u' => {
                            let hex =
                                std::str::from_utf8(&self.bytes[self.pos..self.pos + 4]).unwrap();
                            self.pos += 4;
                            char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap()
                        }
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        other => other as char,
                    };
                    let mut buf = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                other => out.push(other),
            }
        }
    }
}

/// An identity-mode dump with its `id` index.
pub struct Dump {
    pub root: Json,
    ids: HashMap<i64, Json>,
}

impl Dump {
    pub fn load(path: &Path) -> Self {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Self {
        let root = Json::parse(text);
        let mut ids = HashMap::new();
        root.walk(&mut |value| {
            if let Some(id) = value.get("id").and_then(Json::as_i64) {
                ids.insert(id, value.clone());
            }
        });
        Self { root, ids }
    }

    /// Follows a `{"ref":N}`.
    pub fn resolve<'a>(&'a self, value: &'a Json) -> &'a Json {
        match value.get("ref").and_then(Json::as_i64) {
            Some(id) => self
                .ids
                .get(&id)
                .unwrap_or_else(|| panic!("dangling ref {id}")),
            None => value,
        }
    }

    /// The (resolved) value of a field of a `fr.soe.a3s` object.
    pub fn field<'a>(&'a self, object: &'a Json, name: &str) -> &'a Json {
        let object = self.resolve(object);
        let fields = object
            .get("fields")
            .unwrap_or_else(|| panic!("not an object dump: {object:?}"));
        self.resolve(
            fields
                .get(name)
                .unwrap_or_else(|| panic!("no field {name} in {object:?}")),
        )
    }

    /// The raw (unresolved) value of a field.
    pub fn raw_field<'a>(object: &'a Json, name: &str) -> &'a Json {
        object.get("fields").and_then(|f| f.get(name)).unwrap()
    }

    /// The text of a (possibly referenced) string object.
    pub fn string(&self, value: &Json) -> Option<String> {
        self.resolve(value)
            .get("str")
            .and_then(Json::as_str)
            .map(str::to_owned)
    }

    pub fn class(&self, value: &Json) -> String {
        self.resolve(value)
            .get("class")
            .and_then(Json::as_str)
            .unwrap()
            .to_owned()
    }

    /// Resolved elements of a list or set.
    pub fn elements<'a>(&'a self, value: &'a Json) -> Vec<&'a Json> {
        self.resolve(value)
            .get("elements")
            .unwrap()
            .as_array()
            .iter()
            .map(|e| self.resolve(e))
            .collect()
    }

    /// String elements of a list or set.
    pub fn strings(&self, value: &Json) -> Vec<String> {
        self.elements(value)
            .into_iter()
            .map(|e| self.string(e).unwrap())
            .collect()
    }

    /// Resolved entries of a map.
    pub fn entries<'a>(&'a self, value: &'a Json) -> Vec<(&'a Json, &'a Json)> {
        self.resolve(value)
            .get("entries")
            .unwrap()
            .as_array()
            .iter()
            .map(|pair| {
                let pair = pair.as_array();
                (self.resolve(&pair[0]), self.resolve(&pair[1]))
            })
            .collect()
    }

    /// String keys of a map.
    pub fn keys(&self, value: &Json) -> Vec<String> {
        self.entries(value)
            .into_iter()
            .map(|(k, _)| self.string(k).unwrap())
            .collect()
    }
}
