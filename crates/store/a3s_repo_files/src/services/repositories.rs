//! Repository CRUD and settings (`service/RepositoryService.java`).

use a3s_types::Protocol;
use a3s_types::java_string::java_trim;
use a3s_types::local::Repository;
use a3s_types::local::protocol::{check_protocol, host_url, new_protocol};

use super::{ClientData, ProtocolInput, Result, ServiceError};
use crate::local::CacheKind;

/// Builds and checks the protocol of a connection dialog; `unsupported` is the text for a type
/// without protocol class (A3S).
fn build_protocol(input: &ProtocolInput, unsupported: &'static str) -> Result<Protocol> {
    let protocol = new_protocol(
        input.protocol_type,
        &input.url,
        &input.port,
        &input.login,
        &input.password,
        input.validate_ssl_certificate,
    )
    .ok_or(ServiceError::UnsupportedProtocol(unsupported))?;
    check_protocol(&protocol)?;
    Ok(protocol)
}

impl ClientData {
    /// The repositories, in name order.
    pub fn repositories(&self) -> impl Iterator<Item = &Repository> {
        self.repositories.values()
    }

    /// The repository names, in order.
    pub fn repository_names(&self) -> Vec<String> {
        self.repositories.keys().cloned().collect()
    }

    // Mirrors service/RepositoryService.java:161-170
    pub fn repository(&self, name: &str) -> Result<&Repository> {
        self.repositories
            .get(name)
            .ok_or_else(|| ServiceError::RepositoryNotFound(name.to_owned()))
    }

    /// `scheme://host:port/path` of the download protocol.
    // Mirrors service/RepositoryService.java:192-201
    pub fn repository_display_url(&self, name: &str) -> Result<String> {
        Ok(host_url(&self.repository(name)?.protocol))
    }

    /// Whether `name` is in use: a loaded repository, an unreadable repository file naming it,
    /// or a file at its path.
    pub(super) fn repository_name_taken(&self, name: &str) -> bool {
        self.repositories.contains_key(name)
            || self.unreadable_repositories.contains(name)
            || self.store.repository_path(name).exists()
    }

    /// Creates and writes a repository with default settings. Follow-ups such as the default
    /// download location are the caller's.
    // Mirrors service/RepositoryService.java:80-97
    pub fn create_repository(&mut self, name: &str, input: &ProtocolInput) -> Result<()> {
        // A blank name would make a file the store refuses to load.
        if java_trim(name).is_empty() {
            return Err(ServiceError::EmptyRepositoryName);
        }
        if self.repository_name_taken(name) {
            return Err(ServiceError::RepositoryExists(name.to_owned()));
        }
        let protocol = build_protocol(input, "Protocol not supported yet.")?;
        self.commit_repository(Repository::new(name, protocol))
    }

    /// Replaces the download protocol. The new protocol has the default timeouts, so custom
    /// timeouts are reset, as in Java.
    // Mirrors service/RepositoryService.java:99-116
    pub fn set_repository_protocol(&mut self, name: &str, input: &ProtocolInput) -> Result<()> {
        if name.is_empty() {
            return Err(ServiceError::EmptyRepositoryName);
        }
        let protocol = build_protocol(input, "Protocol is not supported yet.")?;
        self.set_repository(name, |r| r.protocol = protocol)
    }

