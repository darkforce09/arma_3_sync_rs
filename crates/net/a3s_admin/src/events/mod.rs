//! Editing a repository's events (modsets) and saving or uploading `.a3s/events`.
//!
//! [`EventsDoc`] models the Java `Events` object the admin panel edits in memory: deserialized
//! from the remote `.a3s/events`, changed by the event dialogs, then written with
//! `ObjectOutputStream`. The written bytes depend on more than the values: which `String` and
//! `Boolean` instances are shared, and each `HashMap`'s table length and insertion history.
//! The document therefore tracks an identity for every string and boolean
//! ([`EventsDoc::encode`] writes one object per identity) and keeps each map as a model of
//! Java's `HashMap`.
//!
//! Identity limits: keys saved from a selection are one instance per text and selection list
//! (Java shares the tree node's name instance, which differs only when two tree nodes of the
//! same list have the same name), and the order of saved keys that share a bucket assumes one
//! save per refresh of the event list.

#![allow(
    clippy::result_large_err,
    reason = "the errors carry transport errors; they are returned once per operation"
)]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_foundation::error::{ErrorKind, UserFacing};
use a3s_jserial::jdk::{self, Builder};
use a3s_jserial::{Graph, NodeId, Value};
use a3s_repo_files::A3sFile;
use a3s_repo_files::layout::A3S_FOLDER;
use a3s_sync::{RemoteConnection, RemoteError};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::local::Repository;
use a3s_types::{Changelogs, Event, Events, ServerInfo, SyncTreeDirectory, wire};

use crate::build::carry::RawGraph;
use crate::build::java_hash::JavaHashMap;
use crate::check::transport_kind;
use crate::upload::distinct_boolean;

/// Message after [`EventsDoc::write_local`].
// Mirrors ui/repository/EventsPanel.java:604-605
pub const EVENTS_SAVED_MESSAGE: &str = "Events information has been saved to the repository.";

/// Message after [`upload_events`].
// Mirrors ui/repository/dialogs/progress/ProgressUploadEventsDialog.java:59-60
pub const EVENTS_UPLOADED_MESSAGE: &str = "Events information has been uploaded to the repository.";

/// Appended after `"\n"` to the message of a failed [`upload_events`].
// Mirrors ui/repository/dialogs/progress/ProgressUploadEventsDialog.java:71-75
pub const UPLOAD_FAILURE_HINT: &str = "Please check upload options and server permissions.";

/// Which selection list a saved key comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SelectionList {
    Addons,
    Dlcs,
    Userconfig,
}

/// A `String` reference of the Java object graph, with its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
enum JavaString {
    Null,

    /// Deserialized: one instance per node of the old stream.
    Old(NodeId, String),

    /// Created by an edit: one instance per id, possibly referenced from several places.
    Fresh(u64, String),

    /// A key saved from a selection: one instance per text and list.
    Selection(SelectionList, String),
}

impl JavaString {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Null => None,
            Self::Old(_, s) | Self::Fresh(_, s) | Self::Selection(_, s) => Some(s),
        }
    }
}

/// A `java.lang.Boolean` reference of the Java object graph, with its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JavaBoolean {
    Null,

    /// Deserialized: one (non-canonical) object per node of the old stream.
    Old(NodeId, bool),

    /// Autoboxed: `Boolean.TRUE` or `Boolean.FALSE`.
    Canonical(bool),
}

impl JavaBoolean {
    fn value(self) -> bool {
        match self {
            Self::Null => false,
            Self::Old(_, v) | Self::Canonical(v) => v,
        }
    }
}

/// A `HashMap<String, Boolean>` field; `None` is a Java `null`.
type BoolMap = JavaHashMap<JavaString, JavaBoolean>;

/// One `fr.soe.a3s.domain.repository.Event`.
#[derive(Clone, Debug, PartialEq)]
struct DocEvent {
    name: JavaString,
    description: JavaString,
    addon_names: Option<BoolMap>,
    dlc_names: Option<BoolMap>,
    userconfig_folder_names: Option<BoolMap>,
}

impl DocEvent {
    fn name(&self) -> &str {
        self.name.as_str().unwrap_or_default()
    }

    /// `getAddonNames()` / `getDlcNames()` / `getUserconfigFolderNames()`: a null map becomes a
    /// new empty `HashMap`.
    // Mirrors domain/repository/Event.java:38-55
    fn map_mut(&mut self, list: SelectionList) -> &mut BoolMap {
        let map = match list {
            SelectionList::Addons => &mut self.addon_names,
            SelectionList::Dlcs => &mut self.dlc_names,
            SelectionList::Userconfig => &mut self.userconfig_folder_names,
        };
        map.get_or_insert_with(JavaHashMap::new)
    }

