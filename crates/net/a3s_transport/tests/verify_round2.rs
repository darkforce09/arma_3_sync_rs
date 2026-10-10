//! Adversarial verification of the WP06 review-round-1 code paths: known-directory chains
//! that vanish during an FTP session, the `LIST -a` fallback on servers without `MLST`,
//! FTP resume restarts when the remote size is unknown, and the SFTP configured base
//! (`""` = home, `"/"` = root, `"/x/"` = absolute).

#[allow(dead_code, unused_imports)]
#[path = "transport/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use a3s_transport::{Error, RemoteKind, TransferCtx, Transport, TransportOptions};
use a3s_types::{Protocol, ProtocolType};
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

use support::ftp::AbortOnDrop;
use support::strict_ftp::{self, Behaviour};
use support::{random_bytes, snapshot};

// ---------------------------------------------------------------------------------------
// FTP: known-directory chains that vanish during a session (B14, forget_chain)
// ---------------------------------------------------------------------------------------

/// Two known siblings below a vanished ancestor: both are recreated on their next upload.
#[tokio::test]
async fn verify_ftp_vanished_ancestor_recreates_both_siblings() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session.upload_bytes(b"a", "k/l/a.bin", &ctx).await.unwrap();
    session.upload_bytes(b"c", "k/x/c.bin", &ctx).await.unwrap();
    std::fs::remove_dir_all(fx.base.join("k")).unwrap();
    session.upload_bytes(b"b", "k/l/b.bin", &ctx).await.unwrap();
    session.upload_bytes(b"d", "k/x/d.bin", &ctx).await.unwrap();
    assert_eq!(std::fs::read(fx.base.join("k/l/b.bin")).unwrap(), b"b");
    assert_eq!(std::fs::read(fx.base.join("k/x/d.bin")).unwrap(), b"d");
    assert!(!fx.base.join("k/l/a.bin").exists());
    drop(session);
    t.close().await;
}

/// Deep chain, middle component removed: `p/q/r/s` known, `p/q` deleted externally.
#[tokio::test]
async fn verify_ftp_vanished_middle_of_deep_chain() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session
        .upload_bytes(b"1", "p/q/r/s/one.bin", &ctx)
        .await
        .unwrap();
    std::fs::remove_dir_all(fx.base.join("p/q")).unwrap();
    session
        .upload_bytes(b"2", "p/q/r/s/two.bin", &ctx)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(fx.base.join("p/q/r/s/two.bin")).unwrap(),
        b"2"
    );
    drop(session);
    t.close().await;
}

/// After a vanished chain is recreated, cached listings of the old directory are gone:
/// the old file is reported missing, the new one present.
#[tokio::test]
async fn verify_ftp_vanished_chain_drops_stale_listing() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    // Cache the listing of k/l, then add a.bin to it through the upload.
    assert!(!session.exists("k/l/zz.bin", &ctx).await.unwrap());
    session.upload_bytes(b"a", "k/l/a.bin", &ctx).await.unwrap();
    assert!(session.exists("k/l/a.bin", &ctx).await.unwrap());
    std::fs::remove_dir_all(fx.base.join("k")).unwrap();
    session.upload_bytes(b"b", "k/l/b.bin", &ctx).await.unwrap();
    assert!(
        !session.exists("k/l/a.bin", &ctx).await.unwrap(),
        "stale cached listing survived the recreated directory"
    );
    assert!(session.exists("k/l/b.bin", &ctx).await.unwrap());
    drop(session);
    t.close().await;
}

/// A known ancestor vanished and the upload goes into a not-yet-known child: Java's
/// `makeDir` walks every segment with `CWD` (`FtpDAO.java:641-658`), so `k` is recreated.
#[tokio::test]
async fn verify_ftp_vanished_known_ancestor_unknown_child() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session.mkdirs("k", &ctx).await.unwrap();
    std::fs::remove_dir_all(fx.base.join("k")).unwrap();
    let result = session.upload_bytes(b"n", "k/new/f.bin", &ctx).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(std::fs::read(fx.base.join("k/new/f.bin")).unwrap(), b"n");
    drop(session);
    t.close().await;
}

