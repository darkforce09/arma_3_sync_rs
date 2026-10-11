//! One-time, idempotent import of the Java client's data into the Rust store.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use a3s_error::{ErrorKind, UserFacing};
use a3s_hash::Sha1Cache;
use a3s_paths::legacy::LegacyJavaData;
use a3s_types::constants::DEFAULT_PROFILE_NAME;
use a3s_types::local::{FileAttributes, LegacyConfiguration, LegacyRepository, Profile};

use super::files::{
    LegacyFile, read_configuration_file, read_preferences_file, read_profile_file,
    read_repository_file,
};
use super::workshop::parse_workshop_file;
use crate::layout::{PROFILE_EXTENSION, REPOSITORY_EXTENSION};
use crate::local::naming::java_trim;
use crate::local::toml_file::modified_millis;
use crate::local::{self, CacheKind, ImportMarker, LocalStore, ignored_warning, replaced_warning};

/// Options of [`import`].
#[derive(Clone, Debug, Default)]
pub struct ImportOptions {
    /// Run even though an import marker exists (a re-import never overwrites either).
    pub ignore_marker: bool,
}

/// What happened to one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemOutcome {
    /// The source has no such item.
    Absent,

    /// The item was written to the Rust store.
    Imported,

    /// The Rust store already holds the item (or a better copy of it was chosen); nothing was
    /// written.
    SkippedExisting,

    /// The item could not be read; the text is the user-facing reason.
    Failed(String),
}

/// The outcome of one profile or repository file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedOutcome {
    /// The Java file name.
    pub file: String,

    /// The profile or repository name, when the file could be decoded.
    pub name: Option<String>,
    pub outcome: ItemOutcome,
}

/// The outcomes of one Java location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceReport {
    /// The Java configuration directory of the location.
    pub configuration_dir: PathBuf,

    /// `a3s.cfg`.
    pub configuration: ItemOutcome,

    /// `a3s.prefs`.
    pub preferences: ItemOutcome,

    /// `known-workshop-mods.properties`.
    pub workshop: ItemOutcome,
    pub profiles: Vec<NamedOutcome>,
    pub repositories: Vec<NamedOutcome>,

    /// What the migration of `a3s.cfg`'s deprecated fields did, if there was anything to
    /// migrate.
    pub migration: Option<String>,
}

impl SourceReport {
    fn new(configuration_dir: PathBuf) -> Self {
        Self {
            configuration_dir,
            configuration: ItemOutcome::Absent,
            preferences: ItemOutcome::Absent,
            workshop: ItemOutcome::Absent,
            profiles: Vec::new(),
            repositories: Vec::new(),
            migration: None,
        }
    }
}

/// The result of [`import`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub sources: Vec<SourceReport>,

    /// Decoder warnings (prefixed with the file name) and duplicate notices.
    pub warnings: Vec<String>,
}

/// Imported / skipped / failed counts of one kind.
#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    imported: u32,
    skipped: u32,
    failed: u32,
}

impl Counts {
    fn add(&mut self, outcome: &ItemOutcome) {
        match outcome {
            ItemOutcome::Absent => {}
            ItemOutcome::Imported => self.imported += 1,
            ItemOutcome::SkippedExisting => self.skipped += 1,
            ItemOutcome::Failed(_) => self.failed += 1,
        }
    }
}

/// The kinds of items, in report order.
const KINDS: [&str; 5] = [
    "profiles",
    "repositories",
    "configuration",
    "preferences",
    "workshop",
];

impl ImportReport {
    fn counts(&self) -> [Counts; 5] {
        let mut counts = [Counts::default(); 5];
        for source in &self.sources {
            for p in &source.profiles {
                counts[0].add(&p.outcome);
            }
            for r in &source.repositories {
                counts[1].add(&r.outcome);
            }
            counts[2].add(&source.configuration);
            counts[3].add(&source.preferences);
            counts[4].add(&source.workshop);
        }
        counts
    }

    /// One human-readable line per kind, e.g. `"Repositories: 2 imported, 1 skipped, 0 failed"`.
    pub fn summary(&self) -> Vec<String> {
        let titles = [
            "Profiles",
            "Repositories",
            "Configuration",
            "Preferences",
            "Workshop metadata",
        ];
        titles
            .iter()
            .zip(self.counts())
            .map(|(title, c)| {
                format!(
                    "{title}: {} imported, {} skipped, {} failed",
                    c.imported, c.skipped, c.failed
                )
            })
            .collect()
    }

