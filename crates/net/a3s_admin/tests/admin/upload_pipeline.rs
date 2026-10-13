//! The upload pipeline against in-process FTP(S), SFTP and WebDAV servers.

use crate::net_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use a3s_admin::build::BuildOptions;
use a3s_admin::build::testing::RawGraph;
use a3s_admin::upload::{RoundTripOverrides, java_roundtrip_bytes};
use a3s_admin::{UploadError, UploadReport, UploadResume, build_repository, upload_repository};
use a3s_repo_files::A3sFile;
use a3s_repo_files::hash::Sha1Cache;
use a3s_sync::{EventSender, Outcome, Phase, SyncEvent};
use a3s_test_servers::tcp::Relay;
use a3s_transport::{CancellationToken, TransferCtx, TransportOptions};
use a3s_types::local::Repository;
use a3s_types::{Protocol, ProtocolType, ServerInfo};
use parking_lot::Mutex;

use net_support::{
    DavServer, Events, NOT_IN_SYNC, built_repo, copy_tree, data_files, gunzip, repository, servers,
    tree,
};

/// Directories and uploaded files of the `http_zip` sample with `.zsync` and `.zip` files.
const SAMPLE_DIRECTORIES: usize = 10;
const SAMPLE_FILES: usize = 23;

/// A writable copy of a Java-built sample repository.
fn local_copy(variant: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("local");
    copy_tree(&built_repo(variant, "1_initial"), &local);
    (dir, local)
}

/// The repository at `local`, uploaded to `upload`.
fn setup(download: ProtocolType, local: &Path, upload: &Protocol, compressed: bool) -> Repository {
    let mut repo = repository(download, Some(local));
    repo.upload_protocol = Some(upload.clone());
    repo.compressed = compressed;
    repo.excluded_files_from_build
        .insert(local.join("excluded.txt").to_string_lossy().into_owned());
    repo
}

async fn upload(
    repo: &Repository,
    resume: &mut UploadResume,
    ctx: &TransferCtx,
    events: &EventSender,
) -> Result<UploadReport, UploadError> {
    let options =
        TransportOptions::from_protocol(repo.upload_protocol.as_ref().unwrap_or(&repo.protocol));
    upload_repository(repo, &options, resume, ctx, events).await
}

async fn upload_ok(repo: &Repository, events: &Events) -> UploadReport {
    upload(
        repo,
        &mut UploadResume::new(),
        &TransferCtx::new(),
        &events.sender(),
    )
    .await
    .expect("upload")
}

fn rebuild(repo: &Repository) {
    build_repository(
        repo,
        &mut Sha1Cache::default(),
        &BuildOptions {
            build_date: Some(1_704_067_200_000),
        },
        &AtomicBool::new(false),
        &|_| {},
    )
    .expect("build");
}

/// The local data files expected on the server.
fn expected_files(local: &Path, exclude: impl Fn(&str) -> bool) -> BTreeMap<String, Vec<u8>> {
    data_files(local)
        .into_iter()
        .filter(|(path, _)| !NOT_IN_SYNC.contains(&path.as_str()) && !exclude(path))
        .collect()
}

/// The decompressed `.a3s` streams Java uploads for `local`.
fn expected_metadata(local: &Path, only_zip: bool) -> Vec<Vec<u8>> {
    A3sFile::ALL
        .into_iter()
        .map(|file| {
            let raw: RawGraph =
                a3s_repo_files::read_a3s(&local.join(".a3s").join(file.file_name())).expect("read");
            let overrides = RoundTripOverrides {
                compressed_pbo_files_only: (file == A3sFile::ServerInfo).then_some(only_zip),
            };
            gunzip(&java_roundtrip_bytes(file, &raw.graph, &overrides).expect("round trip"))
        })
        .collect()
}

fn server_metadata(base: &Path) -> Vec<Option<Vec<u8>>> {
    A3sFile::ALL
        .into_iter()
        .map(|file| {
            std::fs::read(base.join(".a3s").join(file.file_name()))
                .ok()
                .map(|b| gunzip(&b))
        })
        .collect()
}

