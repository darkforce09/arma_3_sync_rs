//! A SOCKS4/SOCKS4a and SOCKS5 proxy (`CONNECT` only) that records every tunnel it opens.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use crate::AbortOnDrop;

/// One tunnel opened by [`SocksProxy`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tunnel {
    /// SOCKS version the client spoke (4 or 5).
    pub version: u8,

    /// SOCKS4 user id or SOCKS5 user name; `None` when the client sent none.
    pub user: Option<String>,

    /// The target, as the client sent it (`host:port` for a domain name).
    pub target: String,
}

/// A running SOCKS proxy on `127.0.0.1:<port>`.
pub struct SocksProxy {
    pub port: u16,
    tunnels: Arc<Mutex<Vec<Tunnel>>>,
    _task: AbortOnDrop,
}

impl SocksProxy {
    /// Starts the proxy. With `credentials`, SOCKS5 clients must authenticate with that user
    /// name and password (RFC 1929); SOCKS4 clients are always accepted.
    pub async fn start(credentials: Option<(&str, &str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind SOCKS proxy");
        let port = listener.local_addr().expect("addr").port();
        let tunnels: Arc<Mutex<Vec<Tunnel>>> = Arc::default();
        let log = Arc::clone(&tunnels);
        let credentials = credentials.map(|(user, password)| {
            Arc::new((user.as_bytes().to_vec(), password.as_bytes().to_vec()))
        });
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let log = Arc::clone(&log);
                let credentials = credentials.clone();
                tokio::spawn(async move {
                    let _ignored = serve(client, credentials.as_deref(), &log).await;
                });
            }
        });
        Self {
            port,
            tunnels,
            _task: AbortOnDrop(task),
        }
    }

    /// Every tunnel opened so far.
    pub fn tunnels(&self) -> Vec<Tunnel> {
        self.tunnels.lock().clone()
    }
}

/// Runs one client: handshake, connect, then relays both directions until both ends close.
async fn serve(
    mut client: TcpStream,
    credentials: Option<&(Vec<u8>, Vec<u8>)>,
    log: &Mutex<Vec<Tunnel>>,
) -> std::io::Result<()> {
    let version = client.read_u8().await?;
    let tunnel = match version {
        4 => socks4(&mut client).await?,
        5 => socks5(&mut client, credentials).await?,
        _ => return Ok(()),
    };
    let Some((tunnel, mut server)) = tunnel else {
        return Ok(());
    };
    log.lock().push(tunnel);
    tokio::io::copy_bidirectional(&mut client, &mut server).await?;
    Ok(())
}

/// Reads a NUL-terminated field.
async fn read_nul_terminated(client: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut field = Vec::new();
    loop {
        match client.read_u8().await? {
            0 => return Ok(field),
            byte => field.push(byte),
        }
    }
}

/// The SOCKS4 request after the version byte; replies and returns the tunnel.
async fn socks4(client: &mut TcpStream) -> std::io::Result<Option<(Tunnel, TcpStream)>> {
    let command = client.read_u8().await?;
    let port = client.read_u16().await?;
    let mut ip = [0u8; 4];
    client.read_exact(&mut ip).await?;
    let user = read_nul_terminated(client).await?;
    // SOCKS4a: 0.0.0.x announces a host name after the user id.
    let target = if ip[..3] == [0, 0, 0] && ip[3] != 0 {
        let host = read_nul_terminated(client).await?;
        format!("{}:{port}", String::from_utf8_lossy(&host))
    } else {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port).to_string()
    };
    let server = if command == 1 {
        TcpStream::connect(&target).await.ok()
    } else {
        None
    };
    let status = if server.is_some() { 0x5A } else { 0x5B };
    client.write_all(&[0, status, 0, 0, 0, 0, 0, 0]).await?;
    let tunnel = Tunnel {
        version: 4,
        user: (!user.is_empty()).then(|| String::from_utf8_lossy(&user).into_owned()),
        target,
    };
    Ok(server.map(|server| (tunnel, server)))
}

/// The SOCKS5 greeting, authentication and request after the version byte; replies and returns
/// the tunnel.
async fn socks5(
    client: &mut TcpStream,
    credentials: Option<&(Vec<u8>, Vec<u8>)>,
) -> std::io::Result<Option<(Tunnel, TcpStream)>> {
    let count = client.read_u8().await?;
    let mut methods = vec![0u8; usize::from(count)];
    client.read_exact(&mut methods).await?;
    let method = if credentials.is_some() { 2 } else { 0 };
    if !methods.contains(&method) {
        client.write_all(&[5, 0xFF]).await?;
        return Ok(None);
    }
    client.write_all(&[5, method]).await?;

    let mut user = None;
    if let Some((expected_user, expected_password)) = credentials {
        let _subnegotiation_version = client.read_u8().await?;
        let mut name = vec![0u8; usize::from(client.read_u8().await?)];
        client.read_exact(&mut name).await?;
        let mut password = vec![0u8; usize::from(client.read_u8().await?)];
        client.read_exact(&mut password).await?;
        if &name != expected_user || &password != expected_password {
            client.write_all(&[1, 1]).await?;
            return Ok(None);
        }
        client.write_all(&[1, 0]).await?;
        user = Some(String::from_utf8_lossy(&name).into_owned());
    }

    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    let [_version, command, _reserved, address_type] = request;
    let host = match address_type {
        1 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            IpAddr::V4(Ipv4Addr::from(ip)).to_string()
        }
        4 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await?;
            format!("[{}]", Ipv6Addr::from(ip))
        }
        3 => {
            let mut name = vec![0u8; usize::from(client.read_u8().await?)];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => return Ok(None),
    };
    let port = client.read_u16().await?;
    let target = format!("{host}:{port}");
    let server = if command == 1 {
        TcpStream::connect(&target).await.ok()
    } else {
        None
    };
    let status = if server.is_some() { 0 } else { 5 };
    client
        .write_all(&[5, status, 0, 1, 0, 0, 0, 0, 0, 0])
        .await?;
    let tunnel = Tunnel {
        version: 5,
        user,
        target,
    };
    Ok(server.map(|server| (tunnel, server)))
}