    fn to_event(&self) -> Event {
        let map = |m: &Option<BoolMap>| -> BTreeMap<String, bool> {
            m.iter()
                .flat_map(JavaHashMap::iter)
                .map(|e| (e.key.clone(), e.value.value()))
                .collect()
        };
        Event {
            name: self.name().to_owned(),
            description: self.description.as_str().map(str::to_owned),
            addon_names: map(&self.addon_names),
            dlc_names: map(&self.dlc_names),
            userconfig_folder_names: map(&self.userconfig_folder_names),
        }
    }
}

/// The selection saved into an event, each list in the UI's tree order. Values are the
/// "optional" flags. Entries are `put` in order: a repeated key keeps its position and gets
/// the later value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventSelection {
    /// Addon folder names.
    pub addon_names: Vec<(String, bool)>,

    /// `GameDlc` constant names.
    pub dlc_names: Vec<(String, bool)>,

    /// Folder names below `userconfig/`.
    pub userconfig_folder_names: Vec<(String, bool)>,
}

impl EventSelection {
    fn list(&self, list: SelectionList) -> &[(String, bool)] {
        match list {
            SelectionList::Addons => &self.addon_names,
            SelectionList::Dlcs => &self.dlc_names,
            SelectionList::Userconfig => &self.userconfig_folder_names,
        }
    }
}

/// The events of a repository as the admin panel edits them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventsDoc {
    events: Vec<DocEvent>,
    next_fresh: u64,
}

/// A rejected event name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EventNameError {
    /// The trimmed name is empty (Java ignores the dialog's OK).
    #[error("empty name")]
    Empty,

    /// Another event has this name.
    // Mirrors ui/repository/events/EventAddDialog.java:33-35
    #[error("duplicate name!")]
    Duplicate,
}

/// Validates a name typed into the add or rename dialog: trimmed, not empty, not the name of
/// another event. `renaming` is the event being renamed, whose own name stays allowed.
// Mirrors ui/repository/events/EventAddDialog.java:26-40, EventRenameDialog.java:39-54
pub fn validate_event_name(
    doc: &EventsDoc,
    raw: &str,
    renaming: Option<&str>,
) -> Result<String, EventNameError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(EventNameError::Empty);
    }
    let taken = doc
        .events()
        .iter()
        .any(|e| e.name == name && Some(e.name.as_str()) != renaming);
    if taken {
        return Err(EventNameError::Duplicate);
    }
    Ok(name.to_owned())
}

/// Why an events operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EventsError {
    /// The repository has no main folder.
    // Mirrors ui/repository/EventsPanel.java:594-597
    #[error(
        "Repository main folder location is missing.\nPlease check the Repository Administration panel."
    )]
    PathNotSet,

    /// The main folder does not exist.
    // Mirrors ui/repository/EventsPanel.java:598-600
    #[error("Repository main folder location: {0} does not exist.")]
    PathNotFound(String),

    /// Writing `.a3s/events` failed.
    // Mirrors dao/repository/RepositoryDAO.java:305-308
    #[error("Failed to write file: {}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: a3s_repo_files::Error,
    },

    /// The repository has no upload protocol.
    #[error("Please use the upload options to configure a connection.")]
    UploadProtocolNotSet,

    /// An events stream cannot be read or written.
    #[error("{0}")]
    Decode(String),

    /// Connecting failed.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// The upload failed.
    #[error("{source}")]
    Transport {
        /// The transport error.
        #[source]
        source: a3s_transport::Error,
    },
}

impl UserFacing for EventsError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::PathNotSet | Self::UploadProtocolNotSet => ErrorKind::InvalidInput,
            Self::PathNotFound(_) => ErrorKind::NotFound,
            Self::Write { .. } => ErrorKind::WriteFailed,
            Self::Decode(_) => ErrorKind::InvalidData,
            Self::Remote(err) => err.kind(),
            Self::Transport { source } => transport_kind(source),
        }
    }

    fn user_message(&self) -> String {
        self.to_string()
    }
}

impl EventsDoc {
    /// `new Events()`: no events.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes a gzip `.a3s/events` file (see [`Self::from_graph`]).
    pub fn decode(gzipped: &[u8]) -> Result<Self, EventsError> {
        let raw: RawGraph =
            a3s_repo_files::decode_a3s(gzipped).map_err(|e| EventsError::Decode(e.to_string()))?;
        Self::from_graph(&raw.graph)
    }