/// `mkdirs` of a known directory that vanished: Java `CWD`s into known directories and
/// recreates them when that fails (`FtpDAO.java:631-639`).
#[tokio::test]
async fn verify_ftp_mkdirs_known_dir_vanished() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session.mkdirs("z/y", &ctx).await.unwrap();
    std::fs::remove_dir_all(fx.base.join("z")).unwrap();
    session.mkdirs("z/y", &ctx).await.unwrap();
    assert!(
        fx.base.join("z/y").is_dir(),
        "mkdirs returned Ok but the directory is missing"
    );
    drop(session);
    t.close().await;
}

/// A known directory replaced by a file of the same name: the upload fails with an error
/// (no panic, no infinite recreate loop) and the transport stays usable.
#[tokio::test]
async fn verify_ftp_known_dir_replaced_by_file() {
    let fx = support::ftp::fixture(false).await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    session.upload_bytes(b"a", "k/l/a.bin", &ctx).await.unwrap();
    std::fs::remove_dir_all(fx.base.join("k/l")).unwrap();
    std::fs::write(fx.base.join("k/l"), b"now a file").unwrap();
    let result = session.upload_bytes(b"b", "k/l/b.bin", &ctx).await;
    assert!(result.is_err(), "{result:?}");
    assert!(!result.unwrap_err().is_connection_lost());
    session.upload_bytes(b"c", "k/c.bin", &ctx).await.unwrap();
    assert!(session.exists("k/c.bin", &ctx).await.unwrap());
    drop(session);
    t.close().await;
}

// ---------------------------------------------------------------------------------------
// FTP without MLST: scripted server over a tempdir (LIST -a fallback, B11/B12/B17)
// ---------------------------------------------------------------------------------------

/// How the stub answers `LIST -a`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DashA {
    /// Lists dot entries, `.` and `..`.
    Supported,

    /// Treats `-a` as a path: 550.
    Refused,
}

/// Behaviour of the MLST-less stub.
#[derive(Clone, Copy, Debug)]
struct NoMlst {
    dash_a: DashA,
}

struct NoMlstFtp {
    protocol: Protocol,
    root: PathBuf,
    commands: Arc<Mutex<Vec<String>>>,
    _outer: tempfile::TempDir,
    _task: AbortOnDrop,
}

impl NoMlstFtp {
    fn put(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    async fn connect(&self) -> Transport {
        let options = TransportOptions::from_protocol(&self.protocol);
        Transport::connect(&self.protocol, &options).await.unwrap()
    }

    fn list_commands(&self) -> Vec<String> {
        self.commands
            .lock()
            .iter()
            .filter(|c| c.to_ascii_uppercase().starts_with("LIST"))
            .cloned()
            .collect()
    }
}

async fn serve_no_mlst(behaviour: NoMlst) -> NoMlstFtp {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let commands: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = Arc::clone(&commands);
    let served = root.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let log = Arc::clone(&log);
            let root = served.clone();
            tokio::spawn(async move {
                let _ignored = no_mlst_session(stream, &root, behaviour, &log).await;
            });
        }
    });
    let protocol = Protocol::new(
        ProtocolType::Ftp,
        "127.0.0.1",
        port.to_string(),
        "anonymous",
        "",
        false,
    );
    NoMlstFtp {
        protocol,
        root,
        commands,
        _outer: outer,
        _task: AbortOnDrop(task),
    }
}

/// Virtual absolute path of `arg` relative to `cwd` (`None` for `..`).
fn resolve(cwd: &str, arg: &str) -> Option<String> {
    let joined = if arg.starts_with('/') {
        arg.to_owned()
    } else {
        format!("{cwd}/{arg}")
    };
    let mut parts = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => return None,
            s => parts.push(s),
        }
    }
    Some(format!("/{}", parts.join("/")))
}