fn mtimes(base: &Path) -> BTreeMap<String, std::time::SystemTime> {
    data_files(base)
        .into_keys()
        .map(|p| {
            let time = std::fs::metadata(base.join(&p))
                .and_then(|m| m.modified())
                .expect("mtime");
            (p, time)
        })
        .collect()
}

/// First upload to an empty server, then an upload without changes.
async fn first_and_second_upload(upload_protocol: Protocol, base: PathBuf) {
    let (_dir, local) = local_copy("http_zip");
    let repo = setup(ProtocolType::Http, &local, &upload_protocol, true);
    let events = Events::default();
    let report = upload_ok(&repo, &events).await;

    let expected = expected_files(&local, |_| false);
    assert_eq!(data_files(&base), expected);
    let total: u64 = expected.values().map(|c| c.len() as u64).sum();
    assert_eq!(
        report,
        UploadReport {
            checked: 0,
            missing: 0,
            uploaded_files: SAMPLE_FILES,
            uploaded_bytes: total,
            created_directories: SAMPLE_DIRECTORIES,
            skipped_by_resume: 0,
            deleted: 0,
            metadata_uploaded: true,
        }
    );
    assert_eq!(
        server_metadata(&base),
        expected_metadata(&local, false)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>()
    );
    let all = events.all();
    assert!(all.contains(&SyncEvent::TransferTotal {
        bytes: total,
        files: u32::try_from(SAMPLE_FILES).unwrap(),
    }));
    let last_transfer = all
        .iter()
        .rev()
        .find_map(|e| match e {
            SyncEvent::Transfer(t) => Some(t.clone()),
            _ => None,
        })
        .expect("transfer events");
    assert_eq!(
        (last_transfer.transferred, last_transfer.total),
        (total, total)
    );
    let phases: Vec<Phase> = all
        .iter()
        .filter_map(|e| match e {
            SyncEvent::Phase(p) => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        [
            Phase::CheckingRemoteFiles,
            Phase::Uploading,
            Phase::DeletingRemoteFiles
        ]
    );
    assert_eq!(all.last(), Some(&SyncEvent::Finished(Outcome::Success)));

    let before = mtimes(&base);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let report = upload_ok(&repo, &Events::default()).await;
    assert_eq!(
        report,
        UploadReport {
            checked: SAMPLE_FILES,
            metadata_uploaded: true,
            ..UploadReport::default()
        }
    );
    assert_eq!(mtimes(&base), before);
}

#[tokio::test]
async fn ftp_first_and_second_upload() {
    let fixture = servers::ftp::fixture(false).await;
    first_and_second_upload(fixture.protocol.clone(), fixture.base.clone()).await;
}

#[tokio::test]
async fn sftp_first_and_second_upload() {
    let fixture = servers::sftp::fixture().await;
    first_and_second_upload(fixture.protocol.clone(), fixture.base.clone()).await;
}

#[tokio::test]
async fn webdav_first_and_second_upload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let server = DavServer::serve(dir.path(), None).await;
    first_and_second_upload(server.protocol.clone(), dir.path().to_path_buf()).await;

    // Data files first, then the metadata in Java's order.
    let writes = server.writes();
    let first_metadata = writes
        .iter()
        .position(|w| w.starts_with("PUT /.a3s/"))
        .expect("metadata");
    assert!(
        writes[..first_metadata]
            .iter()
            .all(|w| !w.starts_with("PUT /.a3s/"))
    );
    let metadata: Vec<&str> = writes[first_metadata..first_metadata + 5]
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(
        metadata,
        [
            "PUT /.a3s/sync",
            "PUT /.a3s/serverinfo",
            "PUT /.a3s/changelogs",
            "PUT /.a3s/autoconfig",
            "PUT /.a3s/events"
        ]
    );
}