    /// The `Events` object Java deserializes from `graph`, after the admin panel's first
    /// `getEvents()`, which turns null `dlcNames` and `userconfigFolderNames` into new empty
    /// maps. A null `addonNames` (Java throws) becomes an empty map too.
    // Mirrors ui/repository/dialogs/progress/ProgressConnectionAsAdminDialog.java:51-67,
    // service/RepositoryService.java:866-892, service/ObjectDTOtransformer.java:383-397
    pub fn from_graph(graph: &Graph) -> Result<Self, EventsError> {
        decode_events(graph).map_err(|e| EventsError::Decode(e.to_string()))
    }

    fn fresh(&mut self, text: &str) -> JavaString {
        self.next_fresh += 1;
        JavaString::Fresh(self.next_fresh, text.to_owned())
    }

    /// The events as the UI lists them: in order, the first of each name only.
    // Mirrors service/RepositoryService.java:866-892
    pub fn events(&self) -> Vec<Event> {
        let mut out: Vec<Event> = Vec::new();
        for event in &self.events {
            if !out.iter().any(|e| e.name == event.name()) {
                out.push(event.to_event());
            }
        }
        out
    }

    /// Every event, as written.
    pub fn to_events(&self) -> Events {
        Events {
            list: self.events.iter().map(DocEvent::to_event).collect(),
        }
    }

    /// Appends a new event. The add dialog passes an empty selection; the selection's userconfig
    /// list is ignored, as by Java.
    // Mirrors service/RepositoryService.java:979-991, service/ObjectDTOtransformer.java:361-376
    pub fn add_event(&mut self, name: &str, description: &str, selection: &EventSelection) {
        let name = self.fresh(name);
        let description = self.fresh(description);
        let mut event = DocEvent {
            name,
            description,
            addon_names: Some(JavaHashMap::new()),
            dlc_names: Some(JavaHashMap::new()),
            userconfig_folder_names: Some(JavaHashMap::new()),
        };
        for list in [SelectionList::Addons, SelectionList::Dlcs] {
            let entries = selection_order(list, selection.list(list), 0);
            let map = event.map_mut(list);
            for (key, value) in entries {
                map.put(key.as_str().unwrap_or_default().to_owned(), key, value);
            }
        }
        self.events.push(event);
    }

    /// Renames **every** event named `name`; they share the new name and description instances.
    /// Returns the number of events renamed.
    // Mirrors service/RepositoryService.java:993-1010
    pub fn rename_event(&mut self, name: &str, new_name: &str, new_description: &str) -> usize {
        let new_name = self.fresh(new_name);
        let new_description = self.fresh(new_description);
        let mut renamed = 0;
        for event in self.events.iter_mut().filter(|e| e.name() == name) {
            event.name = new_name.clone();
            event.description = new_description.clone();
            renamed += 1;
        }
        renamed
    }

    /// Appends a copy of the **first** event named `name` as `"<name> - duplicate"`, sharing its
    /// description, keys and values. Returns whether an event was found.
    // Mirrors service/RepositoryService.java:1012-1037
    pub fn duplicate_event(&mut self, name: &str) -> bool {
        let new_name = self.fresh(&format!("{name} - duplicate"));
        let Some(source) = self.events.iter_mut().find(|e| e.name() == name) else {
            return false;
        };
        let copy = |map: &BoolMap| {
            let mut copy = JavaHashMap::new();
            copy.put_all(
                map.iter()
                    .map(|e| (e.key.clone(), e.key_payload.clone(), e.value)),
            );
            copy
        };
        let addon_names = source
            .addon_names
            .as_ref()
            .map_or_else(JavaHashMap::new, copy);
        let dlc_names = copy(source.map_mut(SelectionList::Dlcs));
        let userconfig_folder_names = copy(source.map_mut(SelectionList::Userconfig));
        let duplicate = DocEvent {
            name: new_name,
            description: source.description.clone(),
            addon_names: Some(addon_names),
            dlc_names: Some(dlc_names),
            userconfig_folder_names: Some(userconfig_folder_names),
        };
        self.events.push(duplicate);
        true
    }

    /// Removes the **first** event named `name`. Returns whether one was removed.
    // Mirrors service/RepositoryService.java:1039-1059
    pub fn remove_event(&mut self, name: &str) -> bool {
        let Some(index) = self.events.iter().position(|e| e.name() == name) else {
            return false;
        };
        self.events.remove(index);
        true
    }