fn real(root: &Path, virtual_path: &str) -> PathBuf {
    root.join(virtual_path.trim_start_matches('/'))
}

fn list_line(name: &str, is_dir: bool, size: u64) -> String {
    let kind = if is_dir { 'd' } else { '-' };
    format!("{kind}rw-r--r-- 1 ftp ftp {size} Jan 01 00:00 {name}")
}

fn listing(dir: &Path, all: bool) -> Vec<String> {
    let mut lines = vec!["total 0".to_owned()];
    if all {
        lines.push(list_line(".", true, 0));
        lines.push(list_line("..", true, 0));
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !all && name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata().unwrap();
        lines.push(list_line(&name, meta.is_dir(), meta.len()));
    }
    lines
}

async fn no_mlst_session(
    stream: TcpStream,
    root: &Path,
    behaviour: NoMlst,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"220 no-mlst stub\r\n").await?;
    let mut cwd = "/".to_owned();
    let mut passive: Option<TcpListener> = None;
    while let Some(line) = lines.next_line().await? {
        log.lock().push(line.clone());
        let (verb, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let verb = verb.to_ascii_uppercase();
        let reply = match verb.as_str() {
            "USER" => "331 password please".to_owned(),
            "PASS" => "230 logged in".to_owned(),
            "TYPE" => "200 type set".to_owned(),
            "PWD" => format!("257 \"{cwd}\" is the current directory"),
            "NOOP" => "200 ok".to_owned(),
            "MLST" | "MLSD" | "SIZE" => "500 unknown command".to_owned(),
            "CWD" => match resolve(&cwd, arg) {
                Some(v) if real(root, &v).is_dir() => {
                    cwd = v;
                    "250 directory changed".to_owned()
                }
                _ => "550 no such directory".to_owned(),
            },
            "MKD" => match resolve(&cwd, arg) {
                Some(v) if std::fs::create_dir(real(root, &v)).is_ok() => {
                    format!("257 \"{v}\" created")
                }
                _ => "550 cannot create".to_owned(),
            },
            "DELE" => match resolve(&cwd, arg) {
                Some(v) if real(root, &v).is_file() => {
                    std::fs::remove_file(real(root, &v))?;
                    "250 deleted".to_owned()
                }
                _ => "550 no such file".to_owned(),
            },
            "RMD" => match resolve(&cwd, arg) {
                Some(v) if std::fs::remove_dir(real(root, &v)).is_ok() => "250 removed".to_owned(),
                _ => "550 cannot remove".to_owned(),
            },
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                format!(
                    "227 Entering Passive Mode (127,0,0,1,{},{})",
                    port / 256,
                    port % 256
                )
            }
            "LIST" => {
                let (all, target) = match arg {
                    "" => (false, Some(cwd.clone())),
                    "-a" if behaviour.dash_a == DashA::Supported => (true, Some(cwd.clone())),
                    "-a" => (false, None),
                    // A path argument: like libunftp / Pure-FTPd, only the first word counts.
                    other => (
                        false,
                        resolve(&cwd, other.split(' ').next().unwrap_or_default()),
                    ),
                };
                match (passive.take(), target) {
                    (None, _) => "425 no data connection".to_owned(),
                    (Some(_), None) => "550 no such file or directory".to_owned(),
                    (Some(_), Some(v)) if !real(root, &v).is_dir() => {
                        "550 no such file or directory".to_owned()
                    }
                    (Some(listener), Some(v)) => {
                        write.write_all(b"150 listing\r\n").await?;
                        let (mut data, _) = listener.accept().await?;
                        let body: String = listing(&real(root, &v), all)
                            .into_iter()
                            .map(|l| format!("{l}\r\n"))
                            .collect();
                        data.write_all(body.as_bytes()).await?;
                        data.shutdown().await?;
                        drop(data);
                        "226 listing sent".to_owned()
                    }
                }
            }
            "QUIT" => {
                write.write_all(b"221 bye\r\n").await?;
                return Ok(());
            }
            _ => "502 not implemented".to_owned(),
        };
        write.write_all(format!("{reply}\r\n").as_bytes()).await?;
    }
    Ok(())
}

