//! Repositories (`dao/repository/RepositoryDAO.java`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use a3s_types::local::Repository;

use super::profiles::{ignored_warning, replaced_warning, summary};
use super::toml_file::{self, file_name, modified_millis};
use super::{CacheKind, Error, LocalStore, TOML_EXTENSION};

/// A repository and the file it was loaded from.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedRepository {
    pub repository: Repository,
    pub source: PathBuf,
}

/// Result of [`LocalStore::load_repositories`].
///
/// Successfully loaded repositories are kept even when other files failed, as in Java.
#[derive(Debug, Default)]
pub struct RepositoriesLoad {
    /// Repositories by name.
    pub repositories: BTreeMap<String, LoadedRepository>,

    /// Files that could not be loaded, by file name.
    pub failures: BTreeMap<String, Error>,

    /// One line per duplicate repository name.
    pub duplicate_warnings: Vec<String>,
}

impl RepositoriesLoad {
    /// Java's `LoadingException` text, `None` without failures.
    // Mirrors dao/repository/RepositoryDAO.java:152-164
    pub fn error_message(&self) -> Option<String> {
        summary("Failed to read repository files:", &self.failures)
    }

    /// Adds `repository` loaded from `source`, applying Java's duplicate rule: a later file
    /// replaces the kept one only if it is strictly newer.
    // Mirrors dao/repository/RepositoryDAO.java:169-190
    pub(crate) fn register(&mut self, repository: Repository, source: PathBuf) {
        let name = repository.name.clone();
        let new_file = file_name(&source);
        if let Some(kept) = self.repositories.get(&name) {
            let kept_file = file_name(&kept.source);
            let kept_mtime = crate::hash::modified_millis(&kept.source).unwrap_or(i64::MIN);
            if modified_millis(&source) > kept_mtime {
                self.duplicate_warnings
                    .push(replaced_warning(&name, &kept_file, &new_file));
            } else {
                self.duplicate_warnings
                    .push(ignored_warning(&name, &new_file, &kept_file));
                return;
            }
        }
        self.repositories
            .insert(name, LoadedRepository { repository, source });
    }

    /// Logs the duplicate warnings like Java.
    // Mirrors dao/repository/RepositoryDAO.java:145-150
    pub(crate) fn log_duplicates(&self) {
        if self.duplicate_warnings.is_empty() {
            return;
        }
        let mut text = "Detected duplicate repository definitions during load:".to_owned();
        for warning in &self.duplicate_warnings {
            text.push_str("\n - ");
            text.push_str(warning);
        }
        tracing::warn!("{text}");
    }
}

impl LocalStore {
    /// Loads every `repositories/*.toml`, in file-name order.
    ///
    /// Failures and blank names are collected per file; only an unreadable directory is an
    /// error (`"Failed to read repository files\n<detail>"` as its chain).
    // Mirrors dao/repository/RepositoryDAO.java:103-167
    pub fn load_repositories(&self) -> Result<RepositoriesLoad, Error> {
        let dir = &self.dirs.repositories;
        let files = toml_file::list_toml_files(dir).map_err(|source| Error::ReadRepositories {
            path: dir.clone(),
            source,
        })?;
        let mut load = RepositoriesLoad::default();
        for path in files {
            let file = file_name(&path);
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(source) => {
                    load.failures.insert(file, Error::Read { path, source });
                    continue;
                }
            };
            let repository: Repository = match toml_file::decode(&path, &text, "repository") {
                Ok(repository) => repository,
                Err(err) => {
                    load.failures.insert(file, err);
                    continue;
                }
            };
            if super::naming::java_trim(&repository.name).is_empty() {
                load.failures
                    .insert(file, Error::MissingRepositoryName { path });
                continue;
            }
            load.register(repository, path);
        }
        load.log_duplicates();
        Ok(load)
    }

    /// Writes `repositories/<file_stem(name)>.toml` atomically.
    // Mirrors dao/repository/RepositoryDAO.java:196-239
    pub fn write_repository(&self, repository: &Repository) -> Result<(), Error> {
        let _guard = self.write_lock.lock();
        toml_file::write(
            &self.repository_path(&repository.name),
            "repository",
            repository,
        )
    }

    /// Like [`Self::write_repository`], then deletes `previous` (best effort) when it is another
    /// file, e.g. the file of a renamed repository.
    ///
    /// `previous` is only deleted when it is a `.toml` file directly inside the repositories
    /// directory; anything else is logged and kept.
    // Mirrors dao/repository/RepositoryDAO.java:209-232
    pub fn write_repository_replacing(
        &self,
        repository: &Repository,
        previous: Option<&Path>,
    ) -> Result<(), Error> {
        let _guard = self.write_lock.lock();
        let target = self.repository_path(&repository.name);
        toml_file::write(&target, "repository", repository)?;
        if let Some(previous) = previous.filter(|p| !same_file(p, &target)) {
            if !self.is_repository_file(previous) {
                tracing::warn!(
                    "Not deleting {}: not a repository file of this store.",
                    previous.display()
                );
                return Ok(());
            }
            match std::fs::remove_file(previous) {
                Ok(()) => tracing::info!(
                    "Migrated repository file \"{}\" to \"{}\".",
                    file_name(previous),
                    file_name(&target)
                ),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(
                    "Failed to delete previous repository file {}: {err}",
                    previous.display()
                ),
            }
        }
        Ok(())
    }

    /// Deletes the repository's file and both of its SHA-1 caches; whether any file was removed.
    // Mirrors dao/repository/RepositoryDAO.java:52-91
    pub fn remove_repository(&self, name: &str) -> Result<bool, Error> {
        let _guard = self.write_lock.lock();
        let mut deleted = false;
        for path in [
            self.repository_path(name),
            self.sha1_cache_path(name, CacheKind::Sync),
            self.sha1_cache_path(name, CacheKind::Build),
        ] {
            deleted |= Self::delete_file(&path)?;
        }
        Ok(deleted)
    }
}

impl LocalStore {
    /// Whether `path` is a `.toml` file whose canonical parent is the repositories directory.
    fn is_repository_file(&self, path: &Path) -> bool {
        let is_toml = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(TOML_EXTENSION));
        let parent = path.parent().and_then(|p| std::fs::canonicalize(p).ok());
        let repositories = std::fs::canonicalize(&self.dirs.repositories).ok();
        is_toml && parent.is_some() && parent == repositories
    }
}

/// Whether `a` and `b` name the same file (canonically if both exist).
fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
