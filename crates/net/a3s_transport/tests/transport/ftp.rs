//! FTP and FTPS against libunftp, plus resume against the strict stub.

use a3s_transport::{Error, RemoteKind, TransferCtx, Transport, TransportOptions};

use crate::common;
use crate::support::ftp::{fixture, fixture_with_path};
use crate::support::random_bytes;
use crate::support::strict_ftp::{self, Behaviour};

macro_rules! each_ftp {
    ($($name:ident => $check:expr;)*) => {
        $(
            mod $name {
                use super::*;

                #[tokio::test]
                async fn ftp() {
                    let fx = fixture(false).await;
                    $check(&fx).await;
                }

                #[tokio::test]
                async fn ftps() {
                    let fx = fixture(true).await;
                    $check(&fx).await;
                }
            }
        )*
    };
}

each_ftp! {
    a3_upload_file_round_trip => common::a3_upload_file_round_trip;
    a4_upload_bytes_identity => common::a4_upload_bytes_identity;
    a5_exists => common::a5_exists;
    a6_mkdirs_idempotent => common::a6_mkdirs_idempotent;
    a7_delete => common::a7_delete;
    a8_path_traversal => common::a8_path_traversal;
    a_resume_ftp => common::resume_download;
}

#[tokio::test]
async fn a17_cancellation_ftp() {
    common::a17_cancellation(&fixture(false).await, true).await;
}

#[tokio::test]
async fn a17_cancellation_ftps() {
    common::a17_cancellation(&fixture(true).await, true).await;
}

#[tokio::test]
async fn a9_session_cache() {
    let fx = fixture(false).await;
    for i in 0..50 {
        fx.put(&format!("d/f{i}.pbo"), b"x");
    }
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    {
        let mut session = t.upload_session();
        for i in 0..50 {
            assert!(session.exists(&format!("d/f{i}.pbo"), &ctx).await.unwrap());
        }
        let stats = session.upload_session_stats();
        assert_eq!(stats.directory_listings, 1);
        assert_eq!(stats.cached_file_checks, 50);

        session
            .upload_bytes(b"new", "d/new.pbo", &ctx)
            .await
            .unwrap();
        assert!(session.exists("d/new.pbo", &ctx).await.unwrap());
        // The file is really there, not only in the cache.
        assert!(fx.base.join("d/new.pbo").is_file());
        session
            .delete("d/new.pbo", RemoteKind::File, &ctx)
            .await
            .unwrap();
        assert!(!session.exists("d/new.pbo", &ctx).await.unwrap());
        assert_eq!(session.upload_session_stats().directory_listings, 1);
    }
    let stats = t.upload_session_stats();
    // Outside the session `exists` uses MLST and does not touch the counters.
    fx.put("d/late.pbo", b"x");
    assert!(t.exists("d/late.pbo", &ctx).await.unwrap());
    assert_eq!(t.upload_session_stats(), stats);
    t.close().await;
}

#[tokio::test]
async fn a11_base_directory() {
    // Configured remote path that exists: files land below it.
    let fx = fixture_with_path(false, "/repo").await;
    std::fs::create_dir_all(&fx.base).unwrap();
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"in repo", "a/x.bin", &ctx).await.unwrap();
    assert_eq!(
        std::fs::read(fx.root().join("repo/a/x.bin")).unwrap(),
        b"in repo"
    );
    assert_eq!(t.fetch("a/x.bin", &ctx).await.unwrap(), b"in repo");
    t.close().await;

    // Missing remote path: connect fails.
    let fx = fixture_with_path(false, "/missing").await;
    let options = TransportOptions::from_protocol(&fx.protocol);
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(
        matches!(result, Err(Error::NotFound(ref p)) if p == "/missing"),
        "{:?}",
        result.err()
    );
    assert!(!fx.root().join("missing").exists());

    // Empty remote path: base is the login directory.
    let fx = fixture(false).await;
    let mut t = fx.connect().await;
    t.upload_bytes(b"top", "a/x.bin", &ctx).await.unwrap();
    assert_eq!(std::fs::read(fx.root().join("a/x.bin")).unwrap(), b"top");
    t.close().await;
}