/// No path is ever sent as a `LIST` argument (amended B12): only `LIST` or `LIST -a`.
fn assert_no_list_path(commands: &[String]) {
    for c in commands {
        assert!(
            c == "LIST" || c == "LIST -a",
            "LIST with a path argument: {c:?} in {commands:?}"
        );
    }
}

/// B11 without MLST: outside a session `exists` looks the name up in the parent listing;
/// dot-files are found through `LIST -a`, names with spaces work, missing parents are false.
#[tokio::test]
async fn verify_ftp_no_mlst_exists_outside_session() {
    let fx = serve_no_mlst(NoMlst {
        dash_a: DashA::Supported,
    })
    .await;
    fx.put("d/f.bin", b"f");
    fx.put("d/.hidden", b"h");
    fx.put("m a/g h.bin", b"g");
    fx.put("top.bin", b"t");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    assert!(t.exists("d/f.bin", &ctx).await.unwrap());
    assert!(!t.exists("d/missing.bin", &ctx).await.unwrap());
    assert!(t.exists("d/.hidden", &ctx).await.unwrap());
    assert!(!t.exists("d/.nope", &ctx).await.unwrap());
    assert!(!t.exists("nodir/x.bin", &ctx).await.unwrap());
    assert!(t.exists("m a/g h.bin", &ctx).await.unwrap());
    assert!(t.exists("top.bin", &ctx).await.unwrap());
    assert_no_list_path(&fx.list_commands());
    // The connection is left usable and still at the base for relative-free commands.
    assert!(t.exists("d/f.bin", &ctx).await.unwrap());
    t.close().await;
}

/// B12 without MLST inside a session: one listing per directory; dot-files hidden from
/// `LIST` are confirmed through the fallback (`LIST -a`).
#[tokio::test]
async fn verify_ftp_no_mlst_session_dot_files() {
    let fx = serve_no_mlst(NoMlst {
        dash_a: DashA::Supported,
    })
    .await;
    fx.put("d/f.bin", b"f");
    fx.put("d/.hidden", b"h");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let mut session = t.upload_session();
    assert!(session.exists("d/f.bin", &ctx).await.unwrap());
    assert!(session.exists("d/.hidden", &ctx).await.unwrap());
    assert!(!session.exists("d/.nope", &ctx).await.unwrap());
    assert!(!session.exists("d/nope", &ctx).await.unwrap());
    assert_eq!(session.upload_session_stats().directory_listings, 1);
    assert!(!session.exists("gone/x", &ctx).await.unwrap());
    drop(session);
    assert_no_list_path(&fx.list_commands());
    t.close().await;
}

/// `LIST -a` refused (the server takes `-a` as a path): plain `LIST` is used instead and
/// regular files are still found; no error surfaces.
#[tokio::test]
async fn verify_ftp_no_mlst_dash_a_refused_falls_back() {
    let fx = serve_no_mlst(NoMlst {
        dash_a: DashA::Refused,
    })
    .await;
    fx.put("d/f.bin", b"f");
    fx.put("d/.hidden", b"h");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    assert!(t.exists("d/f.bin", &ctx).await.unwrap());
    assert!(!t.exists("d/missing", &ctx).await.unwrap());
    // Hidden from plain LIST: Java (plain listFiles) answers false as well.
    let hidden = t.exists("d/.hidden", &ctx).await;
    assert!(hidden.is_ok(), "{hidden:?}");
    let commands = fx.list_commands();
    assert!(commands.iter().any(|c| c == "LIST -a"), "{commands:?}");
    assert!(commands.iter().any(|c| c == "LIST"), "{commands:?}");
    assert_no_list_path(&commands);
    t.close().await;
}

