use std::collections::HashMap;

use a3s_types::local::Addon;

/// The addons found on disk, keyed by lowercase symbolic key (Java's static `AddonDAO` map).
///
/// A key is the addon folder name, with one `*` appended per earlier addon whose key is
/// already taken ignoring case: the second `@ace` is `@ace*`, the third `@ace**`. Profiles store
/// these keys as leaf names, so allocation order must match Java's.
///
/// Iteration is in insertion order (Java iterates its `HashMap` in hash order, which only
/// matters when two addons both match a lookup).
#[derive(Clone, Debug, Default)]
pub struct AddonRegistry {
    addons: Vec<Addon>,

    /// `lower(key)` of each entry of `addons`.
    lower_keys: Vec<String>,

    /// `lower(key)` → index into `addons`.
    index: HashMap<String, usize>,
}

impl AddonRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.addons.clear();
        self.lower_keys.clear();
        self.index.clear();
    }

    /// The first of `name`, `name*`, `name**`, … whose lowercase form is not a key yet, keeping
    /// the original case.
    // Mirrors dao/AddonDAO.java:20-28
    pub fn determine_new_key(&self, name: &str) -> String {
        let mut key = name.to_owned();
        while self.index.contains_key(&key.to_lowercase()) {
            key.push('*');
        }
        key
    }

    /// Allocates a key for `name` ([`Self::determine_new_key`]), inserts the addon and returns
    /// the key.
    // Mirrors service/AddonService.java:329-333
    pub fn register(&mut self, name: &str, path: String) -> String {
        let key = self.determine_new_key(name);
        self.insert(Addon::new(key.clone(), name, path));
        key
    }

    /// Inserts `addon` under `lower(addon.key)`, replacing any addon with that key in place.
    pub fn insert(&mut self, addon: Addon) {
        let lower = addon.key.to_lowercase();
        if let Some(&i) = self.index.get(&lower) {
            self.addons[i] = addon;
        } else {
            self.index.insert(lower.clone(), self.addons.len());
            self.lower_keys.push(lower);
            self.addons.push(addon);
        }
    }

    /// The addon with this key, ignoring case.
    // Mirrors service/AddonService.java:195-200
    pub fn get(&self, key: &str) -> Option<&Addon> {
        self.index
            .get(&key.to_lowercase())
            .map(|&i| &self.addons[i])
    }

    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(&key.to_lowercase())
    }

    /// Every key, lowercase, in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.lower_keys.iter().map(String::as_str)
    }

    /// Every addon, in insertion order.
    pub fn addons(&self) -> impl Iterator<Item = &Addon> {
        self.addons.iter()
    }

    /// The lowercase keys of every addon whose name equals (case-sensitively) the name of the
    /// addon found under `key`, `key` itself included; empty when `key` is unknown.
    ///
    /// `@ace` and `@ACE*` are not duplicates of each other: their names differ in case.
    // Mirrors dao/AddonDAO.java:40-53
    pub fn duplicates(&self, key: &str) -> Vec<String> {
        let Some(found) = self.get(key) else {
            return Vec::new();
        };
        self.addons
            .iter()
            .zip(&self.lower_keys)
            .filter(|(a, _)| a.name == found.name)
            .map(|(_, key)| key.clone())
            .collect()
    }

    /// Whether another addon has the same name as the one under `key`.
    // Mirrors dao/AddonDAO.java:30-38
    pub fn has_duplicate(&self, key: &str) -> bool {
        self.duplicates(key).len() > 1
    }

    pub fn len(&self) -> usize {
        self.addons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.addons.is_empty()
    }
}
