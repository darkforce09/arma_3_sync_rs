//! Windows registry access: a read/write abstraction, the real registry (Windows only), an
//! in-memory registry for tests, and the process-wide cache Java kept in front of `reg query`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;

/// A registry root key. Java only ever used these two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hive {
    /// `HKEY_CURRENT_USER`.
    CurrentUser,

    /// `HKEY_LOCAL_MACHINE`.
    LocalMachine,
}

impl fmt::Display for Hive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CurrentUser => "HKEY_CURRENT_USER",
            Self::LocalMachine => "HKEY_LOCAL_MACHINE",
        })
    }
}

/// One registry value to read: hive, key path (`\`-separated) and value name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RegQuery {
    /// The root key.
    pub hive: Hive,

    /// The key path below the hive, `\`-separated.
    pub key: String,

    /// The value name; `None` is the key's default value (`reg query /ve`).
    pub value: Option<String>,
}

impl RegQuery {
    /// Builds a query; every `/` in `key` becomes `\`, as Java's `WinRegistry` did.
    // Mirrors dao/WinRegistry.java:90
    pub fn new(hive: Hive, key: &str, value: Option<&str>) -> Self {
        Self {
            hive,
            key: key.replace('/', "\\"),
            value: value.map(str::to_owned),
        }
    }

    /// The case-insensitive identity of the query: Windows matches key paths and value names
    /// without regard to case.
    fn cache_key(&self) -> CacheKey {
        cache_key(self.hive, &self.key, self.value.as_deref())
    }
}

impl fmt::Display for RegQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\\{}", self.hive, self.key)?;
        match &self.value {
            Some(value) => write!(f, " /v {value}"),
            None => f.write_str(" /ve"),
        }
    }
}

type CacheKey = (Hive, String, String);

fn cache_key(hive: Hive, key: &str, value: Option<&str>) -> CacheKey {
    (
        hive,
        key.replace('/', "\\").to_lowercase(),
        value.unwrap_or_default().to_lowercase(),
    )
}

/// Reads string values from the registry.
pub trait RegistryRead: Send + Sync {
    /// The string value of `query`.
    ///
    /// `Ok(None)` when the key or value is missing, is neither `REG_SZ` nor `REG_EXPAND_SZ`, or is
    /// blank after Java's `trim()`. `REG_EXPAND_SZ` comes back with `%VAR%` expanded.
    fn read_string(&self, query: &RegQuery) -> io::Result<Option<String>>;
}

/// Writes string values to the registry.
pub trait RegistryWrite: Send + Sync {
    /// Writes `value` as `REG_SZ`, creating the key if it is missing.
    fn set_string(&self, hive: Hive, key: &str, name: &str, value: &str) -> io::Result<()>;

    /// Deletes a value; a missing key or value is not an error.
    fn delete_value(&self, hive: Hive, key: &str, name: &str) -> io::Result<()>;
}

/// Raw registry data, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegData {
    /// `REG_SZ`.
    Sz(String),

    /// `REG_EXPAND_SZ`: a string with `%VAR%` references.
    ExpandSz(String),

    /// `REG_DWORD`.
    Dword(u32),
}