/// B17 without MLST: recursive delete with `LIST -a` (which also returns `.` and `..`)
/// removes dot entries and does not recurse into `.`/`..`; missing targets are `Ok`.
#[tokio::test]
async fn verify_ftp_no_mlst_recursive_delete() {
    let fx = serve_no_mlst(NoMlst {
        dash_a: DashA::Supported,
    })
    .await;
    fx.put("keep.bin", b"k");
    fx.put("t r/.a3s/sync", b"s");
    fx.put("t r/.hidden", b"h");
    fx.put("t r/s/f.bin", b"f");
    std::fs::create_dir_all(fx.root.join("t r/s/empty")).unwrap();
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.delete("t r", RemoteKind::Directory, &ctx).await.unwrap();
    assert_eq!(
        snapshot(&fx.root),
        vec![("keep.bin".to_owned(), Some(b"k".to_vec()))]
    );
    t.delete("t r", RemoteKind::Directory, &ctx).await.unwrap();
    t.delete("no/such/file.bin", RemoteKind::File, &ctx)
        .await
        .unwrap();
    t.delete("missing.bin", RemoteKind::File, &ctx)
        .await
        .unwrap();
    assert!(t.exists("keep.bin", &ctx).await.unwrap());
    assert_no_list_path(&fx.list_commands());
    t.close().await;
}

/// Same recursive delete when `LIST -a` is refused: dot entries are invisible, so `RMD`
/// fails while the directory is still enterable → an error naming the path, not `Ok`.
#[tokio::test]
async fn verify_ftp_no_mlst_delete_hidden_left_is_error() {
    let fx = serve_no_mlst(NoMlst {
        dash_a: DashA::Refused,
    })
    .await;
    fx.put("t/.hidden", b"h");
    fx.put("t/f.bin", b"f");
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    let result = t.delete("t", RemoteKind::Directory, &ctx).await;
    assert!(result.is_err(), "{result:?}");
    assert!(!fx.root.join("t/f.bin").exists());
    // Still usable.
    assert!(!t.exists("t/f.bin", &ctx).await.unwrap());
    t.close().await;
}

// ---------------------------------------------------------------------------------------
// FTP resume with an unknown or inconsistent remote size
// ---------------------------------------------------------------------------------------

const STUB_PATH: &str = "/f.pbo";

async fn resume(behaviour: Behaviour, prefill: &[u8], offset: u64) -> (Vec<u8>, u64, Vec<String>) {
    let data = random_bytes(1033, 7);
    let stub = strict_ftp::serve(STUB_PATH, data.clone(), behaviour).await;
    let options = TransportOptions::from_protocol(&stub.protocol);
    let mut t = Transport::connect(&stub.protocol, &options).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f.pbo");
    std::fs::write(&dest, prefill).unwrap();
    let ctx = TransferCtx::new();
    let len = t.download_file("f.pbo", &dest, offset, &ctx).await.unwrap();
    assert_eq!(len, 1033);
    assert!(std::fs::read(&dest).unwrap() == data, "content differs");
    let progress = ctx.progress.load(Ordering::Relaxed);
    t.close().await;
    (data, progress, stub.commands.lock().clone())
}

/// Unknown size (no SIZE, no MLST), destination longer than the remote file and garbage
/// in it: the restart truncates and rewrites everything, progress counts the whole file.
#[tokio::test]
async fn verify_ftp_resume_unknown_size_overlong_dest() {
    let garbage = vec![0xAB; 2000];
    let (_, progress, commands) = resume(Behaviour::default(), &garbage, 516).await;
    assert_eq!(progress, 1033);
    assert!(
        !commands.iter().any(|c| c.starts_with("REST")),
        "{commands:?}"
    );
}

/// Known size but the offset is beyond it (local file longer than the remote one):
/// restart from 0, no REST.
#[tokio::test]
async fn verify_ftp_resume_offset_beyond_size() {
    let garbage = vec![0xCD; 1500];
    let (_, progress, commands) = resume(
        Behaviour {
            size: true,
            ..Behaviour::default()
        },
        &garbage,
        1500,
    )
    .await;
    assert_eq!(progress, 1033);
    assert!(
        !commands.iter().any(|c| c.starts_with("REST")),
        "{commands:?}"
    );
}

