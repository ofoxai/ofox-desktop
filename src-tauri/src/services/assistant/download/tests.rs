use super::*;
use std::net::IpAddr;

#[test]
fn download_dns_rejects_empty_mixed_private_and_reserved_answers_for_every_route() {
    for answers in [
        vec![],
        vec!["127.0.0.1"],
        vec!["1.1.1.1", "10.0.0.1"],
        vec!["198.18.0.1"],
        vec!["100.64.0.1"],
        vec!["::ffff:1.1.1.1"],
        vec!["2001:db8::1"],
    ] {
        let addresses: Vec<IpAddr> = answers.iter().map(|v| v.parse().unwrap()).collect();
        assert!(
            checked("https://example.com/clip.mp4", addresses).is_err(),
            "{answers:?}"
        );
    }
    let plan = checked(
        "https://example.com:8443/clip.mp4?signature=synthetic",
        vec![
            "1.1.1.1".parse().unwrap(),
            "2606:4700:4700::1111".parse().unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(plan.url.host_str(), Some("example.com"));
    assert!(plan.addresses.iter().all(|v| v.port() == 8443));
    assert_eq!(plan.addresses[0].ip(), "1.1.1.1".parse::<IpAddr>().unwrap());
}
#[test]
fn download_proxy_bypass_is_host_port_scoped_and_never_a_suffix_substring() {
    let url = url::Url::parse("https://cdn.example.com/video.mp4").unwrap();
    for rule in [
        "*",
        ".example.com",
        "*.example.com",
        "EXAMPLE.COM:443",
        "example.com.",
    ] {
        assert!(bypasses(&url, rule), "{rule}");
    }
    for rule in [
        "ample.com",
        "other.example.com",
        "example.com:8443",
        "localhost,127.0.0.1",
    ] {
        assert!(!bypasses(&url, rule), "{rule}");
    }
    assert!(bypasses(
        &url::Url::parse("https://1.1.1.1/").unwrap(),
        "1.1.1.0/24"
    ));
    assert!(!bypasses(
        &url::Url::parse("https://1.1.2.1/").unwrap(),
        "1.1.1.0/24"
    ));
}
#[test]
fn download_url_boundary_refuses_credentials_non_https_and_unbounded_sources() {
    for value in [
        "http://example.com/a".to_owned(),
        "https://key:secret@example.com/a".into(),
        "https://localhost./a".into(),
        "https://example.local/a".into(),
        format!("https://example.com/{}", "x".repeat(8193)),
    ] {
        assert!(checked(&value, vec!["1.1.1.1".parse().unwrap()]).is_err());
    }
}

use base64::Engine;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn fixture_tls() -> (tokio_rustls::TlsConnector, tokio_rustls::TlsAcceptor) {
    fn pem(text: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(
                text.lines()
                    .filter(|line| !line.starts_with("-----"))
                    .collect::<String>(),
            )
            .unwrap()
    }
    let ca = rustls::pki_types::CertificateDer::from(pem(include_str!("../../../../../assistant-runtime/search-mcp/test/fixtures/tls/test-only-example.com-cert.txt")));
    let cert = rustls::pki_types::CertificateDer::from(pem(include_str!(
        "../../../../tests/fixtures/assistant-download/test-only-example.com-leaf-cert.txt"
    )));
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(pem(include_str!(
        "../../../../tests/fixtures/assistant-download/test-only-example.com-leaf-key.txt"
    )));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())
        .unwrap();
    (
        tokio_rustls::TlsConnector::from(Arc::new(client)),
        tokio_rustls::TlsAcceptor::from(Arc::new(server)),
    )
}
async fn headers(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut result = Vec::new();
    while !result.ends_with(b"\r\n\r\n") {
        let b = stream.read_u8().await.unwrap();
        result.push(b);
        assert!(result.len() < 16 * 1024);
    }
    result
}
#[tokio::test]
async fn download_http_proxy_connects_to_literal_pinned_address_and_scopes_credentials_to_proxy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut io, _) = listener.accept().await.unwrap();
        let request = String::from_utf8(headers(&mut io).await).unwrap();
        io.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .unwrap();
        request
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let (tls, _) = fixture_tls();
    let proxy = Url::parse("http://user:p%40ss@proxy.example:8080").unwrap();
    let tunnel = connect_proxy_stream(tcp, &proxy, "1.1.1.1:443".parse().unwrap(), &tls)
        .await
        .unwrap();
    drop(tunnel);
    let request = server.await.unwrap();
    assert!(request.starts_with("CONNECT 1.1.1.1:443 HTTP/1.1\r\n"));
    assert!(request.contains("Proxy-Authorization: Basic dXNlcjpwQHNz\r\n"));
    assert!(!request.contains("example.com"));
}
#[tokio::test]
async fn download_socks5_and_socks5h_pin_ip_instead_of_using_remote_dns() {
    for scheme in ["socks5", "socks5h"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut io, _) = listener.accept().await.unwrap();
            let mut hello = [0; 3];
            io.read_exact(&mut hello).await.unwrap();
            assert_eq!(hello, [5, 1, 0]);
            io.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 10];
            io.read_exact(&mut request).await.unwrap();
            assert_eq!(request, [5, 1, 0, 1, 1, 1, 1, 1, 1, 187]);
            io.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let (tls, _) = fixture_tls();
        let proxy = Url::parse(&format!("{scheme}://proxy.example:1080")).unwrap();
        let tunnel = connect_proxy_stream(tcp, &proxy, "1.1.1.1:443".parse().unwrap(), &tls)
            .await
            .unwrap();
        drop(tunnel);
        server.await.unwrap();
    }
}
#[tokio::test]
async fn download_keeps_original_tls_name_and_http_host_with_private_bounded_output() {
    let (tls, acceptor) = fixture_tls();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut io = acceptor.accept(tcp).await.unwrap();
        assert_eq!(io.get_ref().1.server_name(), Some("example.com"));
        let request = String::from_utf8(headers(&mut io).await).unwrap();
        assert!(request.starts_with("GET /clip?signature=synthetic HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("host: example.com\r\n"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(!request.to_ascii_lowercase().contains("cookie:"));
        let body = b"\0\0\0\x18ftypmp42fixture";
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        io.write_all(head.as_bytes()).await.unwrap();
        io.write_all(body).await.unwrap();
        io.shutdown().await.unwrap();
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let url = Url::parse("https://example.com/clip?signature=synthetic").unwrap();
    let response = request_on(&url, Box::new(tcp), tls).await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let store =
        super::super::store::AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
    let mut file = collect_video(store, response, 64).await.unwrap();
    let path = file.path().to_owned();
    use std::io::Read;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp");
    #[cfg(unix)]
    assert_eq!(crate::config::file_mode(&path), Some(0o600));
    drop(file);
    assert!(!path.exists());
    server.await.unwrap();
}

#[test]
fn download_signed_url_never_forwards_a_known_gateway_key_in_raw_or_encoded_form() {
    let key = "synthetic-gateway-key".to_owned();
    let keys = vec![key.clone()];
    assert!(!secret_free(&format!("https://example.com/{key}"), &keys));
    assert!(!secret_free(
        "https://example.com/clip?signature=%73ynthetic-gateway-key",
        &keys
    ));
    assert!(!secret_free(
        "https://example.com/clip?signature=%2573ynthetic-gateway-key",
        &keys
    ));
    assert!(secret_free(
        "https://example.com/clip?signature=unrelated-capability",
        &keys
    ));
}

#[tokio::test]
async fn download_https_proxy_preserves_both_tls_names_and_never_sends_proxy_auth_to_origin() {
    let (tls, acceptor) = fixture_tls();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut outer = acceptor.accept(tcp).await.unwrap();
        assert_eq!(outer.get_ref().1.server_name(), Some("example.com"));
        let connect = String::from_utf8(headers(&mut outer).await).unwrap();
        assert!(connect.starts_with("CONNECT 1.1.1.1:443 HTTP/1.1\r\n"));
        assert!(connect.contains("Proxy-Authorization:"));
        outer.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        let mut origin = acceptor.accept(outer).await.unwrap();
        assert_eq!(origin.get_ref().1.server_name(), Some("example.com"));
        let request = String::from_utf8(headers(&mut origin).await).unwrap();
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(request
            .to_ascii_lowercase()
            .contains("host: example.com\r\n"));
        let body = b"\0\0\0\x18ftypmp42fixture";
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        origin.write_all(head.as_bytes()).await.unwrap();
        origin.write_all(body).await.unwrap();
        origin.shutdown().await.unwrap();
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let proxy = Url::parse("https://user:synthetic@example.com:443").unwrap();
    let socket = connect_proxy_stream(tcp, &proxy, "1.1.1.1:443".parse().unwrap(), &tls)
        .await
        .unwrap();
    let response = request_on(
        &Url::parse("https://example.com/video.mp4").unwrap(),
        socket,
        tls,
    )
    .await
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let store =
        super::super::store::AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
    drop(collect_video(store, response, 64).await.unwrap());
    server.await.unwrap();
}
#[tokio::test]
async fn download_certificate_validation_uses_original_name_instead_of_trusting_the_pinned_address()
{
    let (tls, acceptor) = fixture_tls();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let _ = acceptor.accept(tcp).await;
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    assert!(request_on(
        &Url::parse("https://wrong.example.com/video.mp4").unwrap(),
        Box::new(tcp),
        tls
    )
    .await
    .is_err());
    tokio::time::timeout(std::time::Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn download_failed_status_redirect_encoding_size_and_invalid_video_leave_no_partial_file() {
    for (status, extra, body, chunked) in [
        (
            302,
            "Location: https://other.example/clip\r\n",
            b"redirect".to_vec(),
            false,
        ),
        (500, "", b"error".to_vec(), false),
        (
            200,
            "Content-Encoding: gzip\r\n",
            b"compressed".to_vec(),
            false,
        ),
        (200, "", vec![0; 65], false),
        (200, "", vec![0; 65], true),
        (200, "", b"<html>not a video</html>".to_vec(), false),
    ] {
        let (tls, acceptor) = fixture_tls();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut io = acceptor.accept(tcp).await.unwrap();
            let _ = headers(&mut io).await;
            let length = if chunked {
                "Transfer-Encoding: chunked\r\n".to_owned()
            } else {
                format!("Content-Length: {}\r\n", body.len())
            };
            let head =
                format!("HTTP/1.1 {status} Result\r\n{extra}{length}Connection: close\r\n\r\n");
            let _ = io.write_all(head.as_bytes()).await;
            if chunked {
                let _ = io
                    .write_all(format!("{:X}\r\n", body.len()).as_bytes())
                    .await;
                let _ = io.write_all(&body).await;
                let _ = io.write_all(b"\r\n0\r\n\r\n").await;
            } else {
                let _ = io.write_all(&body).await;
            }
            let _ = io.shutdown().await;
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let response = request_on(
            &Url::parse("https://example.com/clip").unwrap(),
            Box::new(tcp),
            tls,
        )
        .await
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let store =
            super::super::store::AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
        assert!(collect_video(store, response, 64).await.is_err());
        let dir = root
            .path()
            .join("assistant")
            .join("ab".repeat(32))
            .join(".downloads");
        if dir.exists() {
            assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
        }
        server.await.unwrap();
    }
}
#[tokio::test]
async fn download_cancel_drops_the_open_partial_file_and_owned_connection() {
    const FIRST_CHUNK: &[u8] = b"\0\0\0\x18ftypmp42fixture";
    let (tls, acceptor) = fixture_tls();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut io = acceptor.accept(tcp).await.unwrap();
        let _ = headers(&mut io).await;
        let body = FIRST_CHUNK;
        io.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        io.write_all(format!("{:X}\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        io.write_all(body).await.unwrap();
        io.write_all(b"\r\n").await.unwrap();
        io.flush().await.unwrap();
        let mut byte = [0];
        let _ = io.read(&mut byte).await;
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let response = request_on(
        &Url::parse("https://example.com/clip").unwrap(),
        Box::new(tcp),
        tls,
    )
    .await
    .unwrap();
    let root = tempfile::tempdir().unwrap();
    let store =
        super::super::store::AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
    let dir = root
        .path()
        .join("assistant")
        .join("ab".repeat(32))
        .join(".downloads");
    let task = tokio::spawn(collect_video(store, response, 64));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if dir.exists()
                && std::fs::read_dir(&dir)
                    .unwrap()
                    // Windows directory-entry metadata may lag the open writer.
                    // Observe the actual payload before cancelling the download.
                    .any(|f| {
                        std::fs::read(f.unwrap().path())
                            .is_ok_and(|bytes| bytes.as_slice() == FIRST_CHUNK)
                    })
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while std::fs::read_dir(&dir).unwrap().next().is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn download_socks_auth_and_ipv6_are_bound_to_the_checked_literal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let target: SocketAddr = "[2606:4700:4700::1111]:443".parse().unwrap();
    let server = tokio::spawn(async move {
        let (mut io, _) = listener.accept().await.unwrap();
        let mut hello = [0; 4];
        io.read_exact(&mut hello).await.unwrap();
        assert_eq!(hello, [5, 2, 0, 2]);
        io.write_all(&[5, 2]).await.unwrap();
        assert_eq!(io.read_u8().await.unwrap(), 1);
        let count = io.read_u8().await.unwrap();
        let mut user = vec![0; count as usize];
        io.read_exact(&mut user).await.unwrap();
        assert_eq!(user, b"user");
        let count = io.read_u8().await.unwrap();
        let mut pass = vec![0; count as usize];
        io.read_exact(&mut pass).await.unwrap();
        assert_eq!(pass, b"p@ss");
        io.write_all(&[1, 0]).await.unwrap();
        let mut head = [0; 4];
        io.read_exact(&mut head).await.unwrap();
        assert_eq!(head, [5, 1, 0, 4]);
        let mut payload = [0; 18];
        io.read_exact(&mut payload).await.unwrap();
        let IpAddr::V6(ip) = target.ip() else {
            unreachable!()
        };
        assert_eq!(&payload[..16], ip.octets());
        assert_eq!(&payload[16..], 443_u16.to_be_bytes());
        io.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
            .await
            .unwrap();
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let (tls, _) = fixture_tls();
    let socket = connect_proxy_stream(
        tcp,
        &Url::parse("socks5h://user:p%40ss@proxy.example:1080").unwrap(),
        target,
        &tls,
    )
    .await
    .unwrap();
    drop(socket);
    server.await.unwrap();
}
#[tokio::test]
async fn download_configured_proxy_failure_never_falls_back_to_direct_but_explicit_bypass_does() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);
    // Only this fixture bypasses the constructor to observe any illicit direct
    // fallback locally. Production Target fields are private and checked first.
    let target = Target {
        url: Url::parse("https://example.com/clip").unwrap(),
        addresses: vec![addr],
    };
    let (tls, _) = fixture_tls();
    let mut proxy = super::super::types::RunProxy {
        url: format!("http://{dead_addr}"),
        no_proxy: String::new(),
    };
    assert!(transport::open(&target, Some(&proxy), &tls).await.is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    proxy.no_proxy = "example.com:443".into();
    let socket = transport::open(&target, Some(&proxy), &tls).await.unwrap();
    let _accepted = listener.accept().await.unwrap();
    drop(socket);
}

#[tokio::test]
async fn download_slot_is_held_until_the_private_file_is_published_or_dropped() {
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = slots.clone().acquire_owned().await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let store =
        super::super::store::AssistantStore::for_owner(root.path(), &"ab".repeat(32)).unwrap();
    let file = Downloaded {
        inner: store.video_temp().unwrap(),
        _permit: permit,
    };
    let path = file.path().to_owned();
    assert_eq!(slots.available_permits(), 0);
    assert!(path.exists());
    drop(file);
    assert_eq!(slots.available_permits(), 1);
    assert!(!path.exists());
}
#[test]
fn download_public_dns_fallback_validates_status_families_and_all_returned_addresses() {
    use serde_json::json;
    assert_eq!(dns_answers(&json!({"Status":0,"Answer":[{"type":5,"data":"canonical.example"},{"type":1,"data":"1.1.1.1"}]}),1).unwrap(),vec!["1.1.1.1".parse::<IpAddr>().unwrap()]);
    assert_eq!(
        dns_answers(&json!({"Status":3}), 28).unwrap(),
        Vec::<IpAddr>::new()
    );
    for value in [
        json!({"Status":2}),
        json!({"Status":0,"Answer":"malformed"}),
        json!({"Status":0,"Answer":[{"type":1,"data":"127.0.0.1"}]}),
        json!({"Status":0,"Answer":[{"type":1,"data":"1.1.1.1"},{"type":1,"data":"192.168.0.1"}]}),
        json!({"Status":0,"Answer":[{"type":1,"data":"2606:4700:4700::1111"}]}),
    ] {
        assert!(dns_answers(&value, 1).is_err(), "{value}");
    }
    assert!(fake("198.18.0.1".parse().unwrap()));
    assert!(fake("198.19.255.254".parse().unwrap()));
    assert!(!fake("192.168.0.1".parse().unwrap()));
}
