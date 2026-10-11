//! A minimal model of `OpenJDK` 25's `java.util.HashMap<String, _>`.
//!
//! The build writes maps whose layout (table length, `threshold`, entry order) depends on how
//! Java filled them: sequential `put`s, `putAll` into an empty map, or deserialization. The
//! iteration order also decides which addons a changelog lists first and how carried-over
//! event entries are filtered. This model reproduces all three.
//!
//! Iteration order is "entries stably sorted by bucket, ties in insertion order": inserting
//! appends to a bucket's list and resizing splits each list without reordering it. Bins that Java
//! turns into trees (8 or more colliding keys in a table of at least 64) are not modelled.

use a3s_jserial::jdk::java_string_hash;

/// `HashMap.DEFAULT_INITIAL_CAPACITY`.
const DEFAULT_CAPACITY: i32 = 16;

/// `HashMap.MAXIMUM_CAPACITY`.
const MAXIMUM_CAPACITY: i32 = 1 << 30;

/// `HashMap.DEFAULT_LOAD_FACTOR`.
const LOAD_FACTOR: f32 = 0.75;

/// `HashMap.hash(key)`: the `String` hash with its high bits spread downwards.
// Mirrors java.util.HashMap.hash (OpenJDK 25)
pub fn spread_hash(key: &str) -> i32 {
    let h = java_string_hash(key);
    h ^ ((h as u32) >> 16) as i32
}

/// `HashMap.tableSizeFor(cap)`: the smallest power of two `>= cap` (at least 1).
// Mirrors java.util.HashMap.tableSizeFor (OpenJDK 25)
pub fn table_size_for(cap: i32) -> i32 {
    let n = if cap <= 1 {
        -1
    } else {
        ((-1i32) as u32 >> (cap - 1).leading_zeros()) as i32
    };
    if n < 0 {
        1
    } else if n >= MAXIMUM_CAPACITY {
        MAXIMUM_CAPACITY
    } else {
        n + 1
    }
}

/// One mapping. `K` is what the caller attaches to the key text (e.g. which `String` instance
/// it is); the first key instance of a mapping is kept when the same key is put again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<K, V> {
    pub key: String,
    pub key_payload: K,
    pub value: V,
    hash: i32,
}

/// A `HashMap<String, V>` with Java's capacity, threshold and iteration-order rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaHashMap<K, V> {
    /// Table length; `0` while Java's `table` is still null.
    table_len: i32,
    threshold: i32,

    /// Mappings in insertion order.
    entries: Vec<Entry<K, V>>,
}

impl<K, V> Default for JavaHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> JavaHashMap<K, V> {
    /// `new HashMap<>()`: no table, threshold 0.
    pub fn new() -> Self {
        Self {
            table_len: 0,
            threshold: 0,
            entries: Vec::new(),
        }
    }

    /// The map `ObjectInputStream` rebuilds from `entries` (in stream order).
    ///
    /// JDK 19+ sizes the table like `putAll` does, but never below the default 16:
    /// `max(16, tableSizeFor(ceil(n / 0.75)))` (measured on JDK 25, `java_roundtrip` fixtures).
    // Mirrors java.util.HashMap.readObject (OpenJDK 25)
    pub fn deserialized(entries: impl IntoIterator<Item = (String, K, V)>) -> Self {
        let entries: Vec<_> = entries.into_iter().collect();
        let mut map = Self::new();
        if entries.is_empty() {
            return map;
        }
        let wanted = ceil_div_load_factor(entries.len());
        let cap = if wanted < DEFAULT_CAPACITY {
            DEFAULT_CAPACITY
        } else {
            table_size_for(wanted)
        };
        map.table_len = cap;
        map.threshold = (cap as f32 * LOAD_FACTOR) as i32;
        for (key, payload, value) in entries {
            // `putVal` without resizing: the table is already large enough.
            map.insert(key, payload, value);
        }
        map
    }

    /// Number of mappings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current table length (`0` = no table yet).
    pub fn table_len(&self) -> i32 {
        self.table_len
    }

    /// Current `threshold` field.
    pub fn threshold(&self) -> i32 {
        self.threshold
    }

