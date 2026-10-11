//! zsync completion: how much of each outdated local file already matches the remote one.
//!
//! Port of `ConnectionService.determineFilesCompletion`, `FilesCompletionProcessor`,
//! `ConnectionCompletionProcessor`, `HttpDAO.getFileCompletion` and jazsync
//! `FileMaker.matchControl`. Only HTTP(S) repositories publish `.zsync` files; on every other
//! transport each outdated file is `0.0` complete (a full download) without any network access.
//! The download then patches files with a non-zero completion and downloads the others whole.

// `CompletionError` keeps the `RemoteError` inline; errors end the completion, so their size is
// irrelevant.
#![allow(clippy::result_large_err)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use a3s_error::{ErrorKind, UserFacing};
use a3s_transport::{TransferCtx, TransportOptions};
use a3s_types::{Protocol, ProtocolType, ServerInfo};
use parking_lot::Mutex;

use crate::RemoteError;
use crate::events::{EventSender, PercentCounter, Phase, SyncEvent};
use crate::model::{PlanNode, PlanTree, file_name, parent_relative_path};
use crate::remote::{PartialTransferSupport, RemoteConnection, RemoteFile};

/// Suffix of the zsync control file published next to every file of an HTTP repository.
// Mirrors dao/DataAccessConstants.java:21
pub const ZSYNC_EXTENSION: &str = ".zsync";

/// What [`determine_completion`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompletionStats {
    /// Outdated local files checked against their `.zsync`.
    pub checked: usize,

    /// Checked files with some content already matching (`complete > 0.0`).
    pub partial: usize,
}

/// Determining completion failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CompletionError {
    /// Connecting, the partial-transfer probe or a `.zsync` fetch failed.
    #[error(transparent)]
    Remote(#[from] RemoteError),

    /// A `.zsync` file could not be parsed, or the local file could not be read.
    #[error("{message}")]
    Zsync {
        /// Path of the `.zsync` file below the repository root, without a leading `/`.
        relative_path: String,
        /// `"Failed to read zsync file: /{relative_path}\n{source}"`.
        message: String,
        /// The zsync error.
        #[source]
        source: a3s_zsync::Error,
    },

    /// The operation was cancelled.
    #[error("Cancelled")]
    Cancelled,
}

impl CompletionError {
    /// Whether a new connection may help (Java: "connection lost").
    pub fn is_connection_lost(&self) -> bool {
        match self {
            Self::Remote(e) => e.is_connection_lost(),
            Self::Zsync { .. } | Self::Cancelled => false,
        }
    }

    /// A remote error, with cancellation mapped to [`CompletionError::Cancelled`].
    fn remote(err: RemoteError) -> Self {
        match err {
            RemoteError::Cancelled => Self::Cancelled,
            other => Self::Remote(other),
        }
    }
}

impl UserFacing for CompletionError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Remote(e) => e.kind(),
            Self::Zsync { source, .. } => zsync_kind(source),
            Self::Cancelled => ErrorKind::Cancelled,
        }
    }

    /// The Java message; it already contains the cause.
    fn user_message(&self) -> String {
        self.to_string()
    }
}

fn zsync_kind(err: &a3s_zsync::Error) -> ErrorKind {
    match err {
        a3s_zsync::Error::Io(e) => ErrorKind::from_io(e),
        a3s_zsync::Error::Cancelled => ErrorKind::Cancelled,
        _ => ErrorKind::InvalidData,
    }
}

/// Parallel completion checks: the server's connection count, at least 1, at most the number
/// of CPUs. The client's own connection setting is not used (Java quirk kept).
// Mirrors service/synchronization/FilesCompletionProcessor.java:37-49
pub fn completion_connections(server_info: &ServerInfo) -> usize {
    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    usize::try_from(server_info.number_of_connections)
        .unwrap_or(0)
        .max(1)
        .min(cpus)
}