/// Java's `String.trim()`: strips leading and trailing chars `<= U+0020`.
pub fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Expands `%VAR%` references the way `ExpandEnvironmentStrings` does: a defined variable is
/// replaced, an unknown one stays literally (`%FOO%`), and a lone `%` is kept.
pub fn expand_env(s: &str, var: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(len) = after.find('%') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..len];
        match (!name.is_empty()).then(|| var(name)).flatten() {
            Some(value) => {
                out.push_str(&value);
                rest = &after[len + 1..];
            }
            None if name.is_empty() => {
                // `%%`: keep one `%` and let the second one start a new reference.
                out.push('%');
                rest = after;
            }
            None => {
                out.push('%');
                out.push_str(name);
                out.push('%');
                rest = &after[len + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Turns raw registry data into the string a detection function sees: `REG_SZ` as is,
/// `REG_EXPAND_SZ` expanded, both trimmed like Java's `trim()`; blank strings and other types are
/// `None`.
// Mirrors dao/ConfigurationDAO.java:206-211
pub fn decode_value(data: &RegData, var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let text = match data {
        RegData::Sz(text) => text.clone(),
        RegData::ExpandSz(text) => expand_env(text, var),
        RegData::Dword(_) => return None,
    };
    let trimmed = java_trim(&text);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The real registry: `winreg` on Windows. Elsewhere every read is `Ok(None)` and every write
/// fails with [`io::ErrorKind::Unsupported`].
///
/// Reads use the process's native view: the queried paths spell out `Wow6432Node` where Java did,
/// so a 64-bit process reads what Java's `reg query` read.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRegistry;

#[cfg(windows)]
mod system {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStringExt as _;

    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, RegType::REG_EXPAND_SZ,
        RegType::REG_SZ,
    };

    use super::{Hive, RegData, RegQuery, RegistryRead, RegistryWrite, SystemRegistry};

    fn root(hive: Hive) -> RegKey {
        RegKey::predef(match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        })
    }

    /// Missing keys and values are `None`; every other error is passed on.
    fn found<T>(result: io::Result<T>) -> io::Result<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// UTF-16LE registry string data without its terminating NULs.
    fn utf16_string(bytes: &[u8]) -> String {
        let mut units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        while units.last() == Some(&0) {
            units.pop();
        }
        OsString::from_wide(&units).to_string_lossy().into_owned()
    }

    impl RegistryRead for SystemRegistry {
        fn read_string(&self, query: &RegQuery) -> io::Result<Option<String>> {
            let Some(key) = found(root(query.hive).open_subkey(&query.key))? else {
                return Ok(None);
            };
            let Some(value) = found(key.get_raw_value(query.value.as_deref().unwrap_or("")))?
            else {
                return Ok(None);
            };
            let data = match value.vtype {
                REG_SZ => RegData::Sz(utf16_string(&value.bytes)),
                REG_EXPAND_SZ => RegData::ExpandSz(utf16_string(&value.bytes)),
                _ => return Ok(None),
            };
            Ok(super::decode_value(&data, &|name| std::env::var(name).ok()))
        }
    }

    impl RegistryWrite for SystemRegistry {
        fn set_string(&self, hive: Hive, key: &str, name: &str, value: &str) -> io::Result<()> {
            let (key, _disposition) = root(hive).create_subkey(key.replace('/', "\\"))?;
            key.set_value(name, &value)
        }

        fn delete_value(&self, hive: Hive, key: &str, name: &str) -> io::Result<()> {
            let Some(key) =
                found(root(hive).open_subkey_with_flags(key.replace('/', "\\"), KEY_SET_VALUE))?
            else {
                return Ok(());
            };
            found(key.delete_value(name)).map(|_| ())
        }
    }
}

#[cfg(not(windows))]
impl RegistryRead for SystemRegistry {
    fn read_string(&self, _query: &RegQuery) -> io::Result<Option<String>> {
        Ok(None)
    }
}

#[cfg(not(windows))]
impl RegistryWrite for SystemRegistry {
    fn set_string(&self, _hive: Hive, _key: &str, _name: &str, _value: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn delete_value(&self, _hive: Hive, _key: &str, _name: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// An in-memory registry with Windows' case-insensitive matching, for tests on every OS.
#[derive(Debug, Default)]
pub struct MemoryRegistry {
    values: Mutex<BTreeMap<CacheKey, RegData>>,
    failures: Mutex<HashMap<CacheKey, io::ErrorKind>>,
    env: Vec<(String, String)>,
    queries: AtomicUsize,
}

impl MemoryRegistry {
    /// An empty registry whose `REG_EXPAND_SZ` values expand against `vars` (names matched
    /// case-insensitively, as on Windows).
    pub fn with_env(vars: &[(&str, &str)]) -> Self {
        Self {
            env: vars
                .iter()
                .map(|&(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            ..Self::default()
        }
    }

    /// Stores a value; `name: None` is the key's default value.
    pub fn insert(&self, hive: Hive, key: &str, name: Option<&str>, data: RegData) {
        self.values.lock().insert(cache_key(hive, key, name), data);
    }

    /// Makes every later read of `query` fail with `kind`.
    pub fn fail_with(&self, query: &RegQuery, kind: io::ErrorKind) {
        self.failures.lock().insert(query.cache_key(), kind);
    }

    /// The stored value, if any.
    pub fn get(&self, hive: Hive, key: &str, name: Option<&str>) -> Option<RegData> {
        self.values.lock().get(&cache_key(hive, key, name)).cloned()
    }

    /// Every stored value's hive, lowercase key path and lowercase value name (`""` for the
    /// default value), in sorted order.
    pub fn keys(&self) -> Vec<(Hive, String, String)> {
        self.values.lock().keys().cloned().collect()
    }

    /// How many times [`RegistryRead::read_string`] was called.
    pub fn query_count(&self) -> usize {
        self.queries.load(Ordering::Relaxed)
    }

    fn lookup_var(&self, name: &str) -> Option<String> {
        self.env
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    }
}

impl RegistryRead for MemoryRegistry {
    fn read_string(&self, query: &RegQuery) -> io::Result<Option<String>> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        let key = query.cache_key();
        if let Some(&kind) = self.failures.lock().get(&key) {
            return Err(kind.into());
        }
        let data = self.values.lock().get(&key).cloned();
        Ok(data.and_then(|data| decode_value(&data, &|name| self.lookup_var(name))))
    }
}

impl RegistryWrite for MemoryRegistry {
    fn set_string(&self, hive: Hive, key: &str, name: &str, value: &str) -> io::Result<()> {
        self.insert(hive, key, Some(name), RegData::Sz(value.to_owned()));
        Ok(())
    }

    fn delete_value(&self, hive: Hive, key: &str, name: &str) -> io::Result<()> {
        self.values.lock().remove(&cache_key(hive, key, Some(name)));
        Ok(())
    }
}

#[derive(Debug, Default)]
struct CacheState {
    found: HashMap<CacheKey, String>,
    missing: HashSet<CacheKey>,
}

/// A process-lifetime cache in front of a registry, with Java's positive **and** negative entries:
/// a value that was missing, or whose read failed, is never queried again until [`Self::clear`].
///
/// Writes through the cache drop the written value's entry, so a value this process changed is
/// read fresh.
// Mirrors dao/ConfigurationDAO.java:179-196
#[derive(Debug)]
pub struct CachedRegistry<R> {
    inner: R,
    state: Mutex<CacheState>,
}

impl<R: RegistryRead> CachedRegistry<R> {
    /// Wraps `inner` with an empty cache.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            state: Mutex::new(CacheState::default()),
        }
    }

    /// Forgets every cached result (for a "detect again" action).
    pub fn clear(&self) {
        let mut state = self.state.lock();
        state.found.clear();
        state.missing.clear();
    }

    /// The wrapped registry.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    fn forget(&self, hive: Hive, key: &str, name: &str) {
        let key = cache_key(hive, key, Some(name));
        let mut state = self.state.lock();
        state.found.remove(&key);
        state.missing.remove(&key);
    }
}

impl<R: RegistryRead> RegistryRead for CachedRegistry<R> {
    /// Never fails: a read error is logged and cached as "not found", as in Java.
    fn read_string(&self, query: &RegQuery) -> io::Result<Option<String>> {
        let key = query.cache_key();
        {
            let state = self.state.lock();
            if let Some(value) = state.found.get(&key) {
                return Ok(Some(value.clone()));
            }
            if state.missing.contains(&key) {
                return Ok(None);
            }
        }
        // The lock is not held during the OS query; two threads may both query the same value,
        // which is harmless.
        let value = match self.inner.read_string(query) {
            Ok(value) => value,
            Err(err) => {
                tracing::debug!("registry query {query} failed: {err}");
                None
            }
        };
        let mut state = self.state.lock();
        match &value {
            Some(found) => {
                state.found.insert(key, found.clone());
            }
            None => {
                state.missing.insert(key);
            }
        }
        Ok(value)
    }
}

impl<R: RegistryRead + RegistryWrite> RegistryWrite for CachedRegistry<R> {
    fn set_string(&self, hive: Hive, key: &str, name: &str, value: &str) -> io::Result<()> {
        let result = self.inner.set_string(hive, key, name, value);
        self.forget(hive, key, name);
        result
    }

    fn delete_value(&self, hive: Hive, key: &str, name: &str) -> io::Result<()> {
        let result = self.inner.delete_value(hive, key, name);
        self.forget(hive, key, name);
        result
    }
}

/// The process-wide cached system registry.
pub fn system_registry() -> &'static CachedRegistry<SystemRegistry> {
    static REGISTRY: OnceLock<CachedRegistry<SystemRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| CachedRegistry::new(SystemRegistry))
}
