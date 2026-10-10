use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::wire::{self, JavaCodec};

/// A game server entry, mirrors `fr.soe.a3s.domain.configration.FavoriteServer`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteServer {
    pub name: Option<String>,
    pub ip_address: Option<String>,
    pub port: i32,
    pub password: Option<String>,
    pub selected: bool,

    /// Event (modset) to load when joining.
    pub modset_name: Option<String>,
    pub repository_name: Option<String>,
}

impl JavaCodec for FavoriteServer {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::FAVORITE_SERVER)?;
        Ok(Self {
            name: graph.field_string(id, "name")?,
            ip_address: graph.field_string(id, "ipAddress")?,
            port: graph.field_i32(id, "port")?,
            password: graph.field_string(id, "password")?,
            selected: graph.field_bool(id, "selected")?,
            modset_name: graph.field_string(id, "modsetName")?,
            repository_name: graph.field_string(id, "repositoryName")?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::favorite_server_desc(b);
        let ip = b.opt_string(self.ip_address.as_deref());
        let modset = b.opt_string(self.modset_name.as_deref());
        let name = b.opt_string(self.name.as_deref());
        let password = b.opt_string(self.password.as_deref());
        let repository = b.opt_string(self.repository_name.as_deref());
        b.simple_object(
            class,
            vec![
                Value::Int(self.port),
                Value::Boolean(self.selected),
                ip,
                modset,
                name,
                password,
                repository,
            ],
        )
    }
}
