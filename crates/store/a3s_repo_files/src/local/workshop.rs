//! Workshop metadata cache (`service/WorkshopMetadataStore.java`): Steam Workshop id and name of
//! addons, which a mod folder does not record.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::naming::{java_is_blank as is_blank, java_trim};
use super::{Error, LocalStore, toml_file};

/// The Workshop metadata of one addon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopEntry {
    /// The addon key (folder name).
    pub addon_key: String,

    /// The Workshop item id, a positive decimal number.
    pub published_id: String,

    /// The Workshop item name.
    pub name: String,
}

/// One `[workshop.mods."<key>"]` table.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Record {
    published_id: String,
    name: String,
}

/// `[workshop]`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct WorkshopFile {
    mods: BTreeMap<String, Record>,
}

/// Access to `config/workshop_mods.toml`, from [`LocalStore::workshop`].
#[derive(Clone, Copy, Debug)]
pub struct WorkshopMetadata<'a> {
    store: &'a LocalStore,
}

/// Java `matches("[1-9][0-9]*")`, on the untrimmed value.
fn is_positive_number(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b'1'..=b'9')) && bytes.all(|b| b.is_ascii_digit())
}

impl<'a> WorkshopMetadata<'a> {
    pub(crate) fn new(store: &'a LocalStore) -> Self {
        Self { store }
    }

    /// The current file; an unreadable or corrupt one reads as empty, as in Java.
    // Mirrors service/WorkshopMetadataStore.java:106-117
    fn load(&self) -> WorkshopFile {
        match toml_file::read::<WorkshopFile>(&self.store.workshop_path(), "workshop") {
            Ok(file) => file.unwrap_or_default(),
            Err(err) => {
                tracing::warn!("Ignoring unreadable Workshop cache: {}", err.detail());
                WorkshopFile::default()
            }
        }
    }

    /// The entry of `addon_key`, trimmed; `None` for a blank key or a blank stored value.
    // Mirrors service/WorkshopMetadataStore.java:42-53
    pub fn find(&self, addon_key: &str) -> Option<WorkshopEntry> {
        if is_blank(addon_key) {
            return None;
        }
        let file = self.load();
        let record = file.mods.get(addon_key)?;
        valid_entry(addon_key, record)
    }

    /// Every entry that [`Self::find`] would return, by addon key.
    pub fn all(&self) -> BTreeMap<String, WorkshopEntry> {
        self.load()
            .mods
            .iter()
            .filter(|(key, _)| !is_blank(key))
            .filter_map(|(key, record)| Some((key.clone(), valid_entry(key, record)?)))
            .collect()
    }

    /// Validates and stores an entry, replacing an existing one for the same key.
    // Mirrors service/WorkshopMetadataStore.java:55-100
    pub fn save(&self, addon_key: &str, published_id: &str, name: &str) -> Result<(), Error> {
        if is_blank(addon_key) {
            return Err(Error::InvalidWorkshopEntry("Addon key must not be empty."));
        }
        if !is_positive_number(published_id) {
            return Err(Error::InvalidWorkshopEntry(
                "Workshop ID must be a positive number.",
            ));
        }
        if is_blank(name) {
            return Err(Error::InvalidWorkshopEntry(
                "Workshop name must not be empty.",
            ));
        }
        let _guard = self.store.write_lock.lock();
        let mut file = self.load();
        file.mods.insert(
            addon_key.to_owned(),
            Record {
                published_id: java_trim(published_id).to_owned(),
                name: java_trim(name).to_owned(),
            },
        );
        toml_file::write(&self.store.workshop_path(), "workshop", &file)
    }

    /// Adds the entries whose key is not present yet (values trimmed, no validation), in one
    /// atomic write. Returns the keys that were added.
    pub fn insert_missing(&self, entries: &[WorkshopEntry]) -> Result<Vec<String>, Error> {
        let _guard = self.store.write_lock.lock();
        let mut file = self.load();
        let mut added = Vec::new();
        for entry in entries {
            if file.mods.contains_key(&entry.addon_key) {
                continue;
            }
            file.mods.insert(
                entry.addon_key.clone(),
                Record {
                    published_id: java_trim(&entry.published_id).to_owned(),
                    name: java_trim(&entry.name).to_owned(),
                },
            );
            added.push(entry.addon_key.clone());
        }
        if !added.is_empty() {
            toml_file::write(&self.store.workshop_path(), "workshop", &file)?;
        }
        Ok(added)
    }
}

fn valid_entry(addon_key: &str, record: &Record) -> Option<WorkshopEntry> {
    if is_blank(&record.published_id) || is_blank(&record.name) {
        return None;
    }
    Some(WorkshopEntry {
        addon_key: addon_key.to_owned(),
        published_id: java_trim(&record.published_id).to_owned(),
        name: java_trim(&record.name).to_owned(),
    })
}
