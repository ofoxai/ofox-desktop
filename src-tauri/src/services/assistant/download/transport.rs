//! Per-download sockets; target DNS is never delegated to the proxy.
use super::*;
use base64::Engine;
use bytes::Bytes;
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

pub(super) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub(super) type Socket = Box<dyn Io>;

pub(super) fn tls() -> TlsConnector {
    static TLS: OnceLock<TlsConnector> = OnceLock::new();
    TLS.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring has supported TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        TlsConnector::from(Arc::new(config))
    })
    .clone()
}
async fn secure(socket: Socket, host: String, tls: &TlsConnector) -> Result<Socket, String> {
    let name = rustls::pki_types::ServerName::try_from(host).map_err(|_| FAILED)?;
    Ok(Box::new(
        tls.connect(name, socket).await.map_err(|_| FAILED)?,
    ))
}
type Credentials = (Vec<u8>, Vec<u8>);
fn credentials(proxy: &Url) -> Result<Option<Credentials>, String> {
    fn decode(value: &str) -> Result<Vec<u8>, String> {
        if value.len() > 2048 {
            return Err(FAILED.into());
        }
        let mut result = Vec::new();
        let mut it = value.bytes();
        while let Some(b) = it.next() {
            if b == b'%' {
                let h = it
                    .next()
                    .and_then(|b| (b as char).to_digit(16))
                    .ok_or(FAILED)?;
                let l = it
                    .next()
                    .and_then(|b| (b as char).to_digit(16))
                    .ok_or(FAILED)?;
                result.push((h * 16 + l) as u8);
            } else {
                result.push(b);
            }
        }
        Ok(result)
    }
    if proxy.username().is_empty() && proxy.password().is_none() {
        return Ok(None);
    }
    Ok(Some((
        decode(proxy.username())?,
        decode(proxy.password().unwrap_or(""))?,
    )))
}

pub(super) async fn open(
    target: &Target,
    proxy: Option<&super::super::types::RunProxy>,
    tls: &TlsConnector,
) -> Result<Socket, String> {
    let route = proxy.filter(|p| !bypasses(&target.url, &p.no_proxy));
    if let Some(route) = route {
        if route.url.len() > 8192 {
            return Err(FAILED.into());
        }
        let proxy = Url::parse(&route.url).map_err(|_| FAILED)?;
        if !["http", "https", "socks5", "socks5h"].contains(&proxy.scheme())
            || proxy.host().is_none()
        {
            return Err(FAILED.into());
        }
        let port = proxy.port().unwrap_or(match proxy.scheme() {
            "https" => 443,
            "http" => 80,
            _ => 1080,
        });
        if port == 0 {
            return Err(FAILED.into());
        }
        let socket = TcpStream::connect((host_name(&proxy).as_str(), port))
            .await
            .map_err(|_| FAILED)?;
        // One tunnel to an already checked literal address. Never retry direct
        // or send the original DNS name for the proxy to resolve independently.
        return connect_proxy_stream(socket, &proxy, target.addresses[0], tls).await;
    }
    for address in &target.addresses {
        if let Ok(socket) = TcpStream::connect(address).await {
            return Ok(Box::new(socket));
        }
    }
    Err(FAILED.into())
}

