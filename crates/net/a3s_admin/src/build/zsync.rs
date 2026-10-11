//! `.zsync` stage (B37–B40).

use std::fs;
use std::io::{self, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_repo_files::layout::ZSYNC_EXTENSION;
use a3s_types::Protocol;
use a3s_zsync::{ControlFile, format_mtime};
use parking_lot::Mutex;
use rayon::prelude::*;

use super::scan::LeafRef;
use super::{BuildError, BuildEvent, Progress, worker_pool};

/// One `.zsync` to (re)generate.
pub(crate) struct ZsyncJob {
    pub file: PathBuf,
    pub name: String,
    pub url: String,
    pub sha1: Option<String>,
}

/// `<file>.zsync`.
pub(crate) fn zsync_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(ZSYNC_EXTENSION);
    PathBuf::from(name)
}

/// Leaves whose `.zsync` must be (re)generated: re-hashed, missing or empty (B38).
// Mirrors dao/repository/RepositoryBuildProcessor.java:595-617
pub(crate) fn plan(
    leaves: &[LeafRef],
    names: &[String],
    sha1: &[Option<String>],
    rehashed: &[bool],
    protocol: &Protocol,
) -> Vec<ZsyncJob> {
    let mut jobs = Vec::new();
    for (i, leaf) in leaves.iter().enumerate() {
        if !leaf.path.exists() {
            continue;
        }
        let zsync = zsync_path(&leaf.path);
        let compute = rehashed[i] || fs::metadata(&zsync).map_or(true, |m| m.len() == 0);
        if compute {
            jobs.push(ZsyncJob {
                file: leaf.path.clone(),
                name: names[i].clone(),
                // No port and no URL encoding.
                // Mirrors dao/repository/RepositoryZsyncProcessor.java:71-72
                url: format!(
                    "{}{}/{}",
                    protocol.protocol_type.prompt(),
                    protocol.url,
                    leaf.relpath
                ),
                sha1: sha1[i].clone(),
            });
        }
    }
    jobs
}

/// Generates one `.zsync`, written to a temporary file and renamed (Q10).
// Mirrors jazsyncmake/MetaFileMaker.java:64-121
fn generate(job: &ZsyncJob) -> io::Result<()> {
    let meta = fs::metadata(&job.file)?;
    let mtime = a3s_hash::modified_millis(&job.file)?;
    let reader = BufReader::new(fs::File::open(&job.file)?);
    let control = ControlFile::generate(
        reader,
        meta.len(),
        &job.name,
        &format_mtime(mtime),
        &job.url,
        // `HeaderMaker` appends a null SHA-1 as "null".
        job.sha1.as_deref().unwrap_or("null"),
    )
    .map_err(|err| match err {
        a3s_zsync::Error::Io(err) => err,
        other => io::Error::other(other),
    })?;
    let target = zsync_path(&job.file);
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(&control.to_bytes())?;
    tmp.persist(&target).map_err(|err| err.error)?;
    Ok(())
}

/// Generates the `.zsync` files in parallel. On the first error the remaining files are skipped
/// and that error is returned.
// Mirrors dao/repository/RepositoryZsyncProcessor.java:51-305
pub(crate) fn run(
    jobs: &[ZsyncJob],
    cancel: &AtomicBool,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Result<usize, BuildError> {
    if jobs.is_empty() {
        return Ok(0);
    }
    let failed = AtomicBool::new(false);
    let first_error: Mutex<Option<BuildError>> = Mutex::new(None);
    let progress = Mutex::new(Progress::new(jobs.len()));
    let work = || {
        jobs.par_iter().for_each(|job| {
            if cancel.load(Ordering::Relaxed) || failed.load(Ordering::Relaxed) {
                return;
            }
            match generate(job) {
                Ok(()) => {
                    let value = progress.lock().increment();
                    on_event(BuildEvent::Progress(value));
                }
                Err(source) => {
                    failed.store(true, Ordering::Relaxed);
                    first_error.lock().get_or_insert(BuildError::Zsync {
                        path: zsync_path(&job.file),
                        source,
                    });
                }
            }
        });
    };
    worker_pool("a3s-zsync", super::hash_threads()).map_or_else(&work, |pool| pool.install(work));
    if let Some(err) = first_error.into_inner() {
        return Err(err);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(BuildError::Cancelled);
    }
    tracing::info!("Number of zsync files generated = {}", jobs.len());
    Ok(jobs.len())
}

/// Non-HTTP repositories: Java deletes the `.zsync` files of leaves whose own extension is
/// `.zsync`. The scan never produces such leaves, so nothing is ever deleted (B39).
// Mirrors dao/repository/RepositoryDeleteZSyncProcessor.java:54-86
pub(crate) fn delete_stale(
    leaves: &[LeafRef],
    names: &[String],
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> usize {
    let targets: Vec<&LeafRef> = leaves
        .iter()
        .zip(names)
        .filter(|(_, name)| {
            name.rfind('.')
                .is_some_and(|i| name[i..].to_lowercase() == ZSYNC_EXTENSION)
        })
        .map(|(leaf, _)| leaf)
        .collect();
    let mut progress = Progress::new(targets.len());
    for leaf in &targets {
        super::delete_file(&leaf.path, on_event);
        on_event(BuildEvent::Progress(progress.increment()));
    }
    tracing::info!("Number of zsync files deleted = {}", targets.len());
    targets.len()
}
