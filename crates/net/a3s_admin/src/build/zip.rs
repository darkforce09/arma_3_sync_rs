//! Compression stage: `<file>.zip` next to every `.pbo`/`.ebo` (B41–B43).

use std::fs;
use std::io::{self, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_repo_files::layout::{ZIP_EXTENSION, is_compressible};
use parking_lot::Mutex;
use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use super::scan::LeafRef;
use super::{BuildError, BuildEvent, Progress, worker_pool};

/// Sources of at least 4 GiB need ZIP64.
const ZIP64_THRESHOLD: u64 = 0xFFFF_FFFF;

/// `<file>.zip`.
pub(crate) fn zip_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(ZIP_EXTENSION);
    PathBuf::from(name)
}

/// What the compression stage does with a compressible leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZipAction {
    /// Not a `.pbo`/`.ebo`.
    None,

    /// The existing `.zip` is reused: `compressed = true`, `compressed_size = <zip length>`.
    Reuse(i64),

    /// (Re)compress.
    Compress,
}

/// Decides per leaf: re-hashed leaves lose their zip and are compressed again, leaves without a
/// zip are compressed, others reuse their zip (B42).
// Mirrors dao/repository/RepositoryBuildProcessor.java:548-575
pub(crate) fn plan(
    leaves: &[LeafRef],
    names: &[String],
    rehashed: &[bool],
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Vec<ZipAction> {
    leaves
        .iter()
        .enumerate()
        .map(|(i, leaf)| {
            if !is_compressible(&names[i]) {
                return ZipAction::None;
            }
            let zip = zip_path(&leaf.path);
            if rehashed[i] {
                // "Force delete in case of the user has stopped the process previously".
                super::delete_file(&zip, on_event);
                ZipAction::Compress
            } else {
                match fs::metadata(&zip) {
                    Ok(meta) => ZipAction::Reuse(i64::try_from(meta.len()).unwrap_or(i64::MAX)),
                    Err(_) => ZipAction::Compress,
                }
            }
        })
        .collect()
}

/// Writes `<file>.zip` with one DEFLATE (level 9) entry named after the file, through a temporary
/// file that is deleted if compression fails or is cancelled. Returns the zip's length.
// Mirrors dao/zip/ZipDAO.java:112-185
pub(crate) fn zip_file(file: &Path, cancel: &AtomicBool) -> Result<u64, BuildError> {
    let err = |source: io::Error| BuildError::Zip {
        path: file.to_path_buf(),
        source,
    };
    let zip_err = |e: zip::result::ZipError| err(io::Error::other(e));
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut source = BufReader::new(fs::File::open(file).map_err(err)?);
    let len = fs::metadata(file).map_err(err)?.len();
    let target = zip_path(file);
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::NamedTempFile::new_in(dir).map_err(err)?;
    let mut writer = ZipWriter::new(tmp);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(9))
        .last_modified_time(DateTime::DEFAULT)
        .large_file(len >= ZIP64_THRESHOLD);
    writer.start_file(name, options).map_err(zip_err)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            // Dropping the temporary file deletes the partial zip.
            return Err(BuildError::Cancelled);
        }
        let n = source.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).map_err(err)?;
    }
    let tmp = writer.finish().map_err(zip_err)?;
    tmp.as_file().sync_all().map_err(err)?;
    tmp.persist(&target).map_err(|e| err(e.error))?;
    fs::metadata(&target).map(|m| m.len()).map_err(err)
}

/// Compresses the leaves marked [`ZipAction::Compress`] in parallel and returns their zip sizes
/// (index-aligned with `leaves`). The first error stops the remaining work.
// Mirrors dao/zip/ZipBatchProcessor.java:31-85
pub(crate) fn run(
    leaves: &[LeafRef],
    actions: &[ZipAction],
    cancel: &AtomicBool,
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> Result<Vec<Option<u64>>, BuildError> {
    let jobs: Vec<usize> = (0..leaves.len())
        .filter(|&i| actions[i] == ZipAction::Compress)
        .collect();
    let sizes = Mutex::new(vec![None; leaves.len()]);
    let failed = AtomicBool::new(false);
    let first_error: Mutex<Option<BuildError>> = Mutex::new(None);
    let progress = Mutex::new(Progress::new(jobs.len()));
    let work = || {
        jobs.par_iter().for_each(|&i| {
            if cancel.load(Ordering::Relaxed) || failed.load(Ordering::Relaxed) {
                return;
            }
            match zip_file(&leaves[i].path, cancel) {
                Ok(size) => {
                    sizes.lock()[i] = Some(size);
                    let value = progress.lock().increment();
                    on_event(BuildEvent::Progress(value));
                }
                Err(e) => {
                    failed.store(true, Ordering::Relaxed);
                    first_error.lock().get_or_insert(e);
                }
            }
        });
    };
    if !jobs.is_empty() {
        worker_pool("a3s-zip", super::cpu_threads()).map_or_else(&work, |pool| pool.install(work));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(BuildError::Cancelled);
    }
    if let Some(e) = first_error.into_inner() {
        return Err(e);
    }
    tracing::info!("Number of files compressed = {}", jobs.len());
    Ok(sizes.into_inner())
}

/// Uncompressed repositories: deletes the `.zip` of every compressible leaf that has one; the
/// caller clears `compressed` on every compressible leaf (B43). Returns the number of zips.
// Mirrors dao/zip/DeleteZipBatchProcessor.java:54-105
pub(crate) fn delete(
    leaves: &[LeafRef],
    names: &[String],
    on_event: &(dyn Fn(BuildEvent) + Sync),
) -> usize {
    let targets: Vec<PathBuf> = leaves
        .iter()
        .zip(names)
        .filter(|(_, name)| is_compressible(name))
        .map(|(leaf, _)| zip_path(&leaf.path))
        .filter(|zip| zip.exists())
        .collect();
    let progress = Mutex::new(Progress::new(targets.len()));
    let work = || {
        targets.par_iter().for_each(|zip| {
            super::delete_file(zip, on_event);
            let value = progress.lock().increment();
            on_event(BuildEvent::Progress(value));
        });
    };
    if !targets.is_empty() {
        worker_pool("a3s-zip", super::cpu_threads()).map_or_else(&work, |pool| pool.install(work));
    }
    tracing::info!("Number of zip files deleted = {}", targets.len());
    targets.len()
}