    /// Replaces the maps of **every** event named `name` with `selection` (`clear()` keeps each
    /// map's table, so maps never shrink). Returns the number of events updated.
    // Mirrors ui/repository/EventsPanel.java:846-861, service/RepositoryService.java:1155-1186
    pub fn save_event(&mut self, name: &str, selection: &EventSelection) -> usize {
        let Some(first) = self.events.iter().find(|e| e.name() == name) else {
            return 0;
        };
        // The UI's `EventDTO` maps were filled from the first event, then cleared and refilled.
        let previous_len = |map: &Option<BoolMap>| map.as_ref().map_or(0, JavaHashMap::len);
        let lists = [
            (SelectionList::Addons, previous_len(&first.addon_names)),
            (SelectionList::Dlcs, previous_len(&first.dlc_names)),
            (
                SelectionList::Userconfig,
                previous_len(&first.userconfig_folder_names),
            ),
        ]
        .map(|(list, previous)| (list, selection_order(list, selection.list(list), previous)));
        let mut updated = 0;
        for event in self.events.iter_mut().filter(|e| e.name() == name) {
            for (list, entries) in &lists {
                let map = event.map_mut(*list);
                map.clear();
                for (key, value) in entries {
                    map.put(
                        key.as_str().unwrap_or_default().to_owned(),
                        key.clone(),
                        *value,
                    );
                }
            }
            updated += 1;
        }
        updated
    }

    /// The uncompressed stream of Java's `writeObject(events)`.
    fn encode_stream(&self) -> Result<Vec<u8>, EventsError> {
        let mut b = Builder::new();
        let root = Encoder::default().events(&mut b, &self.events);
        let raw =
            a3s_jserial::write(&b.finish(root)).map_err(|e| EventsError::Decode(e.to_string()))?;
        crate::build::write::check_stream(&raw).map_err(|e| EventsError::Decode(e.to_string()))?;
        Ok(raw)
    }

    /// The `.a3s/events` file bytes (gzip), as Java writes them.
    // Mirrors dao/A3SFilesAccessor.java:144-162
    pub fn encode(&self) -> Result<Vec<u8>, EventsError> {
        a3s_repo_files::encode_a3s_stream(&self.encode_stream()?)
            .map_err(|e| EventsError::Decode(e.to_string()))
    }

    /// Writes `<repository_path>/.a3s/events` atomically, creating `.a3s` (one level) if needed.
    /// Returns the file's path.
    // Mirrors service/RepositoryService.java:1061-1067, dao/repository/RepositoryDAO.java:290-311
    pub fn write_local(&self, repository_path: Option<&str>) -> Result<PathBuf, EventsError> {
        let root = match repository_path {
            Some(path) if !path.is_empty() => Path::new(path),
            _ => return Err(EventsError::PathNotSet),
        };
        if !root.exists() {
            return Err(EventsError::PathNotFound(root.display().to_string()));
        }
        let folder = root.join(A3S_FOLDER);
        let path = folder.join(A3sFile::Events.file_name());
        let write_error = |source| EventsError::Write {
            path: path.clone(),
            source,
        };
        if let Err(source) = std::fs::create_dir(&folder)
            && !folder.is_dir()
        {
            return Err(write_error(a3s_repo_files::Error::Io {
                path: folder.clone(),
                source,
            }));
        }
        let raw = self.encode_stream()?;
        a3s_repo_files::write_a3s_stream(&path, &raw).map_err(write_error)?;
        Ok(path)
    }
}

/// The order in which the UI's `EventDTO` map hands `entries` to the service: a `HashMap` that
/// held `previous_len` entries, cleared, then filled with `entries` in order.
// Mirrors ui/repository/EventsPanel.java:846-861 and service/ObjectDTOtransformer.java:383-397
fn selection_order(
    list: SelectionList,
    entries: &[(String, bool)],
    previous_len: usize,
) -> Vec<(JavaString, JavaBoolean)> {
    let mut dto: JavaHashMap<(), bool> = JavaHashMap::new();
    for i in 0..previous_len {
        dto.put(format!("\u{0}{i}"), (), false);
    }
    dto.clear();
    for (key, value) in entries {
        dto.put(key.clone(), (), *value);
    }
    dto.iter()
        .map(|e| {
            (
                JavaString::Selection(list, e.key.clone()),
                JavaBoolean::Canonical(e.value),
            )
        })
        .collect()
}

