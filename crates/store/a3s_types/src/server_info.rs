use std::collections::BTreeSet;

use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use serde::{Deserialize, Serialize};

use crate::wire::{self, JavaCodec};
use crate::{JavaDate, Result};

/// Repository summary (`.a3s/serverinfo`), mirrors `fr.soe.a3s.domain.repository.ServerInfo`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Incremented by each build that changed content.
    pub revision: i32,
    pub build_date: Option<JavaDate>,
    pub number_of_files: i64,

    /// Total size of all files below the repository root except `.a3s`
    /// (including `.zsync` and `.zip` files).
    pub total_files_size: i64,

    /// `/`-joined relative paths of folders that clients must not touch.
    pub hidden_folder_paths: BTreeSet<String>,

    /// Maximum number of parallel client connections the server allows.
    pub number_of_connections: i32,
    pub no_partial_file_transfer: bool,
    pub repository_content_updated: bool,
    pub compressed_pbo_files_only: bool,
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self {
            revision: 0,
            build_date: None,
            number_of_files: 0,
            total_files_size: 0,
            hidden_folder_paths: BTreeSet::new(),
            number_of_connections: 1,
            no_partial_file_transfer: false,
            repository_content_updated: false,
            compressed_pbo_files_only: false,
        }
    }
}

impl JavaCodec for ServerInfo {
    fn decode(graph: &Graph, value: Value) -> Result<Self> {
        let id = wire::expect_object(graph, value, wire::SERVER_INFO)?;
        let hidden = a3s_jserial::jdk::set(graph, graph.field(id, "hiddenFolderPaths")?)?
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| graph.opt_string(v).transpose())
            .map(|s| s.map(str::to_owned))
            .collect::<a3s_jserial::Result<_>>()?;
        Ok(Self {
            revision: graph.field_i32(id, "revision")?,
            build_date: a3s_jserial::jdk::date(graph, graph.field(id, "buildDate")?)?,
            number_of_files: graph.field_i64(id, "numberOfFiles")?,
            total_files_size: graph.field_i64(id, "totalFilesSize")?,
            hidden_folder_paths: hidden,
            number_of_connections: graph.field_i32(id, "numberOfConnections")?,
            no_partial_file_transfer: graph.field_bool(id, "noPartialFileTransfer")?,
            repository_content_updated: graph.field_bool(id, "repositoryContentUpdated")?,
            compressed_pbo_files_only: graph.field_bool(id, "compressedPboFilesOnly")?,
        })
    }

    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::server_info_desc(b);
        let build_date = self.build_date.map_or(Value::Null, |d| b.date(d));
        let hidden: Vec<String> = self.hidden_folder_paths.iter().cloned().collect();
        let hidden = b.hash_set_str(&hidden);
        b.simple_object(
            class,
            vec![
                Value::Boolean(self.compressed_pbo_files_only),
                Value::Boolean(self.no_partial_file_transfer),
                Value::Int(self.number_of_connections),
                Value::Long(self.number_of_files),
                Value::Boolean(self.repository_content_updated),
                Value::Int(self.revision),
                Value::Long(self.total_files_size),
                build_date,
                hidden,
            ],
        )
    }
}
