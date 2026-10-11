//! A recording TCP relay in front of a server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use a3s_transport::CancellationToken;
use a3s_types::Protocol;
use parking_lot::Mutex;

use crate::AbortOnDrop;

/// A TCP relay in front of a server: counts concurrent connections, records what clients send
/// and can cut every connection.
pub struct Relay {
    pub port: u16,
    pub cut: CancellationToken,
    pub max_connections: Arc<AtomicUsize>,
    pub connections: Arc<AtomicUsize>,
    sent: Arc<Mutex<Vec<u8>>>,
    _task: AbortOnDrop,
}

impl Relay {
    pub async fn start(target_port: u16) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind relay");
        let port = listener.local_addr().expect("addr").port();
        let cut = CancellationToken::new();
        let max_connections = Arc::new(AtomicUsize::new(0));
        let connections = Arc::new(AtomicUsize::new(0));
        let sent: Arc<Mutex<Vec<u8>>> = Arc::default();
        let (task_cut, task_max, task_now, task_sent) = (
            cut.clone(),
            Arc::clone(&max_connections),
            Arc::clone(&connections),
            Arc::clone(&sent),
        );
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let cut = task_cut.clone();
                let max = Arc::clone(&task_max);
                let now = Arc::clone(&task_now);
                let sent = Arc::clone(&task_sent);
                tokio::spawn(async move {
                    let Ok(server) =
                        tokio::net::TcpStream::connect(("127.0.0.1", target_port)).await
                    else {
                        return;
                    };
                    let open = now.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(open, Ordering::SeqCst);
                    tokio::select! {
                        biased;
                        () = cut.cancelled() => {}
                        () = forward(client, server, sent) => {}
                    }
                    now.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Self {
            port,
            cut,
            max_connections,
            connections,
            sent,
            _task: AbortOnDrop(task),
        }
    }

    /// The lines clients sent so far (FTP control commands).
    pub fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.sent.lock())
            .split("\r\n")
            .map(str::to_owned)
            .collect()
    }

    /// Forgets the recorded lines.
    pub fn clear_lines(&self) {
        self.sent.lock().clear();
    }

    /// `protocol` with its port replaced by the relay's.
    pub fn protocol(&self, protocol: &Protocol) -> Protocol {
        let mut proxied = protocol.clone();
        proxied.port = self.port.to_string();
        proxied
    }
}

/// Copies both directions until both ends closed, recording what the client sends.
async fn forward(
    client: tokio::net::TcpStream,
    server: tokio::net::TcpStream,
    sent: Arc<Mutex<Vec<u8>>>,
) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    let upstream = async {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = client_read.read(&mut buf).await {
            if n == 0 || server_write.write_all(&buf[..n]).await.is_err() {
                break;
            }
            sent.lock().extend_from_slice(&buf[..n]);
        }
        let _ignored = server_write.shutdown().await;
    };
    let downstream = async {
        let _ignored = tokio::io::copy(&mut server_read, &mut client_write).await;
        let _closed = client_write.shutdown().await;
    };
    tokio::join!(upstream, downstream);
}
