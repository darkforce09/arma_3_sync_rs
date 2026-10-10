use std::collections::BTreeMap;

use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::wire::{self, JavaCodec};

/// Events / modsets (`.a3s/events`), mirrors `fr.soe.a3s.domain.repository.Events`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Events {
    pub list: Vec<Event>,
}

/// A named selection of addons, mirrors `fr.soe.a3s.domain.repository.Event`.
///
/// Map values are the "optional" flag of each entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub name: String,
    pub description: Option<String>,

    /// Addon directory name → optional.
    pub addon_names: BTreeMap<String, bool>,

    /// `GameDLCs` constant name (e.g. `Contact`, `GM`) → optional.
    ///
    /// Added by the vpzbrig21 fork; absent (and decoded as empty) in older files.
    pub dlc_names: BTreeMap<String, bool>,

    /// Folder name below `userconfig/` → optional.
    pub userconfig_folder_names: BTreeMap<String, bool>,
}

fn bool_map(graph: &Graph, value: Option<Value>) -> Result<BTreeMap<String, bool>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (k, v) in a3s_jserial::jdk::map(graph, value)?.unwrap_or_default() {
        if let Some(key) = graph.opt_string(k)? {
            let optional = a3s_jserial::jdk::boolean(graph, v)?.unwrap_or(false);
            out.insert(key.to_owned(), optional);
        }
    }
    Ok(out)
}

fn encode_bool_map(b: &mut Builder, map: &BTreeMap<String, bool>) -> Value {
    let entries: Vec<(String, Value)> = map
        .iter()
        .map(|(k, v)| (k.clone(), b.boolean(*v)))
        .collect();
    b.hash_map_str(&entries)
}

impl JavaCodec for Events {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::EVENTS)?;
        Ok(Self {
            list: wire::object_list(graph, graph.field(id, "list")?)?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::events_desc(b);
        let items = self.list.iter().map(|e| e.encode(b)).collect();
        let list = b.array_list(items);
        b.simple_object(class, vec![list])
    }
}

impl JavaCodec for Event {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::EVENT)?;
        Ok(Self {
            name: graph.field_string(id, "name")?.unwrap_or_default(),
            description: graph.field_string(id, "description")?,
            addon_names: bool_map(graph, graph.opt_field(id, "addonNames")?)?,
            dlc_names: bool_map(graph, graph.opt_field(id, "dlcNames")?)?,
            userconfig_folder_names: bool_map(
                graph,
                graph.opt_field(id, "userconfigFolderNames")?,
            )?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::event_desc(b);
        // Wire order: addonNames, description, dlcNames, name, userconfigFolderNames.
        let addons = encode_bool_map(b, &self.addon_names);
        let description = b.opt_string(self.description.as_deref());
        let dlcs = encode_bool_map(b, &self.dlc_names);
        let name = b.string(&self.name);
        let userconfig = encode_bool_map(b, &self.userconfig_folder_names);
        b.simple_object(class, vec![addons, description, dlcs, name, userconfig])
    }
}
