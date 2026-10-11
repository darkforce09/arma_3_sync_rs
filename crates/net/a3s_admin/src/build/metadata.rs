//! Content fingerprint, revision, `ServerInfo` hidden folders and `AutoConfig` (B22–B27).

use std::collections::HashMap;

use a3s_jserial::jdk::Builder;
use a3s_jserial::{Graph, Value};
use a3s_types::{FavoriteServer, JavaCodec, Protocol, SyncTreeDirectory, SyncTreeNode, wire};

/// Map from lowercase `/`-joined relative path to `"<sha1>:<size>"` over the deep leaf list; a
/// later path that differs only in case overwrites an earlier one.
// Mirrors dao/repository/RepositoryBuildProcessor.java:384-392
pub(crate) fn content_fingerprint(sync: &SyncTreeDirectory) -> HashMap<String, String> {
    let mut map = HashMap::new();
    sync.walk(&mut |path, node| {
        if let SyncTreeNode::Leaf(leaf) = node {
            let key = path.replace('\\', "/").to_lowercase();
            let value = format!("{}:{}", leaf.sha1.as_deref().unwrap_or(""), leaf.size);
            map.insert(key, value);
        }
    });
    map
}

/// Whether the repository content changed since the previous build (no previous sync: yes).
// Mirrors dao/repository/RepositoryBuildProcessor.java:376-382
pub(crate) fn content_updated(old: Option<&SyncTreeDirectory>, new: &SyncTreeDirectory) -> bool {
    old.is_none_or(|old| content_fingerprint(old) != content_fingerprint(new))
}

/// 1 without a previous `serverinfo`, otherwise the previous revision, plus one if the content
/// changed.
// Mirrors dao/repository/RepositoryBuildProcessor.java:150-151
pub(crate) fn revision(old_revision: Option<i32>, content_updated: bool) -> i32 {
    match old_revision {
        None => 1,
        Some(r) => r.wrapping_add(i32::from(content_updated)),
    }
}

/// The hidden folder path of an excluded folder, relative to the repository folder, or `None`
/// where Java's `substring` would throw (B25).
///
/// `repository_path` is the raw repository path. Java finds the first occurrence of
/// `<separator><main folder name>` in the lowercased entry and keeps what follows it plus one
/// separator, so a prefix match elsewhere in the path is taken as is. Indices are UTF-16 units.
// Mirrors dao/repository/RepositoryBuildProcessor.java:160-170
pub fn hidden_folder_path(
    repository_path: &str,
    excluded_folder: &str,
    separator: char,
) -> Option<String> {
    let path: Vec<u16> = repository_path.encode_utf16().collect();
    let sep = separator.to_string();
    let sep16: Vec<u16> = sep.encode_utf16().collect();
    let main_start = last_index_of(&path, &sep16).map_or(0, |i| i + sep16.len());
    let main = String::from_utf16_lossy(&path[main_start..]);
    let main_len = main.encode_utf16().count() as i64;

    let lower: Vec<u16> = excluded_folder.to_lowercase().encode_utf16().collect();
    let needle: Vec<u16> = format!("{sep}{}", main.to_lowercase())
        .encode_utf16()
        .collect();
    let index = index_of(&lower, &needle).map_or(-1, |i| i as i64);

    let entry: Vec<u16> = excluded_folder.encode_utf16().collect();
    let start = usize::try_from(index + main_len + 2).ok()?;
    let folder = entry.get(start..)?;
    Some(String::from_utf16_lossy(folder).replace('\\', "/"))
}

fn index_of(haystack: &[u16], needle: &[u16]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn last_index_of(haystack: &[u16], needle: &[u16]) -> Option<usize> {
    if needle.is_empty() {
        return Some(haystack.len());
    }
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// The `AutoConfig` written by a build. Write-only: its `decode` always fails; read the written
/// file as [`a3s_types::AutoConfig`].
///
/// Java sets every autoconfig favorite's `repositoryName` to the repository's own name `String`
/// (`RepositoryService.setFavoriteServerToAutoconfig`), so the favorites and the autoconfig share
/// one instance (R5); favorites with another name keep their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoConfigEmit {
    /// `repositoryName`.
    pub repository_name: String,

    /// `protocole`.
    pub protocol: Protocol,

    /// `favoriteServers`, in order.
    pub favorite_servers: Vec<FavoriteServer>,
}

impl JavaCodec for AutoConfigEmit {
    fn decode(_graph: &Graph, _value: Value) -> a3s_types::Result<Self> {
        Err(a3s_jserial::Error::Unsupported("build output is write-only").into())
    }

    // Mirrors dao/repository/RepositoryBuildProcessor.java:172-176
    fn encode(&self, b: &mut Builder) -> Value {
        let class = wire::auto_config_desc(b);
        let name = b.string(&self.repository_name);
        let servers = self
            .favorite_servers
            .iter()
            .map(|s| {
                let class = wire::favorite_server_desc(b);
                let ip = b.opt_string(s.ip_address.as_deref());
                let modset = b.opt_string(s.modset_name.as_deref());
                let server_name = b.opt_string(s.name.as_deref());
                let password = b.opt_string(s.password.as_deref());
                let repository = match s.repository_name.as_deref() {
                    Some(n) if n == self.repository_name => name,
                    other => b.opt_string(other),
                };
                b.simple_object(
                    class,
                    vec![
                        Value::Int(s.port),
                        Value::Boolean(s.selected),
                        ip,
                        modset,
                        server_name,
                        password,
                        repository,
                    ],
                )
            })
            .collect();
        let servers = b.array_list(servers);
        let protocol = self.protocol.encode(b);
        b.simple_object(class, vec![servers, protocol, name])
    }
}

#[cfg(test)]
mod tests {
    use super::{hidden_folder_path, revision};

    #[test]
    fn revision_rule() {
        assert_eq!(revision(None, true), 1);
        assert_eq!(revision(None, false), 1);
        assert_eq!(revision(Some(4), false), 4);
        assert_eq!(revision(Some(4), true), 5);
    }

    #[test]
    fn hidden_folder_cases() {
        assert_eq!(
            hidden_folder_path("/srv/repo", "/srv/repo/docs/hidden", '/').as_deref(),
            Some("docs/hidden")
        );
        // First-occurrence prefix quirk.
        assert_eq!(
            hidden_folder_path("/srv/repo2/repo", "/srv/repo2/repo/@x", '/').as_deref(),
            Some("/repo/@x")
        );
        // Backslashes become slashes.
        assert_eq!(
            hidden_folder_path("C:\\a\\Repo", "C:\\a\\Repo\\x\\y", '\\').as_deref(),
            Some("x/y")
        );
        // Not found: index -1 keeps everything after main.len() + 1 units.
        assert_eq!(
            hidden_folder_path("/r/abc", "/zzzzz/q", '/').as_deref(),
            Some("zz/q")
        );
        // Out of range.
        assert_eq!(hidden_folder_path("/r/abcdefgh", "/x", '/'), None);
    }
}
