use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use serde::{Deserialize, Serialize};

use crate::wire::{self, JavaCodec};
use crate::{FavoriteServer, Protocol, Result};

/// Repository bootstrap file (`.a3s/autoconfig`), mirrors
/// `fr.soe.a3s.domain.repository.AutoConfig`.
///
/// Clients import a repository from its autoconfig URL; it carries the download protocol
/// (including credentials) and favorite servers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoConfig {
    pub repository_name: Option<String>,
    pub protocol: Option<Protocol>,
    pub favorite_servers: Vec<FavoriteServer>,
}

impl JavaCodec for AutoConfig {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::AUTO_CONFIG)?;
        let protocol = match graph.field(id, "protocole")? {
            Value::Null => None,
            v => Some(Protocol::decode(graph, v)?),
        };
        Ok(Self {
            repository_name: graph.field_string(id, "repositoryName")?,
            protocol,
            favorite_servers: wire::object_list(graph, graph.field(id, "favoriteServers")?)?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::auto_config_desc(b);
        let servers = self.favorite_servers.iter().map(|s| s.encode(b)).collect();
        let servers = b.array_list(servers);
        let protocol = self.protocol.as_ref().map_or(Value::Null, |p| p.encode(b));
        let name = b.opt_string(self.repository_name.as_deref());
        b.simple_object(class, vec![servers, protocol, name])
    }
}