/// After a rebuild with one changed file and one deleted addon, only the changed file goes up
/// and the addon is deleted remotely, after the metadata.
async fn incremental_upload(upload_protocol: Protocol, base: &Path) {
    let (_dir, local) = local_copy("http_zip");
    let repo = setup(ProtocolType::Http, &local, &upload_protocol, true);
    upload_ok(&repo, &Events::default()).await;

    std::fs::write(local.join("@Mod_A/addons/mod_a.pbo"), vec![7u8; 21_000]).expect("modify");
    std::fs::remove_dir_all(local.join("@mod_b")).expect("delete addon");
    rebuild(&repo);
    let events = Events::default();
    let report = upload_ok(&repo, &events).await;
    assert_eq!(report.uploaded_files, 3, "{report:?}");
    assert_eq!(report.deleted, 1);
    assert_eq!(report.created_directories, 0);
    assert!(!base.join("@mod_b").exists());
    assert_eq!(
        std::fs::read(base.join("@Mod_A/addons/mod_a.pbo")).expect("uploaded"),
        vec![7u8; 21_000]
    );
    assert_eq!(
        events.count(|e| matches!(
            e,
            SyncEvent::Progress {
                phase: Phase::DeletingRemoteFiles,
                ..
            }
        )),
        1
    );
    let uploaded_sync = gunzip(&std::fs::read(base.join(".a3s/sync")).expect("sync"));
    assert_eq!(uploaded_sync, expected_metadata(&local, false)[0]);
}

#[tokio::test]
async fn ftp_incremental_upload() {
    let fixture = servers::ftp::fixture(false).await;
    incremental_upload(fixture.protocol.clone(), &fixture.base).await;
}

#[tokio::test]
async fn webdav_incremental_upload_orders_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let server = DavServer::serve(dir.path(), None).await;
    incremental_upload(server.protocol.clone(), dir.path()).await;
    let writes = server.writes();
    let last_metadata = writes
        .iter()
        .rposition(|w| w.starts_with("PUT /.a3s/"))
        .expect("metadata");
    let deletes: Vec<usize> = writes
        .iter()
        .enumerate()
        .filter(|(_, w)| w.starts_with("DELETE"))
        .map(|(i, _)| i)
        .collect();
    assert!(!deletes.is_empty());
    assert!(deletes.iter().all(|&i| i > last_metadata), "{writes:?}");
}