/// Known size (from MLST only), offset == size: nothing is transferred.
#[tokio::test]
async fn verify_ftp_resume_complete_via_mlst_size() {
    let data = random_bytes(1033, 7);
    let (_, progress, commands) = resume(
        Behaviour {
            mlst: true,
            ..Behaviour::default()
        },
        &data,
        1033,
    )
    .await;
    assert_eq!(progress, 0);
    assert!(
        !commands.iter().any(|c| c.starts_with("RETR")),
        "{commands:?}"
    );
}

/// Known size, REST honoured: progress counts only the remaining bytes.
#[tokio::test]
async fn verify_ftp_resume_progress_counts_remaining_only() {
    let data = random_bytes(1033, 7);
    let (_, progress, _) = resume(
        Behaviour {
            size: true,
            ..Behaviour::default()
        },
        &data[..100],
        100,
    )
    .await;
    assert_eq!(progress, 933);
}

// ---------------------------------------------------------------------------------------
// SFTP configured base: "" = home, "/" = root, "/x/" = absolute (B20, B21, B24)
// ---------------------------------------------------------------------------------------

const HOME: &str = "/home/a3s";

async fn sftp_upload_lands(url_path: &str, relative: &str) -> (support::Fixture, Vec<String>) {
    let fx = support::sftp::fixture_with(HOME, url_path, support::sftp::PASSWORD, "/").await;
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    t.upload_bytes(b"payload", relative, &ctx).await.unwrap();
    assert!(t.fetch(relative, &ctx).await.unwrap() == b"payload");
    assert!(t.exists(relative, &ctx).await.unwrap());
    t.close().await;
    let files = snapshot(&fx.root())
        .into_iter()
        .filter(|(_, content)| content.is_some())
        .map(|(path, _)| path)
        .collect();
    (fx, files)
}

#[tokio::test]
async fn verify_sftp_empty_path_is_home() {
    let (_fx, files) = sftp_upload_lands("", "x.bin").await;
    assert_eq!(files, vec!["home/a3s/x.bin".to_owned()]);
}

#[tokio::test]
async fn verify_sftp_slash_path_is_root() {
    let (_fx, files) = sftp_upload_lands("/", "d/x.bin").await;
    assert_eq!(files, vec!["d/x.bin".to_owned()]);
}

#[tokio::test]
async fn verify_sftp_trailing_slash_path_is_absolute() {
    let (_fx, files) = sftp_upload_lands("/x/", "x.bin").await;
    assert_eq!(files, vec!["x/x.bin".to_owned()]);
}

#[tokio::test]
async fn verify_sftp_double_slashes_and_dot_collapse() {
    let (_fx, files) = sftp_upload_lands("//x//./y/", "a/b.bin").await;
    assert_eq!(files, vec!["x/y/a/b.bin".to_owned()]);
}

/// Older builds stored `sftp:///host/path` (any case): same base as `host/path`.
#[tokio::test]
async fn verify_sftp_scheme_url_base() {
    let fx = support::sftp::fixture_with(HOME, "", support::sftp::PASSWORD, "/").await;
    let mut protocol = fx.protocol.clone();
    protocol.url = format!("SFTP:///{}/srv/repo/", protocol.url);
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    t.upload_bytes(b"s", ".a3s/sync", &ctx).await.unwrap();
    t.close().await;
    assert_eq!(
        std::fs::read(fx.root().join("srv/repo/.a3s/sync")).unwrap(),
        b"s"
    );
}

/// Downloads use the resolved base too: with an empty path a file in home is fetched,
/// the same name at the root is not.
#[tokio::test]
async fn verify_sftp_empty_path_download_from_home() {
    let fx = support::sftp::fixture_with(HOME, "", support::sftp::PASSWORD, "/").await;
    std::fs::write(fx.root().join("home/a3s/h.bin"), b"home").unwrap();
    std::fs::write(fx.root().join("r.bin"), b"root").unwrap();
    let mut t = fx.connect().await;
    let ctx = TransferCtx::new();
    assert!(t.fetch("h.bin", &ctx).await.unwrap() == b"home");
    assert!(!t.exists("r.bin", &ctx).await.unwrap());
    let missing = t.fetch("r.bin", &ctx).await;
    assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
    t.close().await;
}