    /// Whether at least one item was written to the store.
    pub fn imported_anything(&self) -> bool {
        self.counts().iter().any(|c| c.imported > 0)
    }

    fn marker_counts(&self) -> BTreeMap<String, u32> {
        let mut out = BTreeMap::new();
        for (kind, c) in KINDS.iter().zip(self.counts()) {
            out.insert(format!("{kind}.imported"), c.imported);
            out.insert(format!("{kind}.skipped"), c.skipped);
            out.insert(format!("{kind}.failed"), c.failed);
        }
        out
    }
}

/// The Rust store could not be read or written; the import stopped.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ImportError(#[from] pub local::Error);

impl UserFacing for ImportError {
    fn kind(&self) -> ErrorKind {
        self.0.kind()
    }

    fn user_message(&self) -> String {
        self.0.user_message()
    }
}

/// Whether to offer the import: no marker yet, and some Java data was found.
pub fn should_offer_import(store: &LocalStore, sources: &[LegacyJavaData]) -> bool {
    store.legacy_import_marker().is_none() && sources.iter().any(|s| !s.is_empty())
}

/// Imports the Java data of `sources` into `store`.
///
/// Per source, in order: profiles, repositories (with their SHA-1 caches), configuration
/// (migrating its deprecated fields into the active profile), preferences and Workshop metadata.
///
/// Nothing inside a source is modified and nothing already in the store is overwritten: an
/// existing profile or repository name, configuration or preferences file, or Workshop key is
/// skipped. Problems with Java files are recorded in the report; only a failure of the Rust
/// store is an error. On success the import marker is written, even if some files failed.
///
/// Without [`ImportOptions::ignore_marker`], an existing marker makes this a no-op.
pub fn import(
    store: &LocalStore,
    sources: &[LegacyJavaData],
    options: &ImportOptions,
) -> Result<ImportReport, ImportError> {
    if !options.ignore_marker && store.legacy_import_marker().is_some() {
        return Ok(ImportReport {
            sources: Vec::new(),
            warnings: vec!["The Java data was already imported.".to_owned()],
        });
    }
    let mut importer = Importer::new(store)?;
    for source in sources {
        let report = importer.source(source)?;
        importer.report.sources.push(report);
    }
    let report = importer.report;
    let marker = ImportMarker {
        imported_at_ms: now_millis(),
        sources: sources
            .iter()
            .map(|s| s.configuration_dir.display().to_string())
            .collect(),
        counts: report.marker_counts(),
    };
    store.write_legacy_import_marker(&marker)?;
    for line in report.summary() {
        tracing::info!("Java data import: {line}");
    }
    Ok(report)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A decoded candidate file.
struct Candidate<T> {
    file: String,
    path: PathBuf,
    value: T,
}

/// State of one import run.
struct Importer<'a> {
    store: &'a LocalStore,

    /// Profile names with a file in the store before the run.
    existing_profiles: BTreeSet<String>,

    /// Repository names in the store before the run.
    existing_repositories: BTreeSet<String>,

    /// Profiles written in this run, the targets of the configuration migration.
    imported_profiles: HashMap<String, Profile>,
    imported_repositories: BTreeSet<String>,
    configuration_done: bool,
    preferences_done: bool,
    report: ImportReport,
}

impl<'a> Importer<'a> {
    fn new(store: &'a LocalStore) -> Result<Self, ImportError> {
        let profiles = store.load_profiles()?;
        let repositories = store.load_repositories()?;
        Ok(Self {
            store,
            existing_profiles: profiles.sources.into_keys().collect(),
            existing_repositories: repositories.repositories.into_keys().collect(),
            imported_profiles: HashMap::new(),
            imported_repositories: BTreeSet::new(),
            configuration_done: store.configuration_path().exists(),
            preferences_done: store.preferences_path().exists(),
            report: ImportReport::default(),
        })
    }