#[tokio::test]
async fn a18_parallel_uploads_into_same_tree() {
    let fx = fixture(false).await;
    let mut tasks = Vec::new();
    for worker in 0..2 {
        let mut t = fx.connect().await;
        tasks.push(tokio::spawn(async move {
            let ctx = TransferCtx::new();
            let mut session = t.upload_session();
            for i in 0..20 {
                let path = format!("shared/a/b/c{}/w{worker}_{i}.bin", i % 3);
                session
                    .upload_bytes(&random_bytes(100, i), &path, &ctx)
                    .await
                    .unwrap();
            }
            drop(session);
            t.close().await;
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    for worker in 0..2 {
        for i in 0..20 {
            let path = fx
                .base
                .join(format!("shared/a/b/c{}/w{worker}_{i}.bin", i % 3));
            assert!(path.is_file(), "{path:?}");
        }
    }
}

#[tokio::test]
async fn a16_ftp_supports_upload() {
    let fx = fixture(false).await;
    let t = fx.connect().await;
    assert!(t.supports_upload());
    t.close().await;
}

const STUB_PATH: &str = "/f.pbo";

async fn resume_against_stub(behaviour: Behaviour) -> Vec<String> {
    let data = random_bytes(1033, 99);
    let stub = strict_ftp::serve(STUB_PATH, data.clone(), behaviour).await;
    let options = TransportOptions::from_protocol(&stub.protocol);
    let mut t = Transport::connect(&stub.protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.pbo");
    std::fs::write(&dest, &data[..516]).unwrap();
    let len = t.download_file("f.pbo", &dest, 516, &ctx).await.unwrap();
    assert_eq!(len, 1033);
    assert!(std::fs::read(&dest).unwrap() == data);
    // The transport is still usable.
    assert!(t.fetch("f.pbo", &ctx).await.unwrap() == data);
    t.close().await;
    stub.commands.lock().clone()
}

/// The WOG server bug: a server that clears the restart marker on `PASV` received
/// `REST, PASV, RETR` and sent the whole file, which was appended to the first half.
#[tokio::test]
async fn a_resume_ftp_rest_immediately_before_retr() {
    for behaviour in [
        Behaviour {
            size: true,
            ..Behaviour::default()
        },
        // Size from the MLST `size=` fact when SIZE is not implemented.
        Behaviour {
            mlst: true,
            ..Behaviour::default()
        },
    ] {
        let commands = resume_against_stub(behaviour).await;
        let rest = commands
            .iter()
            .position(|c| c == "REST 516")
            .expect("REST sent");
        assert_eq!(commands[rest - 1], "PASV", "{commands:?}");
        assert_eq!(
            commands[rest + 1],
            format!("RETR {STUB_PATH}"),
            "{commands:?}"
        );
    }
}

/// Without a known remote size a resume cannot be checked: the download restarts from 0
/// (no REST), even against a server that would ignore REST.
#[tokio::test]
async fn a_resume_ftp_unknown_size_restarts() {
    let commands = resume_against_stub(Behaviour {
        ignore_rest: true,
        ..Behaviour::default()
    })
    .await;
    assert!(
        !commands.iter().any(|c| c.starts_with("REST")),
        "{commands:?}"
    );
}

/// A server that acknowledges `REST` but sends the whole file anyway is detected through
/// `SIZE`; the download restarts from 0 instead of appending.
#[tokio::test]
async fn a_resume_ftp_server_ignoring_rest() {
    let commands = resume_against_stub(Behaviour {
        size: true,
        ignore_rest: true,
        ..Behaviour::default()
    })
    .await;
    let retrs = commands.iter().filter(|c| c.starts_with("RETR")).count();
    // Resume attempt, restart from 0, then the final fetch.
    assert_eq!(retrs, 3, "{commands:?}");
}

/// A refused `PASV` during a resume falls back to a full download.
#[tokio::test]
async fn a_resume_ftp_refused_pasv_restarts() {
    let commands = resume_against_stub(Behaviour {
        size: true,
        refuse_first_pasv: true,
        ..Behaviour::default()
    })
    .await;
    assert!(
        !commands.iter().any(|c| c.starts_with("REST")),
        "{commands:?}"
    );
}

/// A missing file: 550 on `SIZE` is "size unknown", the download then reports `NotFound`.
#[tokio::test]
async fn a_resume_ftp_missing_file() {
    let stub = strict_ftp::serve(
        STUB_PATH,
        random_bytes(10, 1),
        Behaviour {
            size: true,
            ..Behaviour::default()
        },
    )
    .await;
    let options = TransportOptions::from_protocol(&stub.protocol);
    let mut t = Transport::connect(&stub.protocol, &options).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("x");
    std::fs::write(&dest, b"half").unwrap();
    let result = t
        .download_file("other.pbo", &dest, 4, &TransferCtx::new())
        .await;
    assert!(matches!(result, Err(Error::NotFound(_))), "{result:?}");
    // Still usable.
    assert_eq!(
        t.fetch("f.pbo", &TransferCtx::new()).await.unwrap().len(),
        10
    );
}

/// A completion reply that never arrives times out and poisons the transport.
#[tokio::test]
async fn a_ftp_missing_completion_reply_poisons() {
    let stub = strict_ftp::serve(
        STUB_PATH,
        random_bytes(10, 1),
        Behaviour {
            withhold_completion: true,
            ..Behaviour::default()
        },
    )
    .await;
    let mut options = TransportOptions::from_protocol(&stub.protocol);
    options.read_timeout = Some(std::time::Duration::from_millis(300));
    let mut t = Transport::connect(&stub.protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    let result = t.fetch("f.pbo", &ctx).await;
    assert!(matches!(result, Err(Error::Timeout)), "{result:?}");
    let result = t.exists("f.pbo", &ctx).await;
    assert!(
        matches!(result, Err(ref e @ Error::ConnectionPoisoned) if e.is_connection_lost()),
        "{result:?}"
    );
}