/// Connection 0 keeps one upload session for the whole run: after the existence check, a known
/// directory is entered once instead of each of its parents being checked again per file.
#[tokio::test]
async fn ftp_session_spans_the_whole_upload() {
    let fixture = servers::ftp::fixture(false).await;
    let relay = Relay::start(fixture.protocol.port.parse().expect("port")).await;
    let (_dir, local) = local_copy("http_zip");
    let mut repo = setup(
        ProtocolType::Http,
        &local,
        &relay.protocol(&fixture.protocol),
        true,
    );
    repo.parallel_upload_connections = 1;
    upload_ok(&repo, &Events::default()).await;

    std::fs::write(local.join("@Mod_A/addons/mod_a.pbo"), vec![7u8; 21_000]).expect("modify");
    rebuild(&repo);
    relay.clear_lines();
    let report = upload_ok(&repo, &Events::default()).await;
    assert_eq!(report.uploaded_files, 3);
    let lines = relay.lines();
    // `mod_a.pbo`, its `.zip` and `.zsync` go to `@Mod_A/addons`: once the first is stored the
    // directory is known, so the parent `@Mod_A` is not checked again.
    let first_store = lines
        .iter()
        .position(|l| l.as_str() == "STOR /@Mod_A/addons/mod_a.pbo")
        .expect("stored");
    assert!(
        lines[first_store..]
            .iter()
            .all(|l| l.as_str() != "CWD /@Mod_A"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn ftp_missing_files_are_repaired_and_only_zip_mode() {
    let fixture = servers::ftp::fixture(false).await;
    let (_dir, local) = local_copy("http_zip");
    let repo = setup(ProtocolType::Http, &local, &fixture.protocol, true);
    upload_ok(&repo, &Events::default()).await;
    std::fs::remove_file(fixture.base.join("@Mod_A/mod.cpp")).expect("remove");
    std::fs::remove_file(fixture.base.join("@Mod_A/mod.cpp.zsync")).expect("remove");
    let report = upload_ok(&repo, &Events::default()).await;
    assert_eq!((report.missing, report.uploaded_files), (2, 2));
    assert_eq!(data_files(&fixture.base), expected_files(&local, |_| false));

    // Only-zip mode: compressed leaves are only their `.zip`; the metadata is Java's upload.
    let fixture = servers::ftp::fixture(false).await;
    let mut repo = setup(ProtocolType::Http, &local, &fixture.protocol, true);
    repo.upload_compressed_pbo_files_only = true;
    upload_ok(&repo, &Events::default()).await;
    let raw_compressed = ["mod_a.pbo", "mod_b.ebo", "nested.pbo"];
    let expected = expected_files(&local, |path| {
        raw_compressed
            .iter()
            .any(|name| path.ends_with(name) || path.ends_with(&format!("{name}.zsync")))
    });
    assert_eq!(data_files(&fixture.base), expected);
    let java_uploaded = built_repo("http_zip", "1_initial").join("../uploaded");
    for (file, uploaded) in A3sFile::ALL.into_iter().zip(server_metadata(&fixture.base)) {
        let java = gunzip(&std::fs::read(java_uploaded.join(file.file_name())).expect("fixture"));
        assert_eq!(uploaded, Some(java), "{}", file.file_name());
    }
    let local_info: ServerInfo =
        a3s_repo_files::read_a3s(&local.join(".a3s/serverinfo")).expect("local");
    assert!(!local_info.compressed_pbo_files_only);

    // `.zsync` files only follow an HTTP download protocol.
    let fixture = servers::ftp::fixture(false).await;
    let repo = setup(ProtocolType::Ftp, &local, &fixture.protocol, true);
    upload_ok(&repo, &Events::default()).await;
    assert_eq!(
        data_files(&fixture.base),
        expected_files(&local, |path| path.ends_with(".zsync"))
    );
}

/// A repository of `files` files in `@many/addons`, built by the Rust builder.
fn many_files_repo(local: &Path, files: usize, upload_protocol: &Protocol) -> Repository {
    for i in 0..files {
        let path = local.join(format!("@many/addons/f{i:02}.pbo"));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, vec![u8::try_from(i).unwrap(); 4096 + i]).expect("write");
    }
    let repo = setup(ProtocolType::Ftp, local, upload_protocol, false);
    rebuild(&repo);
    repo
}

/// Events that also run `action` on every event.
fn events_with(
    events: &Events,
    action: impl Fn(&SyncEvent) + Send + Sync + 'static,
) -> EventSender {
    let sink = events.sender();
    EventSender::from_fn(move |event| {
        action(&event);
        sink.emit(event);
    })
}

fn is_file_start(event: &SyncEvent) -> Option<&str> {
    match event {
        SyncEvent::FileStarted { relative_path } if relative_path.ends_with(".pbo") => {
            Some(relative_path)
        }
        _ => None,
    }
}

async fn parallel_upload(server_protocol: &Protocol, base: &Path, files: usize, raw: i32) -> usize {
    let relay = Relay::start(server_protocol.port.parse().expect("port")).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut repo = many_files_repo(dir.path(), files, &relay.protocol(server_protocol));
    repo.parallel_upload_connections = raw;
    let events = Events::default();
    let report = upload_ok(&repo, &events).await;
    assert_eq!(report.uploaded_files, files);
    assert_eq!(data_files(base), data_files(dir.path()));
    assert_eq!(
        events.count(|e| matches!(e, SyncEvent::FileDone { .. })),
        files
    );
    let all = events.all();
    let last_dir = all
        .iter()
        .rposition(|e| matches!(e, SyncEvent::FileStarted { relative_path } if !relative_path.ends_with(".pbo")))
        .expect("directories");
    let first_file = all
        .iter()
        .position(|e| is_file_start(e).is_some())
        .expect("files");
    assert!(last_dir < first_file);
    relay.max_connections.load(Ordering::SeqCst)
}

#[tokio::test]
async fn ftp_and_sftp_upload_in_parallel() {
    let fixture = servers::ftp::fixture(false).await;
    let connections = parallel_upload(&fixture.protocol, &fixture.base, 40, 4).await;
    assert!((2..=4).contains(&connections), "{connections}");
    let fixture = servers::ftp::fixture(false).await;
    assert_eq!(
        parallel_upload(&fixture.protocol, &fixture.base, 3, 10).await,
        3
    );
    let fixture = servers::sftp::fixture().await;
    let connections = parallel_upload(&fixture.protocol, &fixture.base, 40, 4).await;
    assert!((2..=4).contains(&connections), "{connections}");
}

#[tokio::test]
async fn ftps_and_webdav_upload_sequentially() {
    let fixture = servers::ftp::fixture(true).await;
    assert_eq!(
        parallel_upload(&fixture.protocol, &fixture.base, 6, 4).await,
        1
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let server = DavServer::serve(dir.path(), None).await;
    let local = tempfile::tempdir().expect("tempdir");
    let mut repo = many_files_repo(local.path(), 6, &server.protocol);
    repo.parallel_upload_connections = 4;
    let events = Events::default();
    upload_ok(&repo, &events).await;
    // Each file is done before the next one starts.
    let mut open = None;
    for event in events.all() {
        match event {
            SyncEvent::FileStarted { relative_path } if relative_path.ends_with(".pbo") => {
                assert!(open.is_none());
                open = Some(relative_path);
            }
            SyncEvent::FileDone { relative_path } => {
                assert_eq!(open.take(), Some(relative_path));
            }
            _ => {}
        }
    }
}

/// The first failing worker stops the others; no metadata is uploaded.
#[tokio::test]
async fn parallel_failure_stops_every_worker() {
    let fixture = servers::ftp::fixture(false).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = many_files_repo(dir.path(), 40, &fixture.protocol);
    let victim = dir.path().join("@many/addons/f05.pbo");
    let events = Events::default();
    let sender = events_with(&events, move |event| {
        if is_file_start(event).is_some() {
            // Vanishes after the pre-check: the upload of this file fails.
            let _ignored = std::fs::remove_file(&victim);
        }
    });
    let started = Instant::now();
    let err = upload(
        &repo,
        &mut UploadResume::new(),
        &TransferCtx::new(),
        &sender,
    )
    .await
    .expect_err("file vanished");
    assert!(started.elapsed() < Duration::from_secs(5));
    match &err {
        UploadError::Transport { relative_path, .. } => {
            assert_eq!(relative_path, "@many/addons/f05.pbo");
        }
        other => panic!("{other:?}"),
    }
    assert!(!err.is_connection_lost());
    // Once f05 started, only the other three workers' files already in flight may start.
    let all = events.all();
    let failing = all
        .iter()
        .position(|e| is_file_start(e) == Some("@many/addons/f05.pbo"))
        .expect("f05 started");
    let started_after = all[failing + 1..]
        .iter()
        .filter(|e| is_file_start(e).is_some())
        .count();
    assert!(started_after <= 3, "{started_after}");
    assert!(!fixture.base.join(".a3s").exists());
    assert!(matches!(
        events.all().last(),
        Some(SyncEvent::Finished(Outcome::Failed { .. }))
    ));
}

/// Sequential upload with `@many/addons/f04.pbo` vanishing during the upload.
async fn failing_attempt(repo: &Repository, resume: &mut UploadResume, local: &Path) -> Vec<u8> {
    let victim = local.join("@many/addons/f04.pbo");
    let content = std::fs::read(&victim).expect("read");
    let events = Events::default();
    let sender = events_with(&events, move |event| {
        if is_file_start(event).is_some() {
            let _ignored = std::fs::remove_file(&victim);
        }
    });
    upload(repo, resume, &TransferCtx::new(), &sender)
        .await
        .expect_err("file vanished");
    let last_index = events
        .all()
        .iter()
        .rev()
        .find_map(|e| match e {
            SyncEvent::LastIndexTransferred { index } => Some(*index),
            _ => None,
        })
        .expect("index");
    assert_eq!(last_index as usize, resume.completed());
    content
}

#[tokio::test]
async fn resume_skips_files_of_the_failed_attempt() {
    let fixture = servers::ftp::fixture(false).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path();
    let mut repo = many_files_repo(local, 10, &fixture.protocol);
    repo.parallel_upload_connections = 1;
    let victim = local.join("@many/addons/f04.pbo");

    // Two directories and f00..f03 were uploaded before f04 failed.
    let mut resume = UploadResume::new();
    let content = failing_attempt(&repo, &mut resume, local).await;
    assert_eq!(resume.completed(), 6);
    std::fs::write(&victim, &content).expect("restore");
    let report = upload(
        &repo,
        &mut resume,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect("resumed");
    assert_eq!((report.skipped_by_resume, report.uploaded_files), (6, 6));
    assert!(resume.is_empty());

    // A file touched since the failed attempt is uploaded again.
    let fixture = servers::ftp::fixture(false).await;
    repo.upload_protocol = Some(fixture.protocol.clone());
    let mut resume = UploadResume::new();
    let content = failing_attempt(&repo, &mut resume, local).await;
    std::fs::write(&victim, &content).expect("restore");
    let touched = std::fs::File::options()
        .write(true)
        .open(local.join("@many/addons/f01.pbo"))
        .expect("open");
    touched
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
        .expect("touch");
    drop(touched);
    let report = upload(
        &repo,
        &mut resume,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect("resumed");
    assert_eq!((report.skipped_by_resume, report.uploaded_files), (5, 7));

    // A record for one target is not used for another.
    let fixture = servers::ftp::fixture(false).await;
    repo.upload_protocol = Some(fixture.protocol.clone());
    let mut resume = UploadResume::new();
    let content = failing_attempt(&repo, &mut resume, local).await;
    std::fs::write(&victim, &content).expect("restore");
    let other = servers::ftp::fixture(false).await;
    repo.upload_protocol = Some(other.protocol.clone());
    let report = upload(
        &repo,
        &mut resume,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect("other target");
    assert_eq!((report.skipped_by_resume, report.uploaded_files), (0, 10));
    assert_eq!(data_files(&other.base).len(), 10);

    // A rebuild between attempts discards the resume state.
    let fixture = servers::ftp::fixture(false).await;
    repo.upload_protocol = Some(fixture.protocol.clone());
    let mut resume = UploadResume::new();
    let content = failing_attempt(&repo, &mut resume, local).await;
    std::fs::write(&victim, &content).expect("restore");
    std::fs::write(local.join("@many/addons/f09.pbo"), b"changed").expect("modify");
    rebuild(&repo);
    let report = upload(
        &repo,
        &mut resume,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect("fresh");
    assert_eq!((report.skipped_by_resume, report.uploaded_files), (0, 10));
}

#[tokio::test]
async fn connection_lost_keeps_resume_and_metadata() {
    let fixture = servers::ftp::fixture(false).await;
    let relay = Relay::start(fixture.protocol.port.parse().expect("port")).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut repo = many_files_repo(dir.path(), 4, &relay.protocol(&fixture.protocol));
    repo.parallel_upload_connections = 1;
    let events = Events::default();
    let cut = relay.cut.clone();
    let sender = events_with(&events, move |event| {
        if is_file_start(event).is_some() {
            cut.cancel();
        }
    });
    let mut resume = UploadResume::new();
    let err = upload(&repo, &mut resume, &TransferCtx::new(), &sender)
        .await
        .expect_err("connection cut");
    assert!(err.is_connection_lost(), "{err:?}");
    let all = events.all();
    assert_eq!(all[all.len() - 2], SyncEvent::ConnectionLost);
    assert!(matches!(
        all.last(),
        Some(SyncEvent::Finished(Outcome::Failed { .. }))
    ));
    assert!(!fixture.base.join(".a3s").exists());
    // The two directories were created before the cut.
    assert_eq!(resume.completed(), 2);
}

/// Cancelling during the file phase uploads no metadata and deletes nothing.
async fn cancel_during_files(upload_protocol: Protocol, base: PathBuf) {
    let (_dir, local) = local_copy("http_zip");
    let repo = setup(ProtocolType::Http, &local, &upload_protocol, true);
    upload_ok(&repo, &Events::default()).await;
    let metadata = server_metadata(&base);
    std::fs::write(local.join("@Mod_A/addons/mod_a.pbo"), vec![7u8; 4 << 20]).expect("modify");
    std::fs::remove_dir_all(local.join("@mod_b")).expect("delete addon");
    rebuild(&repo);

    let ctx = TransferCtx::new();
    let cancel = ctx.cancel.clone();
    let events = Events::default();
    let sender = events_with(&events, move |event| {
        if is_file_start(event).is_some() {
            cancel.cancel();
        }
    });
    let started = Instant::now();
    let err = upload(&repo, &mut UploadResume::new(), &ctx, &sender)
        .await
        .expect_err("cancelled");
    assert!(matches!(err, UploadError::Cancelled), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        events.all().last(),
        Some(&SyncEvent::Finished(Outcome::Cancelled))
    );
    assert_eq!(server_metadata(&base), metadata);
    assert!(base.join("@mod_b").exists());
}

#[tokio::test]
async fn ftp_cancel_during_files() {
    let fixture = servers::ftp::fixture(false).await;
    cancel_during_files(fixture.protocol.clone(), fixture.base.clone()).await;
}

#[tokio::test]
async fn sftp_cancel_during_files() {
    let fixture = servers::sftp::fixture().await;
    cancel_during_files(fixture.protocol.clone(), fixture.base.clone()).await;
}

#[tokio::test]
async fn webdav_cancel_during_files_and_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let server = DavServer::serve(dir.path(), None).await;
    cancel_during_files(server.protocol.clone(), dir.path().to_path_buf()).await;

    // A cancel once the metadata upload started: all five files are still written.
    let dir = tempfile::tempdir().expect("tempdir");
    let token: Arc<Mutex<Option<CancellationToken>>> = Arc::default();
    let hook_token = Arc::clone(&token);
    let server = DavServer::serve(
        dir.path(),
        Some(Arc::new(move |method: &str, path: &str| {
            if method == "PUT"
                && path == "/.a3s/sync"
                && let Some(token) = &*hook_token.lock()
            {
                token.cancel();
            }
        })),
    )
    .await;
    let (_local_dir, local) = local_copy("http_zip");
    let repo = setup(ProtocolType::Http, &local, &server.protocol, true);
    let ctx = TransferCtx::new();
    *token.lock() = Some(ctx.cancel.clone());
    let err = upload(&repo, &mut UploadResume::new(), &ctx, &EventSender::noop())
        .await
        .expect_err("cancelled before deleting");
    assert!(matches!(err, UploadError::Cancelled), "{err:?}");
    assert_eq!(
        server_metadata(dir.path()),
        expected_metadata(&local, false)
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn preconditions_fail_before_any_transfer() {
    let fixture = servers::ftp::fixture(false).await;
    let (_dir, local) = local_copy("http_zip");
    let no_events = EventSender::noop;
    let run = |repo: Repository| async move {
        upload(
            &repo,
            &mut UploadResume::new(),
            &TransferCtx::new(),
            &no_events(),
        )
        .await
    };

    let mut repo = setup(ProtocolType::Http, &local, &fixture.protocol, true);
    repo.upload_protocol = None;
    assert!(matches!(
        run(repo).await,
        Err(UploadError::UploadProtocolNotSet)
    ));
    let mut repo = setup(ProtocolType::Http, &local, &fixture.protocol, true);
    repo.path = Some(String::new());
    assert!(matches!(run(repo).await, Err(UploadError::PathNotSet)));

    let repo = setup(ProtocolType::Http, &local, &fixture.protocol, true);
    let changelogs = local.join(".a3s/changelogs");
    let saved = std::fs::read(&changelogs).expect("read");
    std::fs::remove_file(&changelogs).expect("remove");
    assert!(matches!(
        run(repo.clone()).await,
        Err(UploadError::ReadLocal {
            file: A3sFile::Changelogs,
            ..
        })
    ));
    std::fs::write(&changelogs, b"not gzip").expect("corrupt");
    assert!(matches!(
        run(repo.clone()).await,
        Err(UploadError::ReadLocal {
            file: A3sFile::Changelogs,
            source: a3s_repo_files::Error::Gzip(_),
            ..
        })
    ));
    std::fs::write(&changelogs, saved).expect("restore");

    let zip = local.join("@mod_b/Addons/mod_b.ebo.zip");
    std::fs::remove_file(&zip).expect("remove");
    match run(repo.clone()).await {
        Err(UploadError::LocalFileMissing { path }) => {
            assert!(path.is_absolute());
            assert!(path.ends_with("mod_b.ebo.zip"));
        }
        other => panic!("{other:?}"),
    }
    assert!(tree(&fixture.base).is_empty());

    let http = net_support::HttpServer::serve(&local).await;
    let repo = setup(ProtocolType::Http, &local, &http.protocol, true);
    assert!(matches!(
        run(repo).await,
        Err(UploadError::Transport {
            source: a3s_transport::Error::Unsupported { .. },
            ..
        })
    ));
}

/// A file below a folder named `racine` lives at the repository root on the server, as with
/// Java: the upload puts it there, the content check expects it there, and a client downloads it
/// from there back to its full local path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nested_racine_folder_round_trip() {
    let fixture = servers::ftp::fixture(false).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let local = dir.path().join("local");
    std::fs::create_dir_all(local.join("@a/racine/sub")).expect("mkdir");
    std::fs::create_dir_all(local.join("@a/addons")).expect("mkdir");
    std::fs::write(local.join("@a/racine/x.pbo"), b"nested x").expect("write");
    std::fs::write(local.join("@a/racine/sub/y.pbo"), b"nested y").expect("write");
    std::fs::write(local.join("@a/addons/a.pbo"), b"plain").expect("write");
    let repo = setup(ProtocolType::Ftp, &local, &fixture.protocol, false);
    rebuild(&repo);

    let report = upload_ok(&repo, &Events::default()).await;
    assert_eq!(report.uploaded_files, 3);
    let served = data_files(&fixture.base);
    let served: Vec<&str> = served.keys().map(String::as_str).collect();
    assert_eq!(served, ["@a/addons/a.pbo", "sub/y.pbo", "x.pbo"]);

    // Nothing missing for the content check, nothing to do for a second upload.
    let mut checked = repo.clone();
    checked.protocol = fixture.protocol.clone();
    let options = TransportOptions::from_protocol(&fixture.protocol);
    let content = a3s_admin::check_repository_content(
        &checked,
        &options,
        &TransferCtx::new(),
        &EventSender::noop(),
    )
    .await
    .expect("check");
    assert_eq!((content.checked, content.missing.len()), (3, 0));
    let again = upload_ok(&repo, &Events::default()).await;
    assert_eq!((again.uploaded_files, again.deleted), (0, 0));

    // A client synchronizes the server into an empty folder: full local paths.
    let dest = dir.path().join("dest");
    std::fs::create_dir_all(&dest).expect("mkdir");
    let mut client = a3s_types::local::Repository::new("client", fixture.protocol.clone());
    client.default_download_location = Some(dest.to_string_lossy().into_owned());
    let store = Arc::new(a3s_sync::MemoryStore::new([client]));
    let job = a3s_sync::SyncJob {
        check: a3s_sync::CheckRequest {
            repository_name: "client".to_owned(),
            event_name: None,
            protocol: fixture.protocol.clone(),
            addons: Arc::new(a3s_sync::plan::NoAddons),
            removed_files: a3s_sync::cleanup::RemovedFiles::Delete,
        },
        arma3_exe_path: None,
        userconfig: a3s_sync::pipeline::UserconfigPolicy::Silent,
        reconnect: a3s_sync::pipeline::ReconnectPolicy::NONE,
    };
    let synced =
        a3s_sync::sync_repository(job, store, &CancellationToken::new(), &EventSender::noop())
            .await;
    assert_eq!(
        synced.end,
        a3s_sync::SyncEnd::Finished(Outcome::Success),
        "{synced:?}"
    );
    assert_eq!(
        data_files(&dest),
        BTreeMap::from([
            ("@a/addons/a.pbo".to_owned(), b"plain".to_vec()),
            ("@a/racine/sub/y.pbo".to_owned(), b"nested y".to_vec()),
            ("@a/racine/x.pbo".to_owned(), b"nested x".to_vec()),
        ])
    );
}