pub(super) async fn connect_proxy_stream(
    tcp: TcpStream,
    proxy: &Url,
    target: SocketAddr,
    tls: &TlsConnector,
) -> Result<Socket, String> {
    let mut socket: Socket = Box::new(tcp);
    if proxy.scheme() == "https" {
        socket = secure(socket, host_name(proxy), tls).await?;
    }
    let auth = credentials(proxy)?;
    if matches!(proxy.scheme(), "socks5" | "socks5h") {
        socks(&mut socket, target, auth).await?;
    } else if matches!(proxy.scheme(), "http" | "https") {
        let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
        if let Some((mut user, password)) = auth {
            user.push(b':');
            user.extend_from_slice(&password);
            let encoded = base64::engine::general_purpose::STANDARD.encode(user);
            request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
        }
        request.push_str("\r\n");
        socket
            .write_all(request.as_bytes())
            .await
            .map_err(|_| FAILED)?;
        socket.flush().await.map_err(|_| FAILED)?;
        let mut bytes = Vec::new();
        let mut chunk = [0; 2048];
        loop {
            let n = socket.read(&mut chunk).await.map_err(|_| FAILED)?;
            if n == 0 || n > (16 * 1024_usize).saturating_sub(bytes.len()) {
                return Err(FAILED.into());
            }
            bytes.extend_from_slice(&chunk[..n]);
            let mut headers = [httparse::EMPTY_HEADER; 64];
            let mut response = httparse::Response::new(&mut headers);
            match response.parse(&bytes).map_err(|_| FAILED)? {
                httparse::Status::Partial => {}
                httparse::Status::Complete(end) => {
                    if response.code != Some(200) || end != bytes.len() {
                        return Err(FAILED.into());
                    }
                    break;
                }
            }
        }
    } else {
        return Err(FAILED.into());
    }
    Ok(socket)
}
async fn socks(
    socket: &mut Socket,
    target: SocketAddr,
    auth: Option<Credentials>,
) -> Result<(), String> {
    if auth.is_some() {
        socket.write_all(&[5, 2, 0, 2]).await.map_err(|_| FAILED)?;
    } else {
        socket.write_all(&[5, 1, 0]).await.map_err(|_| FAILED)?;
    }
    let mut choice = [0; 2];
    socket.read_exact(&mut choice).await.map_err(|_| FAILED)?;
    if choice[0] != 5 {
        return Err(FAILED.into());
    }
    match choice[1] {
        0 => {}
        2 => {
            let Some((user, password)) = auth else {
                return Err(FAILED.into());
            };
            if user.is_empty() || user.len() > 255 || password.len() > 255 {
                return Err(FAILED.into());
            }
            let mut request = vec![1, user.len() as u8];
            request.extend_from_slice(&user);
            request.push(password.len() as u8);
            request.extend_from_slice(&password);
            socket.write_all(&request).await.map_err(|_| FAILED)?;
            socket.read_exact(&mut choice).await.map_err(|_| FAILED)?;
            if choice != [1, 0] {
                return Err(FAILED.into());
            }
        }
        _ => return Err(FAILED.into()),
    }
    let mut request = vec![5, 1, 0];
    match target.ip() {
        IpAddr::V4(ip) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
    }
    request.extend_from_slice(&target.port().to_be_bytes());
    socket.write_all(&request).await.map_err(|_| FAILED)?;
    let mut reply = [0; 4];
    socket.read_exact(&mut reply).await.map_err(|_| FAILED)?;
    if reply[..3] != [5, 0, 0] {
        return Err(FAILED.into());
    }
    let count = match reply[3] {
        1 => 4,
        4 => 16,
        3 => socket.read_u8().await.map_err(|_| FAILED)? as usize,
        _ => return Err(FAILED.into()),
    };
    let mut tail = vec![0; count + 2];
    socket.read_exact(&mut tail).await.map_err(|_| FAILED)?;
    Ok(())
}

struct Connection(tokio::task::JoinHandle<()>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(super) struct NetworkResponse {
    pub(super) response: hyper::Response<hyper::body::Incoming>,
    _connection: Connection,
}
pub(super) async fn request_on(
    url: &Url,
    socket: Socket,
    tls: TlsConnector,
) -> Result<NetworkResponse, String> {
    let socket = secure(socket, host_name(url), &tls).await?;
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(64)
        .max_buf_size(64 * 1024)
        .handshake::<_, Empty<Bytes>>(TokioIo::new(socket))
        .await
        .map_err(|_| FAILED)?;
    let connection = Connection(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    let host = match url.host() {
        Some(Host::Ipv6(ip)) => format!("[{ip}]"),
        _ => host_name(url),
    };
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", authority)
        .header("User-Agent", "OfoxDesktop-Media/1")
        .header("Accept", "video/mp4,application/octet-stream")
        .header("Accept-Encoding", "identity")
        .header("Connection", "close")
        .body(Empty::<Bytes>::new())
        .map_err(|_| FAILED)?;
    let response = sender.send_request(request).await.map_err(|_| FAILED)?;
    Ok(NetworkResponse {
        response,
        _connection: connection,
    })
}
