use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use serde::{Deserialize, Serialize};

use crate::wire::{self, JavaCodec};
use crate::{JavaDate, Result};

/// Build history (`.a3s/changelogs`), mirrors `fr.soe.a3s.domain.repository.Changelogs`.
///
/// The Java build keeps at most [`Changelogs::MAX_ENTRIES`] entries, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changelogs {
    pub list: Vec<Changelog>,
}

impl Changelogs {
    pub const MAX_ENTRIES: usize = 10;
}

/// One build, mirrors `fr.soe.a3s.domain.repository.Changelog`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changelog {
    pub revision: i32,
    pub build_date: Option<JavaDate>,
    pub content_updated: bool,
    pub new_addons: Vec<String>,
    pub updated_addons: Vec<String>,
    pub deleted_addons: Vec<String>,

    /// Names of all addons in the repository at this revision.
    pub addons: Vec<String>,
}

impl JavaCodec for Changelogs {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::CHANGELOGS)?;
        Ok(Self {
            list: wire::object_list(graph, graph.field(id, "list")?)?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::changelogs_desc(b);
        let items = self.list.iter().map(|c| c.encode(b)).collect();
        let list = b.array_list(items);
        b.simple_object(class, vec![list])
    }
}

impl JavaCodec for Changelog {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::CHANGELOG)?;
        Ok(Self {
            revision: graph.field_i32(id, "revision")?,
            build_date: a3s_jserial::jdk::date(graph, graph.field(id, "buildDate")?)?,
            content_updated: graph.field_bool(id, "contentUpdated")?,
            new_addons: wire::string_list(graph, graph.field(id, "newAddons")?)?,
            updated_addons: wire::string_list(graph, graph.field(id, "updatedAddons")?)?,
            deleted_addons: wire::string_list(graph, graph.field(id, "deletedAddons")?)?,
            addons: wire::string_list(graph, graph.field(id, "addons")?)?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::changelog_desc(b);
        // Object fields in wire order: addons, buildDate, deletedAddons, newAddons, updatedAddons.
        //
        // In Java the new/updated/deleted entries are the same `String` instances as the matching
        // `addons` entries (observed in real repositories), so they are written as references.
        let mut shared = std::collections::HashMap::new();
        let addon_values: Vec<Value> = self
            .addons
            .iter()
            .map(|name| {
                *shared
                    .entry(name.as_str())
                    .or_insert_with(|| b.string(name))
            })
            .collect();
        let addons = b.array_list(addon_values);
        let build_date = self.build_date.map_or(Value::Null, |d| b.date(d));
        let reuse = |b: &mut Builder, names: &[String]| {
            let values = names
                .iter()
                .map(|name| {
                    shared
                        .get(name.as_str())
                        .copied()
                        .unwrap_or_else(|| b.string(name))
                })
                .collect();
            b.array_list(values)
        };
        let deleted = reuse(b, &self.deleted_addons);
        let new = reuse(b, &self.new_addons);
        let updated = reuse(b, &self.updated_addons);
        b.simple_object(
            class,
            vec![
                Value::Boolean(self.content_updated),
                Value::Int(self.revision),
                addons,
                build_date,
                deleted,
                new,
                updated,
            ],
        )
    }
}