/// `..` in the configured base is refused before connecting; a lone `.` is accepted
/// (Java only rejects `..`).
#[tokio::test]
async fn verify_sftp_base_validation() {
    let fx = support::sftp::fixture_with(HOME, "/a/../b", support::sftp::PASSWORD, "/").await;
    let options = TransportOptions::from_protocol(&fx.protocol);
    let result = Transport::connect(&fx.protocol, &options).await;
    assert!(
        matches!(result, Err(Error::InvalidPath(_))),
        "{:?}",
        result.err()
    );
    let (_fx, files) = sftp_upload_lands("/./", "x.bin").await;
    assert_eq!(files, vec!["x.bin".to_owned()]);
}

// ---------------------------------------------------------------------------------------
// Live, read-only: session `exists` (CWD + LIST) against WOG
// ---------------------------------------------------------------------------------------

/// Session `exists` against the WOG FTP repository (read-only): a known file, a missing
/// sibling in the same directory, a file in a missing directory and a metadata file; the
/// answers agree with `exists` outside the session (MLST) and one listing is cached per
/// directory.
#[tokio::test]
#[ignore = "network"]
async fn verify_live_wog_session_exists_cwd_list() {
    use a3s_types::{SyncTreeDirectory, SyncTreeNode};

    let protocol = Protocol::new(
        ProtocolType::Ftp,
        "88.99.69.243/Repo/WOG",
        "21",
        "anonymous",
        "",
        false,
    );
    let options = TransportOptions::from_protocol(&protocol);
    let mut t = Transport::connect(&protocol, &options).await.unwrap();
    let ctx = TransferCtx::new();
    let sync: SyncTreeDirectory =
        a3s_repo_files::decode_a3s(&t.fetch(".a3s/sync", &ctx).await.unwrap()).unwrap();
    let mut deepest: Option<String> = None;
    sync.walk(&mut |path, node| {
        if let SyncTreeNode::Leaf(_) = node
            && deepest
                .as_ref()
                .is_none_or(|d| path.matches('/').count() > d.matches('/').count())
        {
            deepest = Some(path.to_owned());
        }
    });
    let known = deepest.unwrap();
    let (dir, _) = known.rsplit_once('/').unwrap();
    let missing_sibling = format!("{dir}/a3s-verify-missing.pbo");
    let missing_dir = format!("{dir}/a3s-verify-missing-dir/x.pbo");
    let cases = [
        (known.as_str(), true),
        (missing_sibling.as_str(), false),
        (missing_dir.as_str(), false),
        (".a3s/serverinfo", true),
        (".a3s/a3s-verify-missing", false),
    ];
    for (path, expected) in cases {
        assert_eq!(t.exists(path, &ctx).await.unwrap(), expected, "MLST {path}");
    }
    {
        let mut session = t.upload_session();
        for (path, expected) in cases {
            assert_eq!(
                session.exists(path, &ctx).await.unwrap(),
                expected,
                "session {path}"
            );
        }
        // Listings: `dir`, `dir/a3s-verify-missing-dir` (missing) and `.a3s`.
        let stats = session.upload_session_stats();
        assert!(stats.directory_listings <= 3, "{stats:?}");
        assert!(stats.cached_file_checks >= 3, "{stats:?}");
        // Second round answers from the cache.
        let before = session.upload_session_stats().directory_listings;
        assert!(session.exists(&known, &ctx).await.unwrap());
        assert_eq!(session.upload_session_stats().directory_listings, before);
    }
    // The control connection still works for downloads.
    assert!(!t.fetch(".a3s/serverinfo", &ctx).await.unwrap().is_empty());
    t.close().await;
}