/// Percentage of target blocks available locally, after jazsync's clean-up of isolated matches.
///
/// With `seq_matches == 2`, a matched block whose neighbours are both unmatched is dropped (the
/// first and last blocks need only their single neighbour unmatched), evaluated left to right
/// on the updated map like Java. `0.0` when there are no blocks.
// Mirrors jazsync/FileMaker.java:495-525
pub fn completion_percent(matches: &a3s_zsync::BlockMatches, seq_matches: usize) -> f64 {
    let mut map: Vec<bool> = matches.local_offsets.iter().map(Option::is_some).collect();
    let n = map.len();
    if n == 0 {
        return 0.0;
    }
    // Java fails with an index error for one block; jazsync never writes that combination.
    if seq_matches == 2 && n >= 2 {
        for i in 0..n {
            let isolated = if i == 0 {
                map[0] && !map[1]
            } else if i == n - 1 {
                map[i] && !map[i - 1]
            } else {
                !map[i - 1] && map[i] && !map[i + 1]
            };
            if isolated {
                map[i] = false;
            }
        }
    }
    let missing = map.iter().filter(|m| !**m).count();
    (n - missing) as f64 / n as f64 * 100.0
}

/// Sets `complete` on the leaves that need no check and returns the relative paths of those
/// that do, pre-order (deleted leaves included):
///
/// * not `updated` → `100.0`;
/// * `updated` with a compressed-only repository, no partial transfer, or no local file →
///   `0.0`;
/// * other `updated` leaves are returned, untouched.
// Mirrors service/ConnectionService.java:300-323
pub fn classify_leaves(
    tree: &mut PlanTree,
    server_info: &ServerInfo,
    partial_supported: bool,
) -> Vec<String> {
    let mut candidates = Vec::new();
    tree.walk_mut(&mut |relative_path, node| {
        let PlanNode::Leaf(leaf) = node else {
            return;
        };
        if !leaf.updated {
            leaf.complete = 100.0;
        } else if server_info.compressed_pbo_files_only
            || !partial_supported
            || !leaf.local_path().is_some_and(|p| p.exists())
        {
            leaf.complete = 0.0;
        } else {
            candidates.push(relative_path.to_owned());
        }
    });
    candidates
}

