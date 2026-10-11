//! `.a3s/events`: modsets carried over from the previous file, filtered by the new addon and DLC
//! names (B34–B36, B47).

use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use a3s_types::constants::GameDlc;
use a3s_types::{JavaCodec, wire};

use super::carry::{Memo, OldBool, OldStr, RawGraph};
use super::java_hash::JavaHashMap;

/// A `HashMap<String, Boolean>` of the new file, keys and values taken from the old graph.
type BoolMap = JavaHashMap<OldStr, OldBool>;

/// An event of the new file.
#[derive(Clone, Debug, PartialEq)]
struct NewEvent {
    /// Same instance as the old event's name.
    name: OldStr,
    description: OldStr,
    addon_names: BoolMap,
    dlc_names: BoolMap,
    userconfig_folder_names: BoolMap,
}

/// The `Events` object written by a build. Write-only: its `decode` always fails; read the
/// written file as [`a3s_types::Events`].
#[derive(Clone, Debug, PartialEq)]
pub struct EventsEmit {
    events: Vec<NewEvent>,
}

/// Entries of an old map in the iteration order of the map Java deserializes from it, or no
/// entries for null. A null key or a key that is not a string is skipped.
fn old_map(graph: &Graph, value: Value) -> a3s_jserial::Result<BoolMap> {
    let entries = a3s_jserial::jdk::map(graph, value)?.unwrap_or_default();
    let mut resolved = Vec::with_capacity(entries.len());
    for (k, v) in entries {
        let key = OldStr::resolve(graph, k)?;
        let Some(text) = key.as_str().map(str::to_owned) else {
            continue;
        };
        resolved.push((text, key, OldBool::resolve(graph, v)?));
    }
    Ok(JavaHashMap::deserialized(resolved))
}

fn entries(map: &BoolMap) -> Vec<(String, OldStr, OldBool)> {
    map.iter()
        .map(|e| (e.key.clone(), e.key_payload.clone(), e.value))
        .collect()
}

/// Computes the new events (B34–B36). `addon_names` are the new sync's addon names.
// Mirrors dao/repository/RepositoryBuildProcessor.java:245-274
pub fn plan_events(
    old: Option<&RawGraph>,
    addon_names: &[String],
) -> a3s_jserial::Result<EventsEmit> {
    let Some(old) = old else {
        return Ok(EventsEmit { events: Vec::new() });
    };
    let graph = &old.graph;
    let root = old
        .root
        .as_ref()
        .ok_or(a3s_jserial::Error::UnexpectedNull)?;
    a3s_jserial::jdk::expect_class(graph, root, wire::EVENTS)?;
    let list = a3s_jserial::jdk::list(graph, graph.field(root, "list")?)?.unwrap_or_default();
    let mut events = Vec::with_capacity(list.len());
    for element in list {
        let id = element.as_ref().ok_or(a3s_jserial::Error::UnexpectedNull)?;
        a3s_jserial::jdk::expect_class(graph, id, wire::EVENT)?;
        let field = |name| -> a3s_jserial::Result<Value> {
            Ok(graph.opt_field(id, name)?.unwrap_or(Value::Null))
        };
        // `getDlcNames()`/`getUserconfigFolderNames()` turn null into an empty map.
        let old_addons = old_map(graph, field("addonNames")?)?;
        let old_dlcs = old_map(graph, field("dlcNames")?)?;
        let old_userconfig = old_map(graph, field("userconfigFolderNames")?)?;

        let mut merged: BoolMap = JavaHashMap::new();
        merged.put_all(entries(&old_addons));
        merged.put_all(entries(&old_dlcs));

        let mut addon_map = JavaHashMap::new();
        let mut dlc_map = JavaHashMap::new();
        for entry in merged.iter() {
            if addon_names.contains(&entry.key) {
                addon_map.put(entry.key.clone(), entry.key_payload.clone(), entry.value);
            } else if GameDlc::ALL.iter().any(|d| d.java_name() == entry.key) {
                dlc_map.put(entry.key.clone(), entry.key_payload.clone(), entry.value);
            }
        }
        let mut userconfig = JavaHashMap::new();
        userconfig.put_all(entries(&old_userconfig));

        events.push(NewEvent {
            name: OldStr::resolve(graph, field("name")?)?,
            description: OldStr::resolve(graph, field("description")?)?,
            addon_names: addon_map,
            dlc_names: dlc_map,
            userconfig_folder_names: userconfig,
        });
    }
    Ok(EventsEmit { events })
}

fn encode_map(b: &mut Builder, memo: &mut Memo, map: &BoolMap) -> Value {
    let entries: Vec<(Value, Value)> = map
        .iter()
        .map(|e| (memo.string(b, &e.key_payload), memo.boolean(b, e.value)))
        .collect();
    let (buckets, threshold) = map.write_params();
    b.hash_map(&entries, buckets, threshold)
}

impl JavaCodec for EventsEmit {
    fn decode(_graph: &Graph, _value: Value) -> a3s_types::Result<Self> {
        Err(a3s_jserial::Error::Unsupported("build output is write-only").into())
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let events_class = wire::events_desc(b);
        let mut memo = Memo::default();
        let items = self
            .events
            .iter()
            .map(|e| {
                let class = wire::event_desc(b);
                // Wire order: addonNames, description, dlcNames, name, userconfigFolderNames.
                let addons = encode_map(b, &mut memo, &e.addon_names);
                let description = memo.string(b, &e.description);
                let dlcs = encode_map(b, &mut memo, &e.dlc_names);
                let name = memo.string(b, &e.name);
                let userconfig = encode_map(b, &mut memo, &e.userconfig_folder_names);
                b.simple_object(class, vec![addons, description, dlcs, name, userconfig])
            })
            .collect();
        let list = b.array_list(items);
        b.simple_object(events_class, vec![list])
    }
}