    /// The mapping for `key`, if any.
    pub fn get(&self, key: &str) -> Option<&Entry<K, V>> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Whether `key` is mapped (`containsKey`).
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Inserts or replaces without any resize logic. Returns whether the key was new.
    fn insert(&mut self, key: String, key_payload: K, value: V) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value;
            return false;
        }
        let hash = spread_hash(&key);
        self.entries.push(Entry {
            key,
            key_payload,
            value,
            hash,
        });
        true
    }

    /// `put(key, value)`: an existing key keeps its key instance and position.
    // Mirrors java.util.HashMap.putVal (OpenJDK 25)
    pub fn put(&mut self, key: String, key_payload: K, value: V) {
        if self.table_len == 0 {
            self.resize();
        }
        if self.insert(key, key_payload, value) && self.entries.len() as i64 > self.threshold as i64
        {
            self.resize();
        }
    }

    /// `putAll(m)` with `m`'s mappings given in `m`'s iteration order (JDK-8281631 sizing).
    // Mirrors java.util.HashMap.putMapEntries (OpenJDK 25)
    pub fn put_all(&mut self, mappings: impl IntoIterator<Item = (String, K, V)>) {
        let mappings: Vec<_> = mappings.into_iter().collect();
        let s = mappings.len();
        if s == 0 {
            return;
        }
        if self.table_len == 0 {
            let t = ceil_div_load_factor(s);
            if t > self.threshold {
                self.threshold = table_size_for(t);
            }
        } else {
            while s as i64 > self.threshold as i64 && self.table_len < MAXIMUM_CAPACITY {
                self.resize();
            }
        }
        for (key, payload, value) in mappings {
            self.put(key, payload, value);
        }
    }

    // Mirrors java.util.HashMap.resize (OpenJDK 25)
    fn resize(&mut self) {
        let old_cap = self.table_len;
        let old_threshold = self.threshold;
        let mut new_threshold = 0;
        let new_cap;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_CAPACITY {
                new_threshold = old_threshold << 1;
            }
        } else if old_threshold > 0 {
            new_cap = old_threshold;
        } else {
            new_cap = DEFAULT_CAPACITY;
            new_threshold = (LOAD_FACTOR * DEFAULT_CAPACITY as f32) as i32;
        }
        if new_threshold == 0 {
            let ft = new_cap as f32 * LOAD_FACTOR;
            new_threshold = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 {
                ft as i32
            } else {
                i32::MAX
            };
        }
        self.threshold = new_threshold;
        self.table_len = new_cap;
    }

    /// Mappings in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = &Entry<K, V>> {
        let mask = self.table_len.max(1) - 1;
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        order.sort_by_key(|&i| self.entries[i].hash & mask);
        order.into_iter().map(|i| &self.entries[i])
    }

    /// `(buckets, threshold)` as `HashMap.writeObject` writes them: the table length, else the
    /// threshold, else 16.
    // Mirrors java.util.HashMap.capacity (OpenJDK 25)
    pub fn write_params(&self) -> (i32, i32) {
        let buckets = if self.table_len > 0 {
            self.table_len
        } else if self.threshold > 0 {
            self.threshold
        } else {
            DEFAULT_CAPACITY
        };
        (buckets, self.threshold)
    }
}

/// `(int) Math.ceil(n / (double) 0.75f)`.
fn ceil_div_load_factor(n: usize) -> i32 {
    let t = (n as f64 / f64::from(LOAD_FACTOR)).ceil();
    if t < f64::from(MAXIMUM_CAPACITY) {
        t as i32
    } else {
        MAXIMUM_CAPACITY
    }
}

#[cfg(test)]
mod tests {
    use super::{JavaHashMap, table_size_for};

    #[test]
    fn table_size_for_like_java() {
        assert_eq!(table_size_for(0), 1);
        assert_eq!(table_size_for(1), 1);
        assert_eq!(table_size_for(2), 2);
        assert_eq!(table_size_for(3), 4);
        assert_eq!(table_size_for(16), 16);
        assert_eq!(table_size_for(17), 32);
    }

    #[test]
    fn put_existing_key_keeps_first_key_and_position() {
        let mut map = JavaHashMap::new();
        map.put("a".to_owned(), 1, "x");
        map.put("b".to_owned(), 2, "y");
        map.put("a".to_owned(), 3, "z");
        let a = map.get("a").unwrap();
        assert_eq!((a.key_payload, a.value), (1, "z"));
        assert_eq!(map.len(), 2);
    }
}