/// Completion of `local` against `control`; `0.0` for an empty file. Blocking.
///
/// `cancel` is polled while scanning; when set the scan fails with
/// [`a3s_zsync::Error::Cancelled`].
// Mirrors dao/connection/http/HttpDAO.java:455, jazsync/FileMaker.java:263-264,429-430
pub fn local_completion(
    control: &a3s_zsync::ControlFile,
    local: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<f64, a3s_zsync::Error> {
    let file = File::open(local)?;
    if file.metadata()?.len() == 0 {
        return Ok(0.0);
    }
    let matches = a3s_zsync::match_blocks(control, file, cancel)?;
    Ok(completion_percent(
        &matches,
        control.header.hash_lengths.seq_matches,
    ))
}

/// Sets `complete` on every leaf of a planned tree.
///
/// Probes partial-transfer support first (HTTP(S) only, unless the server info disables it),
/// then fetches the `.zsync` of every outdated local file over
/// [`completion_connections`] connections and matches it against the local copy. Bytes of the
/// `.zsync` files count in `ctx.progress`.
///
/// After the first error, no new file is started and the first error is returned (approved
/// deviation DV-P6); completions already computed are written to the tree but must be
/// considered unreliable. Cancellation returns [`CompletionError::Cancelled`] (if the runtime
/// shuts down while classifying, the tree is left empty).
///
/// Emits [`Phase::DeterminingCompletion`], then a [`SyncEvent::Progress`] per percent of the
/// checked files.
// Mirrors service/ConnectionService.java:270-328,
// service/synchronization/FilesCompletionProcessor.java:31-84,
// service/connection/ConnectionCompletionProcessor.java:1-144
pub async fn determine_completion(
    tree: &mut PlanTree,
    protocol: &Protocol,
    options: &TransportOptions,
    server_info: &ServerInfo,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<CompletionStats, CompletionError> {
    events.emit(SyncEvent::Phase(Phase::DeterminingCompletion));

    // Only HTTP repositories have `.zsync` files; Java opens connections whose
    // `getFileCompletion` returns 0 (approved deviation DV-P4: no connection at all).
    // Mirrors dao/connection/ftp/FtpDAO.java:733-735, sftp/SftpDAO.java:351-353,
    // webdav/WebdavDAO.java:415-417
    let is_http = matches!(
        protocol.protocol_type,
        ProtocolType::Http | ProtocolType::Https
    );
    let mut probe_connection = None;
    let mut partial_supported = false;
    // Mirrors service/ConnectionService.java:285-298
    if is_http && !server_info.no_partial_file_transfer {
        let mut connection = RemoteConnection::connect(protocol, options, ctx)
            .await
            .map_err(CompletionError::remote)?;
        let support = connection
            .probe_partial_transfer(ctx)
            .await
            .map_err(CompletionError::remote)?;
        partial_supported = support == PartialTransferSupport::Supported;
        probe_connection = Some(connection);
    }

    // Classification stats every outdated local file: run it on a blocking thread.
    let owned = std::mem::take(tree);
    let info = server_info.clone();
    let classified = tokio::task::spawn_blocking(move || {
        let mut owned = owned;
        let candidates: Vec<Candidate> = classify_leaves(&mut owned, &info, partial_supported)
            .into_iter()
            .filter_map(|relative_path| {
                let local_path = owned.get(&relative_path)?.local_path()?;
                Some(Candidate {
                    relative_path,
                    local_path,
                })
            })
            .collect();
        (owned, candidates)
    })
    .await;
    let candidates = match classified {
        Ok((owned, candidates)) => {
            *tree = owned;
            candidates
        }
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        // The runtime is shutting down; the tree is lost with the task.
        Err(_) => return Err(CompletionError::Cancelled),
    };
    if ctx.cancel.is_cancelled() {
        return Err(CompletionError::Cancelled);
    }
    if candidates.is_empty() {
        if let Some(connection) = probe_connection {
            connection.close().await;
        }
        return Ok(CompletionStats::default());
    }

    let workers = completion_connections(server_info).min(candidates.len());
    let counter = PercentCounter::new(candidates.len() as u64);
    let shared = Shared {
        protocol,
        options,
        ctx,
        events,
        // Java pops from a `Stack` filled in tree order: the last file is checked first.
        stack: Mutex::new(candidates),
        failed: AtomicBool::new(false),
        first_error: Mutex::new(None),
        results: Mutex::new(Vec::new()),
        counter: Mutex::new(counter),
    };
    let mut connections = vec![probe_connection];
    connections.resize_with(workers, || None);
    futures::future::join_all(
        connections
            .into_iter()
            .map(|connection| worker(&shared, connection)),
    )
    .await;

    let Shared {
        first_error,
        results,
        ..
    } = shared;
    let results = results.into_inner();
    let stats = CompletionStats {
        checked: results.len(),
        partial: results.iter().filter(|(_, c)| *c > 0.0).count(),
    };
    for (relative_path, complete) in results {
        if let Some(PlanNode::Leaf(leaf)) = tree.get_mut(&relative_path) {
            leaf.complete = complete;
        }
    }
    if let Some(err) = first_error.into_inner() {
        return Err(err);
    }
    if ctx.cancel.is_cancelled() {
        return Err(CompletionError::Cancelled);
    }
    Ok(stats)
}

/// An outdated local file to check.
struct Candidate {
    relative_path: String,
    local_path: PathBuf,
}

/// State shared by the completion workers.
struct Shared<'a> {
    protocol: &'a Protocol,
    options: &'a TransportOptions,
    ctx: &'a TransferCtx,
    events: &'a EventSender,
    stack: Mutex<Vec<Candidate>>,
    failed: AtomicBool,
    first_error: Mutex<Option<CompletionError>>,
    results: Mutex<Vec<(String, f64)>>,
    counter: Mutex<PercentCounter>,
}

impl Shared<'_> {
    fn fail(&self, err: CompletionError) {
        self.failed.store(true, Ordering::SeqCst);
        let mut first = self.first_error.lock();
        if first.is_none() {
            *first = Some(err);
        }
    }

    fn stopped(&self) -> bool {
        self.failed.load(Ordering::SeqCst) || self.ctx.cancel.is_cancelled()
    }
}