    fn source(&mut self, source: &LegacyJavaData) -> Result<SourceReport, ImportError> {
        let mut report = SourceReport::new(source.configuration_dir.clone());
        if let Some(dir) = &source.profiles_dir {
            report.profiles = self.profiles(dir)?;
        }
        if let Some(dir) = &source.repositories_dir {
            report.repositories = self.repositories(dir)?;
        }
        if let Some(path) = &source.configuration_file {
            let (outcome, migration) = self.configuration(path)?;
            report.configuration = outcome;
            report.migration = migration;
        }
        if let Some(path) = &source.preferences_file {
            report.preferences = self.preferences(path)?;
        }
        if let Some(path) = &source.workshop_metadata_file {
            report.workshop = self.workshop(path)?;
        }
        Ok(report)
    }

    /// The regular files of `dir` accepted by `filter`, sorted by name. A listing failure is a
    /// warning.
    fn list(&mut self, dir: &Path, filter: impl Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                self.report
                    .warnings
                    .push(format!("Cannot list {}: {err}", dir.display()));
                return Vec::new();
            }
        };
        let mut files: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_owned();
                let is_file = std::fs::metadata(entry.path()).is_ok_and(|m| m.is_file());
                (is_file && filter(&name)).then(|| (name, entry.path()))
            })
            .collect();
        files.sort();
        files
    }

    /// Decodes the candidates; failures become outcomes, warnings go to the report.
    fn decode<T>(
        &mut self,
        files: Vec<(String, PathBuf)>,
        read: impl Fn(&Path) -> Result<LegacyFile<T>, super::Error>,
        outcomes: &mut Vec<(usize, NamedOutcome)>,
    ) -> Vec<(usize, Candidate<T>)> {
        let mut decoded = Vec::new();
        for (index, (file, path)) in files.into_iter().enumerate() {
            match read(&path) {
                Ok(mut legacy) => {
                    self.report.warnings.append(&mut legacy.warnings);
                    decoded.push((
                        index,
                        Candidate {
                            file,
                            path,
                            value: legacy.value,
                        },
                    ));
                }
                Err(err) => outcomes.push((
                    index,
                    NamedOutcome {
                        file,
                        name: None,
                        outcome: ItemOutcome::Failed(err.to_string()),
                    },
                )),
            }
        }
        decoded
    }

    /// Profiles: files whose name *contains* `.a3s.profile`, so `.backup` copies count; a
    /// primary file beats a copy, then the newer file wins.
    // Mirrors dao/ProfileDAO.java:24-42
    fn profiles(&mut self, dir: &Path) -> Result<Vec<NamedOutcome>, ImportError> {
        let files = self.list(dir, |name| name.contains(PROFILE_EXTENSION));
        let mut outcomes = Vec::new();
        let decoded = self.decode(files, read_profile_file, &mut outcomes);

        // Winner per name: (index into `decoded`, primary, mtime).
        let mut winners: BTreeMap<String, (usize, bool, i64)> = BTreeMap::new();
        let mut losers = Vec::new();
        for (i, (_, candidate)) in decoded.iter().enumerate() {
            let primary = candidate.file.ends_with(PROFILE_EXTENSION);
            let mtime = modified_millis(&candidate.path);
            let name = candidate.value.name.clone();
            match winners.get(&name).copied() {
                None => {
                    winners.insert(name, (i, primary, mtime));
                }
                Some((kept, kept_primary, kept_mtime)) => {
                    let kept_file = &decoded[kept].1.file;
                    let better = (primary && !kept_primary)
                        || (primary == kept_primary && mtime > kept_mtime);
                    if better {
                        self.report.warnings.push(replaced_warning(
                            &name,
                            kept_file,
                            &candidate.file,
                        ));
                        losers.push(kept);
                        winners.insert(name, (i, primary, mtime));
                    } else {
                        self.report.warnings.push(ignored_warning(
                            &name,
                            &candidate.file,
                            kept_file,
                        ));
                        losers.push(i);
                    }
                }
            }
        }

        for (i, (index, candidate)) in decoded.into_iter().enumerate() {
            let name = candidate.value.name.clone();
            let outcome = if losers.contains(&i)
                || self.existing_profiles.contains(&name)
                || self.imported_profiles.contains_key(&name)
                || self.store.profile_path(&name).exists()
            {
                ItemOutcome::SkippedExisting
            } else {
                self.store.write_profile(&candidate.value)?;
                self.imported_profiles.insert(name.clone(), candidate.value);
                ItemOutcome::Imported
            };
            outcomes.push((
                index,
                NamedOutcome {
                    file: candidate.file,
                    name: Some(name),
                    outcome,
                },
            ));
        }
        outcomes.sort_by_key(|(index, _)| *index);
        Ok(outcomes.into_iter().map(|(_, o)| o).collect())
    }

    /// Repositories: files whose name *ends with* `.a3s.repository`; of two with the same name
    /// the strictly newer one wins.
    // Mirrors dao/repository/RepositoryDAO.java:103-190
    fn repositories(&mut self, dir: &Path) -> Result<Vec<NamedOutcome>, ImportError> {
        let files = self.list(dir, |name| name.ends_with(REPOSITORY_EXTENSION));
        let mut outcomes = Vec::new();
        let decoded = self.decode(files, read_repository_file, &mut outcomes);

        let mut winners: BTreeMap<String, (usize, i64)> = BTreeMap::new();
        let mut losers = Vec::new();
        let mut blank = Vec::new();
        for (i, (_, candidate)) in decoded.iter().enumerate() {
            let name = candidate.value.repository.name.clone();
            if java_trim(&name).is_empty() {
                blank.push(i);
                continue;
            }
            let mtime = modified_millis(&candidate.path);
            match winners.get(&name).copied() {
                None => {
                    winners.insert(name, (i, mtime));
                }
                Some((kept, kept_mtime)) => {
                    let kept_file = &decoded[kept].1.file;
                    if mtime > kept_mtime {
                        self.report.warnings.push(replaced_warning(
                            &name,
                            kept_file,
                            &candidate.file,
                        ));
                        losers.push(kept);
                        winners.insert(name, (i, mtime));
                    } else {
                        self.report.warnings.push(ignored_warning(
                            &name,
                            &candidate.file,
                            kept_file,
                        ));
                        losers.push(i);
                    }
                }
            }
        }

        for (i, (index, candidate)) in decoded.into_iter().enumerate() {
            let name = candidate.value.repository.name.clone();
            let outcome = if blank.contains(&i) {
                // Mirrors dao/repository/RepositoryDAO.java:121-124
                ItemOutcome::Failed("Repository name is missing.".to_owned())
            } else if losers.contains(&i)
                || self.existing_repositories.contains(&name)
                || self.imported_repositories.contains(&name)
                || self.store.repository_path(&name).exists()
            {
                ItemOutcome::SkippedExisting
            } else {
                self.write_repository(&candidate.value)?;
                self.imported_repositories.insert(name.clone());
                ItemOutcome::Imported
            };
            outcomes.push((
                index,
                NamedOutcome {
                    file: candidate.file,
                    name: Some(name),
                    outcome,
                },
            ));
        }
        outcomes.sort_by_key(|(index, _)| *index);
        Ok(outcomes.into_iter().map(|(_, o)| o).collect())
    }

    /// Writes the repository and its non-empty caches, keeping Java's keys and ms mtimes.
    fn write_repository(&self, legacy: &LegacyRepository) -> Result<(), ImportError> {
        let name = &legacy.repository.name;
        self.store.write_repository(&legacy.repository)?;
        for (kind, map) in [
            (CacheKind::Sync, &legacy.files_for_sync),
            (CacheKind::Build, &legacy.files_for_build),
        ] {
            if !map.is_empty() {
                self.store.write_sha1_cache(name, kind, &to_cache(map))?;
            }
        }
        Ok(())
    }

    fn configuration(&mut self, path: &Path) -> Result<(ItemOutcome, Option<String>), ImportError> {
        if self.configuration_done {
            return Ok((ItemOutcome::SkippedExisting, None));
        }
        let legacy = match read_configuration_file(path) {
            Ok(legacy) => legacy,
            Err(err) => return Ok((ItemOutcome::Failed(err.to_string()), None)),
        };
        self.report.warnings.extend(legacy.warnings);
        self.store
            .write_configuration(&legacy.value.configuration)?;
        self.configuration_done = true;
        let migration = self.migrate(&legacy.value)?;
        Ok((ItemOutcome::Imported, migration))
    }

    /// Moves the deprecated `a3s.cfg` fields into the active profile imported in this run.
    // Mirrors ui/main/MainPanel.java:538-569
    fn migrate(&mut self, legacy: &LegacyConfiguration) -> Result<Option<String>, ImportError> {
        if legacy.addon_search_directory_paths.is_none() && legacy.launcher_options.is_none() {
            return Ok(None);
        }
        let active = &legacy.configuration.profile_name;
        let (mut profile, created) = if let Some(profile) = self.imported_profiles.get(active) {
            (profile.clone(), false)
        } else if active == DEFAULT_PROFILE_NAME
            && !self.existing_profiles.contains(DEFAULT_PROFILE_NAME)
            && !self.store.profile_path(DEFAULT_PROFILE_NAME).exists()
        {
            // Java's in-memory default profile (dao/ProfileDAO.java:44-48).
            (Profile::new(DEFAULT_PROFILE_NAME), true)
        } else {
            return Ok(Some(format!(
                "The search directories and launcher options of a3s.cfg were not migrated: \
                 profile \"{active}\" was not imported."
            )));
        };

        let mut parts = Vec::new();
        if let Some(paths) = &legacy.addon_search_directory_paths {
            // Appended without de-duplication (service/ProfileService.java:197-209).
            profile
                .addon_search_directories
                .extend(paths.iter().cloned());
            parts.push(format!("{} search directories", paths.len()));
        }
        if let Some(old) = &legacy.launcher_options {
            let lo = &mut profile.launcher_options;
            lo.arma3_exe_path.clone_from(&old.arma3_exe_path);
            lo.auto_restart = old.auto_restart;
            lo.check_signatures = old.check_signatures;
            lo.file_patching = old.file_patching;
            lo.no_pause = old.no_pause;
            lo.show_script_errors = old.show_script_errors;
            lo.window_mode = old.window_mode;
            // `Integer.parseInt(Integer.toString(x))` is the identity.
            lo.cpu_count_selection = old.cpu_count_selection;
            lo.default_world = old.default_world;
            lo.enable_ht = old.enable_ht;
            lo.huge_pages = old.huge_pages;
            lo.ex_threads_selection
                .clone_from(&old.ex_threads_selection);
            lo.game_profile.clone_from(&old.game_profile);
            lo.malloc_selection.clone_from(&old.malloc_selection);
            lo.max_memory_selection
                .clone_from(&old.max_memory_selection);
            lo.nologs = old.nologs;
            lo.no_splash_screen = old.no_splash_screen;
            parts.push("the launcher options".to_owned());
        }
        self.store.write_profile(&profile)?;
        self.imported_profiles.insert(active.clone(), profile);
        let created = if created { " (created)" } else { "" };
        Ok(Some(format!(
            "Migrated {} of a3s.cfg into profile \"{active}\"{created}.",
            parts.join(" and ")
        )))
    }

    fn preferences(&mut self, path: &Path) -> Result<ItemOutcome, ImportError> {
        if self.preferences_done {
            return Ok(ItemOutcome::SkippedExisting);
        }
        match read_preferences_file(path) {
            Ok(legacy) => {
                self.report.warnings.extend(legacy.warnings);
                self.store.write_preferences(&legacy.value)?;
                self.preferences_done = true;
                Ok(ItemOutcome::Imported)
            }
            Err(err) => Ok(ItemOutcome::Failed(err.to_string())),
        }
    }

    fn workshop(&mut self, path: &Path) -> Result<ItemOutcome, ImportError> {
        let legacy = match parse_workshop_file(path) {
            Ok(legacy) => legacy,
            Err(warning) => {
                self.report.warnings.push(warning.clone());
                return Ok(ItemOutcome::Failed(warning));
            }
        };
        self.report.warnings.extend(legacy.warnings);
        let added = self.store.workshop().insert_missing(&legacy.value)?;
        Ok(if added.is_empty() {
            ItemOutcome::SkippedExisting
        } else {
            ItemOutcome::Imported
        })
    }
}

fn to_cache(map: &BTreeMap<String, FileAttributes>) -> Sha1Cache {
    Sha1Cache {
        entries: map
            .iter()
            .map(|(path, attributes)| {
                (
                    PathBuf::from(path),
                    a3s_hash::FileAttributes {
                        sha1: attributes.sha1.clone(),
                        last_modified: attributes.last_modified,
                    },
                )
            })
            .collect(),
    }
}
