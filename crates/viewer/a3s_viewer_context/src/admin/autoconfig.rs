//! The favourite servers a repository built on this computer publishes in its auto-config.

use a3s_repo_files::services::{ClientData, ServiceError};
use a3s_types::FavoriteServer;

/// One of the client's favourite servers and whether the auto-config includes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoconfigServer {
    /// The favourite server.
    pub server: FavoriteServer,

    /// The next build writes it to the repository's `.a3s/autoconfig`.
    pub included: bool,
}

/// The client's favourite servers, in their order, each marked when `repository` publishes it
/// in its auto-config. Empty when the repository is unknown.
///
/// A stored entry matches a favourite server with the same name, address and port.
// Mirrors ui/repository/dialogs/BuildRepositoryOptionsDialog.java:410-430
pub fn autoconfig_servers(client: &ClientData, repository: &str) -> Vec<AutoconfigServer> {
    let Ok(stored) = client.repository(repository) else {
        return Vec::new();
    };
    client
        .favorite_servers()
        .iter()
        .map(|server| AutoconfigServer {
            included: stored
                .favorite_servers_set_to_autoconfig
                .iter()
                .any(|entry| same_server(entry, server)),
            server: server.clone(),
        })
        .collect()
}

/// Includes the favourite server at `index` in the auto-config of `repository`, or leaves it
/// out; the other servers keep their choice. Stored at once; the next publish uploads it.
// Mirrors ui/repository/dialogs/BuildRepositoryOptionsDialog.java:614-616
pub fn set_autoconfig_server(
    client: &mut ClientData,
    repository: &str,
    index: usize,
    included: bool,
) -> Result<(), ServiceError> {
    client.repository(repository)?;
    let servers = autoconfig_servers(client, repository);
    if index >= servers.len() {
        return Err(ServiceError::FavoriteServerIndex {
            index,
            len: servers.len(),
        });
    }
    let indices: Vec<usize> = servers
        .iter()
        .enumerate()
        .filter(|&(i, entry)| if i == index { included } else { entry.included })
        .map(|(i, _)| i)
        .collect();
    client.set_favorite_servers_to_autoconfig(repository, &indices)
}

fn same_server(a: &FavoriteServer, b: &FavoriteServer) -> bool {
    a.name == b.name && a.ip_address == b.ip_address && a.port == b.port
}