/// Checks files from the shared stack until it is empty, an error occurred or the operation
/// is cancelled. Connects on the first file unless given the probe's connection.
// Mirrors service/connection/ConnectionCompletionProcessor.java:71-101
async fn worker(shared: &Shared<'_>, mut connection: Option<RemoteConnection>) {
    let mut interrupted = false;
    loop {
        if shared.stopped() {
            break;
        }
        let Some(candidate) = shared.stack.lock().pop() else {
            break;
        };
        if connection.is_none() {
            match RemoteConnection::connect(shared.protocol, shared.options, shared.ctx).await {
                Ok(conn) => connection = Some(conn),
                Err(RemoteError::Cancelled) => break,
                Err(err) => {
                    shared.fail(CompletionError::Remote(err));
                    break;
                }
            }
        }
        let Some(conn) = connection.as_mut() else {
            break;
        };
        match file_completion(conn, &candidate, shared.ctx).await {
            Ok(complete) => {
                shared
                    .results
                    .lock()
                    .push((candidate.relative_path, complete));
                // Mirrors service/connection/ConnectionCompletionProcessor.java:138-143
                if let Some(percent) = shared.counter.lock().increment() {
                    shared.events.emit(SyncEvent::Progress {
                        phase: Phase::DeterminingCompletion,
                        percent,
                    });
                }
            }
            Err(CompletionError::Cancelled) => {
                interrupted = true;
                break;
            }
            Err(err) => {
                shared.fail(err);
                break;
            }
        }
    }
    if let Some(connection) = connection {
        // A fetch interrupted by cancellation leaves the connection in an unknown state.
        if !interrupted && !shared.ctx.cancel.is_cancelled() {
            connection.close().await;
        }
    }
}