/// Decodes the `Events` graph into the document model.
fn decode_events(graph: &Graph) -> a3s_jserial::Result<EventsDoc> {
    let root = graph.root_value()?;
    let root = root.as_ref().ok_or(a3s_jserial::Error::UnexpectedNull)?;
    jdk::expect_class(graph, root, wire::EVENTS)?;
    let list = jdk::list(graph, graph.field(root, "list")?)?.unwrap_or_default();
    let mut events = Vec::with_capacity(list.len());
    for element in list {
        let id = element.as_ref().ok_or(a3s_jserial::Error::UnexpectedNull)?;
        jdk::expect_class(graph, id, wire::EVENT)?;
        let field = |name| -> a3s_jserial::Result<Value> {
            Ok(graph.opt_field(id, name)?.unwrap_or(Value::Null))
        };
        let map = |name| -> a3s_jserial::Result<Option<BoolMap>> {
            Ok(Some(decode_map(graph, field(name)?)?.unwrap_or_default()))
        };
        events.push(DocEvent {
            name: decode_string(graph, field("name")?)?,
            description: decode_string(graph, field("description")?)?,
            addon_names: map("addonNames")?,
            dlc_names: map("dlcNames")?,
            userconfig_folder_names: map("userconfigFolderNames")?,
        });
    }
    Ok(EventsDoc {
        events,
        next_fresh: 0,
    })
}

fn decode_string(graph: &Graph, value: Value) -> a3s_jserial::Result<JavaString> {
    Ok(match value {
        Value::Ref(id) => JavaString::Old(id, graph.string(id)?.to_owned()),
        _ => JavaString::Null,
    })
}

/// A `HashMap<String, Boolean>` as `HashMap.readObject` rebuilds it; `None` for null.
fn decode_map(graph: &Graph, value: Value) -> a3s_jserial::Result<Option<BoolMap>> {
    let Value::Ref(id) = value else {
        return Ok(None);
    };
    let class = graph.class_name_of(id)?;
    if class != jdk::HASH_MAP {
        return Err(a3s_jserial::Error::UnsupportedCollection(class.to_owned()));
    }
    let mut entries = Vec::new();
    for (key, value) in jdk::map(graph, value)?.unwrap_or_default() {
        let key = decode_string(graph, key)?;
        let Some(text) = key.as_str().map(str::to_owned) else {
            return Err(a3s_jserial::Error::UnexpectedNull);
        };
        let value = match (value, jdk::boolean(graph, value)?) {
            (Value::Ref(id), Some(flag)) => JavaBoolean::Old(id, flag),
            _ => JavaBoolean::Null,
        };
        entries.push((text, key, value));
    }
    Ok(Some(JavaHashMap::deserialized(entries)))
}

/// Writes the model with one object per identity.
#[derive(Default)]
struct Encoder {
    old_strings: HashMap<NodeId, Value>,
    fresh_strings: HashMap<u64, Value>,
    selection_strings: HashMap<(SelectionList, String), Value>,
    old_booleans: HashMap<NodeId, Value>,
}

impl Encoder {
    fn string(&mut self, b: &mut Builder, s: &JavaString) -> Value {
        match s {
            JavaString::Null => Value::Null,
            JavaString::Old(id, text) => *self
                .old_strings
                .entry(*id)
                .or_insert_with(|| b.string(text)),
            JavaString::Fresh(id, text) => *self
                .fresh_strings
                .entry(*id)
                .or_insert_with(|| b.string(text)),
            JavaString::Selection(list, text) => *self
                .selection_strings
                .entry((*list, text.clone()))
                .or_insert_with(|| b.string(text)),
        }
    }

    fn boolean(&mut self, b: &mut Builder, v: JavaBoolean) -> Value {
        match v {
            JavaBoolean::Null => Value::Null,
            JavaBoolean::Old(id, flag) => *self
                .old_booleans
                .entry(id)
                .or_insert_with(|| distinct_boolean(b, flag)),
            JavaBoolean::Canonical(flag) => b.boolean(flag),
        }
    }

    // Mirrors java.util.HashMap.writeObject (OpenJDK 25)
    fn map(&mut self, b: &mut Builder, map: Option<&BoolMap>) -> Value {
        let Some(map) = map else {
            return Value::Null;
        };
        let entries: Vec<(Value, Value)> = map
            .iter()
            .map(|e| (self.string(b, &e.key_payload), self.boolean(b, e.value)))
            .collect();
        let (buckets, threshold) = map.write_params();
        b.hash_map(&entries, buckets, threshold)
    }