    /// Renames a repository: writes it under `new`, deletes the old file and moves both SHA-1
    /// caches. Favorite servers and profile groups keep naming the old repository, as in Java.
    // Mirrors service/RepositoryService.java:118-131
    pub fn rename_repository(&mut self, old: &str, new: &str) -> Result<()> {
        if java_trim(new).is_empty() {
            return Err(ServiceError::EmptyRepositoryName);
        }
        let repository = self.repository(old)?;
        if new == old {
            return Ok(());
        }
        if self.repository_name_taken(new) {
            return Err(ServiceError::RepositoryExists(new.to_owned()));
        }
        let mut renamed = repository.clone();
        new.clone_into(&mut renamed.name);
        let previous = self
            .repository_sources
            .get(old)
            .cloned()
            .unwrap_or_else(|| self.store.repository_path(old));
        self.store
            .write_repository_replacing(&renamed, Some(&previous))?;
        // Java keeps the caches inside the repository object; here they are files to move. A
        // cache can always be rebuilt, so a failed move is only logged.
        for kind in [CacheKind::Sync, CacheKind::Build] {
            let from = self.store.sha1_cache_path(old, kind);
            let to = self.store.sha1_cache_path(new, kind);
            match std::fs::rename(&from, &to) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(
                    "Failed to move the SHA-1 cache {} to {}: {err}",
                    from.display(),
                    to.display()
                ),
            }
        }
        self.repositories.remove(old);
        self.repository_sources.remove(old);
        self.repository_sources
            .insert(new.to_owned(), self.store.repository_path(new));
        self.repositories.insert(new.to_owned(), renamed);
        Ok(())
    }

    /// Removes a repository from memory and deletes its file and SHA-1 caches; whether a file
    /// was deleted (a repository that was never written has none).
    // Mirrors service/RepositoryService.java:133-140
    pub fn remove_repository(&mut self, name: &str) -> Result<bool> {
        self.repository(name)?;
        let mut deleted = self.store.remove_repository(name)?;
        if let Some(source) = self.repository_sources.get(name)
            && *source != self.store.repository_path(name)
        {
            deleted |= self.store.delete_repository_file(source)?;
        }
        if !deleted {
            // Mirrors dao/repository/RepositoryDAO.java:64-65
            tracing::warn!(
                "Warning: Unable to locate repository file for \"{name}\" during removal."
            );
        }
        self.repositories.remove(name);
        self.repository_sources.remove(name);
        Ok(deleted)
    }

    /// Runs `f` on a copy of the repository and, when it returns `Ok(true)` ("changed"), writes
    /// and keeps it. `Ok(false)` writes nothing; an error or a changed name leaves everything
    /// as it was.
    pub fn update_repository(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Repository) -> Result<bool>,
    ) -> Result<bool> {
        let mut next = self.repository(name)?.clone();
        if !f(&mut next)? {
            return Ok(false);
        }
        if next.name != name {
            return Err(ServiceError::NameChange);
        }
        self.commit_repository(next)?;
        Ok(true)
    }

    /// Runs `f` on the repository and always writes it.
    fn set_repository(&mut self, name: &str, f: impl FnOnce(&mut Repository)) -> Result<()> {
        self.update_repository(name, |r| {
            f(r);
            Ok(true)
        })
        .map(drop)
    }

    /// The main folder of an administered repository.
    // Mirrors service/RepositoryService.java:172-180
    pub fn set_repository_path(&mut self, name: &str, path: Option<String>) -> Result<()> {
        self.set_repository(name, |r| r.path = path)
    }

    /// Hides a local folder from the extra-content check.
    // Mirrors service/RepositoryService.java:661-667
    pub fn add_hidden_folder(&mut self, name: &str, path: String) -> Result<()> {
        self.set_repository(name, |r| {
            r.hidden_folder_paths.insert(path);
        })
    }

    /// Removes `path`, or else `path` with every `/` replaced by `\`; whether one was removed
    /// (nothing is written otherwise).
    // Mirrors service/RepositoryService.java:591-605, 669-679
    pub fn remove_hidden_folder(&mut self, name: &str, path: &str) -> Result<bool> {
        self.update_repository(name, |r| {
            Ok(r.hidden_folder_paths.remove(path)
                || r.hidden_folder_paths.remove(&path.replace('/', "\\")))
        })
    }

    /// With `event`, the download location of that event (`None` removes it); without, the
    /// repository's default download location.
    // Mirrors service/RepositoryService.java:729-744
    pub fn set_default_download_location(
        &mut self,
        name: &str,
        event: Option<&str>,
        location: Option<String>,
    ) -> Result<()> {
        self.set_repository(name, |r| match (event, location) {
            (Some(event), Some(location)) => {
                r.events_download_location
                    .insert(event.to_owned(), location);
            }
            (Some(event), None) => {
                r.events_download_location.remove(event);
            }
            (None, location) => r.default_download_location = location,
        })
    }

    // Mirrors service/RepositoryService.java:770-780
    pub fn set_notify(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.notify = value)
    }

    /// Automatic synchronization (`auto`).
    // Mirrors service/RepositoryService.java:782-792
    pub fn set_auto_update(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.auto = value)
    }

    /// Stores `no_auto_discover` as given. Java's `setAutoDiscover(v)` stores `v` into
    /// `noAutoDiscover` and `isAutoDiscover` returns it back, so the field name is the truth.
    // Mirrors service/RepositoryService.java:812-831
    pub fn set_no_auto_discover(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.no_auto_discover = value)
    }

    // Mirrors service/RepositoryService.java:842-852
    pub fn set_exact_match(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.exact_match = value)
    }

    // Mirrors service/RepositoryService.java:1188-1194
    pub fn add_excluded_file_from_build(&mut self, name: &str, path: String) -> Result<()> {
        self.set_repository(name, |r| {
            r.excluded_files_from_build.insert(path);
        })
    }

    // Mirrors service/RepositoryService.java:1206-1213
    pub fn set_excluded_files_from_build(&mut self, name: &str, paths: Vec<String>) -> Result<()> {
        self.set_repository(name, |r| {
            r.excluded_files_from_build = paths.into_iter().collect();
        })
    }

    // Mirrors service/RepositoryService.java:1215-1221
    pub fn clear_excluded_files_from_build(&mut self, name: &str) -> Result<()> {
        self.set_repository(name, |r| r.excluded_files_from_build.clear())
    }

    // Mirrors service/RepositoryService.java:1223-1229
    pub fn add_excluded_folder_from_sync(&mut self, name: &str, path: String) -> Result<()> {
        self.set_repository(name, |r| {
            r.excluded_folders_from_sync.insert(path);
        })
    }

    // Mirrors service/RepositoryService.java:1249-1256
    pub fn set_excluded_folders_from_sync(&mut self, name: &str, paths: Vec<String>) -> Result<()> {
        self.set_repository(name, |r| {
            r.excluded_folders_from_sync = paths.into_iter().collect();
        })
    }

    // Mirrors service/RepositoryService.java:1231-1237
    pub fn clear_excluded_folders_from_sync(&mut self, name: &str) -> Result<()> {
        self.set_repository(name, |r| r.excluded_folders_from_sync.clear())
    }

    /// Build setting written raw into `.a3s/serverinfo`.
    // Mirrors service/RepositoryService.java:1295-1301
    pub fn set_number_of_connections(&mut self, name: &str, value: i32) -> Result<()> {
        self.set_repository(name, |r| r.number_of_connections = value)
    }

    // Mirrors service/RepositoryService.java:1312-1318
    pub fn set_compressed(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.compressed = value)
    }

    // Mirrors service/RepositoryService.java:1329-1335
    pub fn set_upload_compressed_pbo_files_only(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.upload_compressed_pbo_files_only = value)
    }

    // Mirrors service/RepositoryService.java:1346-1352
    pub fn set_use_partial_file_transfer(&mut self, name: &str, value: bool) -> Result<()> {
        self.set_repository(name, |r| r.set_use_partial_file_transfer(value))
    }

    /// Client download connections, raw (`0` means unset).
    // Mirrors service/RepositoryService.java:1562-1568
    pub fn set_number_of_client_connections(&mut self, name: &str, value: i32) -> Result<()> {
        self.set_repository(name, |r| r.number_of_client_connections = value)
    }

    /// Stores `min(value, 10)`; a value below 1 is rejected.
    // Mirrors service/RepositoryService.java:1578-1583
    pub fn set_parallel_upload_connections(&mut self, name: &str, value: i32) -> Result<()> {
        self.update_repository(name, |r| {
            r.set_parallel_upload_connections(value)?;
            Ok(true)
        })
        .map(drop)
    }

    /// Stores `min(value, 4)`; a value below 1 is rejected.
    // Mirrors service/RepositoryService.java:1594-1600
    pub fn set_repository_check_connections(&mut self, name: &str, value: i32) -> Result<()> {
        self.update_repository(name, |r| {
            r.set_repository_check_connections(value)?;
            Ok(true)
        })
        .map(drop)
    }

    /// Download speed limit in bytes per second, raw (`0` is unlimited).
    // Mirrors service/RepositoryService.java:1611-1617
    pub fn set_maximum_client_download_speed(&mut self, name: &str, value: f64) -> Result<()> {
        self.set_repository(name, |r| r.maximum_client_download_speed = value)
    }

    /// Connection timeout of the download protocol, stored as given.
    // Mirrors service/RepositoryService.java:1619-1625
    pub fn set_connection_timeout(&mut self, name: &str, value: Option<String>) -> Result<()> {
        self.set_repository(name, |r| r.protocol.connection_timeout = value)
    }

    /// Read timeout of the download protocol, stored as given.
    // Mirrors service/RepositoryService.java:1627-1633
    pub fn set_read_timeout(&mut self, name: &str, value: Option<String>) -> Result<()> {
        self.set_repository(name, |r| r.protocol.read_timeout = value)
    }

    /// Sets the upload protocol of an administered repository, with the given timeouts.
    // Mirrors service/RepositoryService.java:1372-1392
    pub fn set_upload_protocol(
        &mut self,
        name: &str,
        input: &ProtocolInput,
        connection_timeout: Option<String>,
        read_timeout: Option<String>,
    ) -> Result<()> {
        self.repository(name)?;
        let mut protocol = new_protocol(
            input.protocol_type,
            &input.url,
            &input.port,
            &input.login,
            &input.password,
            input.validate_ssl_certificate,
        )
        .ok_or(ServiceError::UnsupportedProtocol(
            "Upload protocol not supported.",
        ))?;
        protocol.connection_timeout = connection_timeout;
        protocol.read_timeout = read_timeout;
        check_protocol(&protocol)?;
        self.set_repository(name, |r| r.upload_protocol = Some(protocol))
    }

    // Mirrors service/RepositoryService.java:142-148
    pub fn reset_upload_protocol(&mut self, name: &str) -> Result<()> {
        self.set_repository(name, |r| r.upload_protocol = None)
    }

    /// Selects the favorite servers (indices into [`Self::favorite_servers`], in the given
    /// order) the next build writes to `.a3s/autoconfig`. Each selected configuration entry is
    /// tagged with the repository name; the repository stores copies. Writes the configuration,
    /// then the repository.
    // Mirrors service/RepositoryService.java:1272-1283
    pub fn set_favorite_servers_to_autoconfig(
        &mut self,
        name: &str,
        indices: &[usize],
    ) -> Result<()> {
        self.repository(name)?;
        let len = self.configuration.favorite_servers.len();
        if let Some(&index) = indices.iter().find(|&&i| i >= len) {
            return Err(ServiceError::FavoriteServerIndex { index, len });
        }
        let mut configuration = self.configuration.clone();
        let mut selected = Vec::with_capacity(indices.len());
        for &index in indices {
            let server = &mut configuration.favorite_servers[index];
            server.repository_name = Some(name.to_owned());
            selected.push(server.clone());
        }
        self.commit_configuration(configuration)?;
        self.set_repository(name, |r| r.favorite_servers_set_to_autoconfig = selected)
    }
}