/// Completion of one file: fetches its `.zsync` and matches it against the local copy.
// Mirrors dao/connection/http/HttpDAO.java:449-490
async fn file_completion(
    connection: &mut RemoteConnection,
    candidate: &Candidate,
    ctx: &TransferCtx,
) -> Result<f64, CompletionError> {
    let local =
        std::path::absolute(&candidate.local_path).unwrap_or_else(|_| candidate.local_path.clone());
    // The file may have vanished since classification: Java reads `exists() && length() > 0`.
    let len = tokio::fs::metadata(&local).await.map_or(0, |m| m.len());
    if len == 0 {
        return Ok(0.0);
    }
    tracing::info!("Determining file completion: {}", local.display());
    let remote = RemoteFile::new(
        format!("{}{ZSYNC_EXTENSION}", file_name(&candidate.relative_path)),
        parent_relative_path(&candidate.relative_path),
        false,
    );
    let path = remote.transport_path();
    let fetched = tokio::select! {
        () = ctx.cancel.cancelled() => return Err(CompletionError::Cancelled),
        res = connection.transport_mut().fetch(&path, ctx) => res,
    };
    let bytes = fetched.map_err(|e| {
        CompletionError::remote(RemoteError::fetch(connection.protocol(), &remote, e))
    })?;

    let stop = Arc::new(AtomicBool::new(false));
    let blocking_stop = Arc::clone(&stop);
    let blocking_local = local.clone();
    let mut handle = tokio::task::spawn_blocking(move || {
        let control = a3s_zsync::ControlFile::parse(&bytes)?;
        local_completion(&control, &blocking_local, Some(&blocking_stop))
    });
    let joined = tokio::select! {
        () = ctx.cancel.cancelled() => {
            stop.store(true, Ordering::Relaxed);
            // The scan polls the flag; wait for it so no thread outlives the call for long.
            (&mut handle).await.ok();
            return Err(CompletionError::Cancelled);
        }
        joined = &mut handle => joined,
    };
    let completed = match joined {
        Ok(completed) => completed,
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        // The runtime is shutting down.
        Err(_) => return Err(CompletionError::Cancelled),
    };
    let complete = match completed {
        Ok(complete) => complete,
        Err(a3s_zsync::Error::Cancelled) => return Err(CompletionError::Cancelled),
        // Mirrors dao/connection/http/HttpDAO.java:472-477
        Err(source) => {
            return Err(CompletionError::Zsync {
                message: format!(
                    "Failed to read zsync file: {}\n{source}",
                    remote.relative_file_path()
                ),
                relative_path: path,
                source,
            });
        }
    };
    // Java prints the value truncated to 2 decimals.
    // Mirrors dao/connection/http/HttpDAO.java:484-485
    tracing::info!(
        "File completion: {}% {}",
        a3s_format::java_double_to_string(((100.0 * complete) as i64) as f64 / 100.0),
        local.display()
    );
    Ok(complete)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use a3s_types::{SyncTreeDirectory, SyncTreeLeaf, SyncTreeNode};
    use a3s_zsync::{BLOCK_SIZE, BlockMatches, ControlFile};

    use super::*;
    use crate::plan::{NoAddons, assign_destinations};

    fn matches(map: &[bool]) -> BlockMatches {
        BlockMatches {
            local_offsets: map
                .iter()
                .enumerate()
                .map(|(i, m)| m.then_some(i as u64 * 8192))
                .collect(),
        }
    }

    #[test]
    fn a8_completion_percent() {
        assert!(completion_percent(&matches(&[]), 2) == 0.0);
        assert!(completion_percent(&matches(&[true, true, false, true]), 1) == 75.0);
        // The isolated first block is cleared.
        assert!(completion_percent(&matches(&[true, false, true, true]), 2) == 50.0);
        assert!(completion_percent(&matches(&[false, true, false, false]), 2) == 0.0);
        // The isolated last block is cleared.
        assert!(completion_percent(&matches(&[true, true, false, true]), 2) == 50.0);
        // Left to right on the updated map: clearing block 1 isolates nothing retroactively.
        assert!(completion_percent(&matches(&[true, true, false]), 2) == 2.0 / 3.0 * 100.0);
        assert!(completion_percent(&matches(&[true]), 2) == 100.0);
        assert!(completion_percent(&matches(&[false]), 2) == 0.0);
    }

    #[test]
    fn a8_completion_connections() {
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let info = |n| ServerInfo {
            number_of_connections: n,
            ..Default::default()
        };
        assert_eq!(completion_connections(&info(0)), 1);
        assert_eq!(completion_connections(&info(-3)), 1);
        assert_eq!(completion_connections(&info(10)), 10.min(cpus));
        assert_eq!(completion_connections(&info(1)), 1);
    }

    fn leaf(name: &str, updated: bool) -> SyncTreeNode {
        SyncTreeNode::Leaf(SyncTreeLeaf {
            name: name.to_owned(),
            sha1: Some("abc".to_owned()),
            size: 1,
            updated,
            ..Default::default()
        })
    }

    fn complete_of(tree: &PlanTree, path: &str) -> f64 {
        match tree.get(path) {
            Some(PlanNode::Leaf(l)) => l.complete,
            other => panic!("{path}: {other:?}"),
        }
    }

    #[test]
    fn a8_classify_leaves() {
        let base = tempfile::tempdir().expect("tempdir");
        let b = base.path();
        std::fs::create_dir_all(b.join("d")).expect("mkdir");
        std::fs::write(b.join("d/present"), b"x").expect("write");
        let mut root = SyncTreeDirectory::new_root();
        root.children = vec![SyncTreeNode::Directory(SyncTreeDirectory {
            name: "d".to_owned(),
            children: vec![
                leaf("absent", true),
                leaf("deleted", false),
                leaf("present", true),
                leaf("same", false),
            ],
            ..Default::default()
        })];
        let build = || {
            let mut tree = PlanTree::from_sync_tree(&root);
            assign_destinations(&mut tree, b, false, &NoAddons).expect("safe names");
            if let Some(PlanNode::Leaf(l)) = tree.get_mut("d/deleted") {
                l.deleted = true;
            }
            for path in ["d/absent", "d/present"] {
                if let Some(PlanNode::Leaf(l)) = tree.get_mut(path) {
                    l.complete = 42.0;
                }
            }
            tree
        };
        let info = ServerInfo::default();

        let mut tree = build();
        assert_eq!(classify_leaves(&mut tree, &info, true), ["d/present"]);
        assert!(complete_of(&tree, "d/absent") == 0.0);
        assert!(complete_of(&tree, "d/present") == 42.0, "untouched");
        assert!(complete_of(&tree, "d/same") == 100.0);
        assert!(complete_of(&tree, "d/deleted") == 100.0);

        let mut tree = build();
        assert!(classify_leaves(&mut tree, &info, false).is_empty());
        assert!(complete_of(&tree, "d/present") == 0.0);

        let mut tree = build();
        let compressed = ServerInfo {
            compressed_pbo_files_only: true,
            ..Default::default()
        };
        assert!(classify_leaves(&mut tree, &compressed, true).is_empty());
        assert!(complete_of(&tree, "d/present") == 0.0);
        assert!(complete_of(&tree, "d/same") == 100.0);
    }

    /// Non-periodic content, so that no block also appears elsewhere.
    fn pseudo_random(len: usize) -> Vec<u8> {
        let mut state: u32 = 0x1234_5678;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect()
    }

    #[test]
    fn a8_local_completion() {
        let content = pseudo_random(4 * BLOCK_SIZE);
        let control = ControlFile::generate(
            Cursor::new(&content),
            content.len() as u64,
            "x.pbo",
            "Sat, 10 Oct 2026 12:00:00 GMT",
            "http://h/x.pbo",
            "sha",
        )
        .expect("generate");
        let dir = tempfile::tempdir().expect("tempdir");
        let full = dir.path().join("full");
        std::fs::write(&full, &content).expect("write");
        assert!(local_completion(&control, &full, None).expect("match") == 100.0);

        let half = dir.path().join("half");
        let mut half_content = content[..2 * BLOCK_SIZE].to_vec();
        half_content.extend(std::iter::repeat_n(0xAB_u8, 2 * BLOCK_SIZE));
        std::fs::write(&half, &half_content).expect("write");
        assert!(local_completion(&control, &half, None).expect("match") == 50.0);

        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").expect("write");
        assert!(local_completion(&control, &empty, None).expect("match") == 0.0);

        let missing =
            local_completion(&control, &dir.path().join("missing"), None).expect_err("missing");
        assert!(matches!(missing, a3s_zsync::Error::Io(_)), "{missing:?}");
    }

    #[test]
    fn a8_error_kinds() {
        let err = CompletionError::Zsync {
            relative_path: "a/x.zsync".to_owned(),
            message: "m".to_owned(),
            source: a3s_zsync::Error::InvalidControlFile("bad".to_owned()),
        };
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(!err.is_connection_lost());
        let err = CompletionError::Zsync {
            relative_path: "a/x.zsync".to_owned(),
            message: "m".to_owned(),
            source: a3s_zsync::Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound)),
        };
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(CompletionError::Cancelled.kind(), ErrorKind::Cancelled);
        assert!(matches!(
            CompletionError::remote(RemoteError::Cancelled),
            CompletionError::Cancelled
        ));
    }
}
