//! `export.a3s.autoconfig` export and import (`service/CommonService.java:54-109`).

use std::path::{Path, PathBuf};

use a3s_types::java_string::java_trim;
use a3s_types::local::{AutoConfigExport, Repository};

use super::{AutoConfigImportSummary, ClientData, Result, ServiceError};
use crate::autoconfig_export::{export_auto_config, import_auto_config};

impl ClientData {
    /// Writes `destination_folder/export.a3s.autoconfig` with the named profiles, the favorite
    /// servers at `favorite_server_indices` (into [`Self::favorite_servers`]) and the named
    /// repositories, each in the given order; unknown names are skipped. The file carries no
    /// launcher options and only each repository's name and protocol.
    ///
    /// Java clears the launch settings of the live profiles it exports; here they are kept.
    // Mirrors service/CommonService.java:54-81
    pub fn export_autoconfig(
        &self,
        profile_names: &[String],
        favorite_server_indices: &[usize],
        repository_names: &[String],
        destination_folder: &Path,
    ) -> Result<PathBuf> {
        let servers = self.favorite_servers();
        let favorite_servers = favorite_server_indices
            .iter()
            .map(|&index| {
                servers
                    .get(index)
                    .cloned()
                    .ok_or(ServiceError::FavoriteServerIndex {
                        index,
                        len: servers.len(),
                    })
            })
            .collect::<Result<_>>()?;
        let export = AutoConfigExport {
            profiles: profile_names
                .iter()
                .filter_map(|name| self.profiles.get(name).cloned())
                .collect(),
            favorite_servers,
            repositories: repository_names
                .iter()
                .filter_map(|name| self.repositories.get(name))
                .map(|r| Repository::new(r.name.clone(), r.protocol.clone()))
                .collect(),
        };
        export_auto_config(&export, destination_folder).map_err(ServiceError::AutoConfigWrite)
    }

    /// Merges an export file: its profiles are written, replacing a profile of the same name
    /// (whose launch settings become the defaults); its favorite servers are appended (no
    /// de-duplication) and the configuration written; its repositories are added unless the
    /// name exists, in which case the existing repository is kept untouched.
    ///
    /// Writes happen in that order and stop at the first failure. Java instead writes each
    /// imported repository over an existing one's file, losing its settings on disk.
    // Mirrors service/CommonService.java:83-109
    pub fn import_autoconfig(&mut self, file: &Path) -> Result<AutoConfigImportSummary> {
        let imported = import_auto_config(file).map_err(ServiceError::AutoConfigRead)?;
        let mut summary = AutoConfigImportSummary {
            warnings: imported.warnings,
            ..AutoConfigImportSummary::default()
        };
        let AutoConfigExport {
            profiles,
            favorite_servers,
            repositories,
        } = imported.value;

        for profile in profiles {
            let name = profile.name.clone();
            if java_trim(&name).is_empty() {
                summary
                    .warnings
                    .push("Skipped a profile without a name.".to_owned());
                continue;
            }
            let replaced = self.profiles.contains_key(&name);
            if !replaced && self.profile_name_taken(&name) {
                summary.warnings.push(format!(
                    "Skipped profile {name}: a profile file with this name cannot be read."
                ));
                continue;
            }
            self.commit_profile(profile)?;
            if replaced {
                summary.profiles_replaced.push(name.clone());
            }
            summary.profiles_imported.push(name);
        }

        if !favorite_servers.is_empty() {
            summary.favorite_servers_added = favorite_servers.len();
            let mut configuration = self.configuration.clone();
            configuration.favorite_servers.extend(favorite_servers);
            self.commit_configuration(configuration)?;
        }

        for repository in repositories {
            let name = repository.name.clone();
            if java_trim(&name).is_empty() {
                // Mirrors dao/repository/RepositoryDAO.java:121-124
                summary
                    .warnings
                    .push("Repository name is missing.".to_owned());
                continue;
            }
            if self.repository_name_taken(&name) {
                summary.repositories_skipped.push(name);
                continue;
            }
            self.commit_repository(repository)?;
            summary.repositories_added.push(name);
        }
        Ok(summary)
    }
}
