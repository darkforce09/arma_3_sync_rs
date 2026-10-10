//! Minimal scripted FTP server that applies RFC 3659 §5.3 strictly: the restart marker set by
//! `REST` is cleared by any command other than the transfer command that follows it (like the
//! WOG repository server). Serves one in-memory file and logs every command.

use std::sync::Arc;
use std::time::Duration;

use a3s_types::{Protocol, ProtocolType};
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

use super::ftp::AbortOnDrop;

/// Behaviour switches of the stub.
#[derive(Clone, Copy, Debug, Default)]
pub struct Behaviour {
    /// Answer `SIZE` (otherwise 502).
    pub size: bool,

    /// Answer `MLST` with a `size=` fact (otherwise 502).
    pub mlst: bool,

    /// Accept `REST` with 350 but always send the whole file.
    pub ignore_rest: bool,

    /// Refuse the first `PASV` with 425.
    pub refuse_first_pasv: bool,

    /// Never send the completion reply (226) of a `RETR`.
    pub withhold_completion: bool,
}

pub struct StrictFtp {
    pub protocol: Protocol,
    pub commands: Arc<Mutex<Vec<String>>>,
    _task: AbortOnDrop,
}

pub async fn serve(path: &'static str, content: Vec<u8>, behaviour: Behaviour) -> StrictFtp {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind FTP stub");
    let port = listener.local_addr().expect("local addr").port();
    let commands: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = Arc::clone(&commands);
    let content = Arc::new(content);
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let log = Arc::clone(&log);
            let content = Arc::clone(&content);
            tokio::spawn(async move {
                let _ignored = session(stream, path, &content, behaviour, &log).await;
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
    StrictFtp {
        protocol,
        commands,
        _task: AbortOnDrop(task),
    }
}

async fn session(
    stream: TcpStream,
    path: &str,
    content: &[u8],
    behaviour: Behaviour,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"220 strict stub\r\n").await?;
    let mut restart: u64 = 0;
    let mut passive: Option<TcpListener> = None;
    let mut pasv_count = 0;
    while let Some(line) = lines.next_line().await? {
        log.lock().push(line.clone());
        let (verb, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let verb = verb.to_ascii_uppercase();
        if verb != "RETR" && verb != "REST" {
            // RFC 3659 §5.3: REST must immediately precede the transfer command.
            restart = 0;
        }
        let reply = match verb.as_str() {
            "USER" => "331 password please".to_owned(),
            "PASS" => "230 logged in".to_owned(),
            "TYPE" => "200 type set".to_owned(),
            "PWD" => "257 \"/\" is the current directory".to_owned(),
            "NOOP" => "200 ok".to_owned(),
            "SIZE" if behaviour.size && arg == path => format!("213 {}", content.len()),
            "SIZE" if behaviour.size => "550 no such file".to_owned(),
            "MLST" if behaviour.mlst && arg == path => format!(
                "250-Listing {path}\r\n type=file;size={}; {path}\r\n250 End",
                content.len()
            ),
            "MLST" if behaviour.mlst => "550 no such file".to_owned(),
            "REST" => match arg.parse() {
                Ok(offset) => {
                    restart = offset;
                    format!("350 restarting at {offset}")
                }
                Err(_) => "501 bad offset".to_owned(),
            },
            "PASV" if behaviour.refuse_first_pasv && pasv_count == 0 => {
                pasv_count += 1;
                "425 no passive port today".to_owned()
            }
            "PASV" => {
                pasv_count += 1;
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                format!(
                    "227 Entering Passive Mode (127,0,0,1,{},{})",
                    port / 256,
                    port % 256
                )
            }
            "RETR" => {
                let offset = if behaviour.ignore_rest { 0 } else { restart };
                restart = 0;
                match (passive.take(), arg == path) {
                    (None, _) => "425 no data connection".to_owned(),
                    (Some(_), false) => "550 no such file".to_owned(),
                    (Some(listener), true) => {
                        write.write_all(b"150 sending\r\n").await?;
                        let accepted =
                            tokio::time::timeout(Duration::from_secs(10), listener.accept()).await;
                        let Ok(Ok((mut data, _))) = accepted else {
                            write.write_all(b"425 no data connection\r\n").await?;
                            continue;
                        };
                        let start = usize::try_from(offset)
                            .unwrap_or(usize::MAX)
                            .min(content.len());
                        let sent = data.write_all(&content[start..]).await;
                        drop(data);
                        if behaviour.withhold_completion {
                            continue;
                        }
                        if sent.is_ok() {
                            "226 transfer complete".to_owned()
                        } else {
                            "426 transfer aborted".to_owned()
                        }
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