    fn events(&mut self, b: &mut Builder, events: &[DocEvent]) -> Value {
        let events_class = wire::events_desc(b);
        let items = events
            .iter()
            .map(|e| {
                let class = wire::event_desc(b);
                // Wire order: addonNames, description, dlcNames, name, userconfigFolderNames.
                let addons = self.map(b, e.addon_names.as_ref());
                let description = self.string(b, &e.description);
                let dlcs = self.map(b, e.dlc_names.as_ref());
                let name = self.string(b, &e.name);
                let userconfig = self.map(b, e.userconfig_folder_names.as_ref());
                b.simple_object(class, vec![addons, description, dlcs, name, userconfig])
            })
            .collect();
        let list = b.array_list(items);
        b.simple_object(events_class, vec![list])
    }
}

/// Uploads `doc` as `.a3s/events` over the repository's upload protocol, on a new connection.
// Mirrors service/ConnectionService.java:453-462,
// ui/repository/dialogs/progress/ProgressUploadEventsDialog.java:29-77
pub async fn upload_events(
    doc: &EventsDoc,
    repository: &Repository,
    options: &TransportOptions,
    ctx: &TransferCtx,
) -> Result<(), EventsError> {
    let protocol = repository
        .upload_protocol
        .as_ref()
        .ok_or(EventsError::UploadProtocolNotSet)?;
    let bytes = doc.encode()?;
    let mut transport = RemoteConnection::connect(protocol, options, ctx)
        .await?
        .into_transport();
    let result = transport
        .upload_bytes(&bytes, &A3sFile::Events.relative_path(), ctx)
        .await
        .map_err(|source| EventsError::Transport { source });
    transport.close().await;
    result
}

/// Remote metadata of the admin panel (no autoconfig, events as an editable document).
#[derive(Clone, Debug, Default)]
pub struct AdminMetadata {
    /// `.a3s/sync`.
    pub sync: Option<Arc<SyncTreeDirectory>>,

    /// `.a3s/serverinfo`.
    pub server_info: Option<Arc<ServerInfo>>,

    /// `.a3s/changelogs`.
    pub changelogs: Option<Arc<Changelogs>>,

    /// `.a3s/events`.
    pub events: Option<EventsDoc>,
}

/// Fetches `sync`, `serverinfo`, `changelogs` and `events`, in this order, never `autoconfig`.
///
/// A fetched file replaces its field; the first failure sets its field to `None`, leaves the
/// later fields untouched and is returned.
// Mirrors ui/repository/dialogs/progress/ProgressConnectionAsAdminDialog.java:47-69
pub async fn fetch_admin_metadata(
    conn: &mut RemoteConnection,
    repository_name: &str,
    metadata: &mut AdminMetadata,
    ctx: &TransferCtx,
) -> Result<(), RemoteError> {
    let result = fetch_admin_files(conn, metadata, ctx).await;
    for (missing, name) in [
        (metadata.sync.is_none(), "Sync"),
        (metadata.server_info.is_none(), "ServerInfo"),
        (metadata.changelogs.is_none(), "Changelogs"),
        (metadata.events.is_none(), "Events"),
    ] {
        if missing {
            tracing::info!(
                repository = repository_name,
                "Remote file {name} not found."
            );
        }
    }
    result
}

async fn fetch_admin_files(
    conn: &mut RemoteConnection,
    metadata: &mut AdminMetadata,
    ctx: &TransferCtx,
) -> Result<(), RemoteError> {
    metadata.sync = None;
    metadata.sync = Some(Arc::new(
        conn.fetch_a3s::<SyncTreeDirectory>(A3sFile::Sync, ctx)
            .await?
            .0,
    ));
    metadata.server_info = None;
    metadata.server_info = Some(Arc::new(
        conn.fetch_a3s::<ServerInfo>(A3sFile::ServerInfo, ctx)
            .await?
            .0,
    ));
    metadata.changelogs = None;
    metadata.changelogs = Some(Arc::new(
        conn.fetch_a3s::<Changelogs>(A3sFile::Changelogs, ctx)
            .await?
            .0,
    ));
    metadata.events = None;
    let raw = conn.fetch_a3s::<RawGraph>(A3sFile::Events, ctx).await?.0;
    let doc = decode_events(&raw.graph).map_err(|e| {
        RemoteError::decode(A3sFile::Events, a3s_repo_files::Error::Serialization(e))
    })?;
    metadata.events = Some(doc);
    Ok(())
}
