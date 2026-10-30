//! Pinned unsigned media downloads. No gateway credentials enter this module.
use std::net::{IpAddr, SocketAddr};
use url::{Host, Url};
const FAILED: &str = "assistant_media_download_failed";
const MAX_ADDRESSES: usize = 16;
struct Target {
    url: Url,
    addresses: Vec<SocketAddr>,
}

pub(super) fn source_url(value: &str) -> Result<Url, String> {
    if value.len() > 8192 {
        return Err(FAILED.into());
    }
    let mut url = Url::parse(value).map_err(|_| FAILED)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.host().is_some_and(super::web::is_public_host)
        || url.port_or_known_default().is_none_or(|p| p == 0)
    {
        return Err(FAILED.into());
    }
    url.set_fragment(None);
    Ok(url)
}
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => super::web::is_public_host(Host::Ipv4(ip)),
        IpAddr::V6(ip) => super::web::is_public_host(Host::Ipv6(ip)),
    }
}
fn checked(value: &str, addresses: Vec<IpAddr>) -> Result<Target, String> {
    let url = source_url(value)?;
    if addresses.is_empty()
        || addresses.len() > MAX_ADDRESSES
        || !addresses.iter().all(|ip| public(*ip))
    {
        return Err(FAILED.into());
    }
    let literal = match url.host() {
        Some(Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
        _ => None,
    };
    if literal.is_some_and(|ip| addresses.iter().any(|a| *a != ip)) {
        return Err(FAILED.into());
    }
    let port = url.port_or_known_default().ok_or(FAILED)?;
    let mut pinned = Vec::new();
    for ip in addresses {
        let addr = SocketAddr::new(ip, port);
        if !pinned.contains(&addr) {
            pinned.push(addr);
        }
    }
    Ok(Target {
        url,
        addresses: pinned,
    })
}
fn host_name(url: &Url) -> String {
    match url.host() {
        Some(Host::Domain(host)) => host.to_owned(),
        Some(Host::Ipv4(ip)) => ip.to_string(),
        Some(Host::Ipv6(ip)) => ip.to_string(),
        _ => String::new(),
    }
}
fn cidr_contains(value: &str, host: &str) -> bool {
    let Some((network, bits)) = value.split_once('/') else {
        return false;
    };
    let Ok(bits) = bits.parse::<u8>() else {
        return false;
    };
    match (network.parse::<IpAddr>(), host.parse::<IpAddr>()) {
        (Ok(IpAddr::V4(net)), Ok(IpAddr::V4(ip))) if bits <= 32 => {
            bits == 0 || (u32::from(net) >> (32 - bits)) == (u32::from(ip) >> (32 - bits))
        }
        (Ok(IpAddr::V6(net)), Ok(IpAddr::V6(ip))) if bits <= 128 => {
            bits == 0 || (u128::from(net) >> (128 - bits)) == (u128::from(ip) >> (128 - bits))
        }
        _ => false,
    }
}
fn bypasses(url: &Url, rules: &str) -> bool {
    let host = host_name(url).trim_end_matches('.').to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or(443);
    rules
        .split([',', ' '])
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .any(|value| {
            if value == "*" || cidr_contains(value, &host) {
                return true;
            }
            let (name, rule_port) = if let Some(inner) = value.strip_prefix('[') {
                let Some((name, tail)) = inner.split_once(']') else {
                    return false;
                };
                let port = if tail.is_empty() {
                    None
                } else {
                    match tail.strip_prefix(':').and_then(|v| v.parse::<u16>().ok()) {
                        Some(p) => Some(p),
                        None => return false,
                    }
                };
                (name, port)
            } else if value.bytes().filter(|b| *b == b':').count() == 1 {
                let (name, number) = value.split_once(':').expect("one colon");
                let Ok(port) = number.parse::<u16>() else {
                    return false;
                };
                (name, Some(port))
            } else {
                (value, None)
            };
            if rule_port.is_some_and(|p| p != port) {
                return false;
            }
            let name = name
                .trim_start_matches("*.")
                .trim_start_matches('.')
                .trim_end_matches('.')
                .to_ascii_lowercase();
            !name.is_empty() && (host == name || host.ends_with(&format!(".{name}")))
        })
}

mod transport;
#[cfg(test)]
use transport::connect_proxy_stream;
use transport::{request_on, NetworkResponse};

use super::{host::blocking, store::AssistantStore, types::RunProxy};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    time::Duration,
};

fn fake(ip: IpAddr) -> bool {
    matches!(ip,IpAddr::V4(ip) if ip.octets()[0] == 198 && (18..=19).contains(&ip.octets()[1]))
}
// Fixed public DNS infrastructure, shared with web_read's fake-IP fallback.
// Never used to override a genuinely private or mixed resolver answer.
const DOH: &str = "https://dns.alidns.com/resolve";
async fn doh(client: &reqwest::Client, host: &str) -> Result<Vec<IpAddr>, String> {
    let mut addresses = Vec::new();
    for (kind, record) in [("A", 1_u64), ("AAAA", 28)] {
        let mut response = client
            .get(DOH)
            .query(&[("name", host), ("type", kind)])
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|_| FAILED)?;
        if !response.status().is_success()
            || response.content_length().is_some_and(|n| n > 64 * 1024)
        {
            return Err(FAILED.into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| FAILED)? {
            if chunk.len() > (64 * 1024_usize).saturating_sub(bytes.len()) {
                return Err(FAILED.into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| FAILED)?;
        addresses.extend(dns_answers(&value, record)?);
        if addresses.len() > MAX_ADDRESSES {
            return Err(FAILED.into());
        }
    }
    Ok(addresses)
}
fn dns_answers(value: &Value, record: u64) -> Result<Vec<IpAddr>, String> {
    match value["Status"].as_u64() {
        Some(3) => return Ok(Vec::new()),
        Some(0) => {}
        _ => return Err(FAILED.into()),
    }
    if !value["Answer"].is_null() && !value["Answer"].is_array() {
        return Err(FAILED.into());
    }
    let mut addresses = Vec::new();
    for answer in value["Answer"].as_array().into_iter().flatten() {
        if answer["type"].as_u64() != Some(record) {
            continue;
        }
        let ip = answer["data"]
            .as_str()
            .and_then(|v| v.parse::<IpAddr>().ok())
            .ok_or(FAILED)?;
        if !public(ip) || (record == 1) != ip.is_ipv4() {
            return Err(FAILED.into());
        }
        addresses.push(ip);
        if addresses.len() > MAX_ADDRESSES {
            return Err(FAILED.into());
        }
    }
    Ok(addresses)
}

pub(super) struct Downloaded {
    inner: tempfile::NamedTempFile,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl std::ops::Deref for Downloaded {
    type Target = tempfile::NamedTempFile;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl std::ops::DerefMut for Downloaded {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

async fn resolve(value: &str, client: &reqwest::Client) -> Result<Target, String> {
    let url = source_url(value)?;
    let addresses = match url.host().ok_or(FAILED)? {
        Host::Ipv4(ip) => vec![IpAddr::V4(ip)],
        Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
        Host::Domain(host) => {
            let resolved = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::lookup_host((host, url.port_or_known_default().ok_or(FAILED)?)),
            )
            .await
            .map_err(|_| FAILED)?
            .map_err(|_| FAILED)?;
            let mut addresses = resolved
                .take(MAX_ADDRESSES + 1)
                .map(|addr| addr.ip())
                .collect::<Vec<_>>();
            if !addresses.is_empty() && addresses.iter().all(|ip| fake(*ip)) {
                addresses = doh(client, host).await?;
            }
            addresses
        }
    };
    checked(value, addresses)
}

fn secret_free(value: &str, keys: &[String]) -> bool {
    let mut bytes = value.as_bytes().to_vec();
    for round in 0..=4 {
        if keys
            .iter()
            .any(|key| !key.is_empty() && bytes.windows(key.len()).any(|v| v == key.as_bytes()))
        {
            return false;
        }
        if round == 4 {
            break;
        }
        let mut decoded = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' && index + 2 < bytes.len() {
                if let (Some(h), Some(l)) = (
                    (bytes[index + 1] as char).to_digit(16),
                    (bytes[index + 2] as char).to_digit(16),
                ) {
                    decoded.push((h * 16 + l) as u8);
                    index += 3;
                    continue;
                }
            }
            decoded.push(bytes[index]);
            index += 1;
        }
        if decoded == bytes {
            break;
        }
        bytes = decoded;
    }
    true
}

pub(super) async fn get(
    store: AssistantStore,
    value: &str,
    proxy: Option<RunProxy>,
    metadata: &reqwest::Client,
    secrets: &[String],
) -> Result<Downloaded, String> {
    if value.len() > 8192 || !secret_free(value, secrets) {
        return Err(FAILED.into());
    }
    static SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let slots = SLOTS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(3)))
        .clone();
    tokio::time::timeout(Duration::from_secs(300), async {
        let permit = slots.acquire_owned().await.map_err(|_| FAILED)?;
        let target = resolve(value, metadata).await?;
        let tls = blocking(|| Ok(transport::tls())).await?;
        let socket = tokio::time::timeout(
            Duration::from_secs(30),
            transport::open(&target, proxy.as_ref(), &tls),
        )
        .await
        .map_err(|_| FAILED)??;
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            request_on(&target.url, socket, tls),
        )
        .await
        .map_err(|_| FAILED)??;
        let inner = collect_video(store, response, super::video::MAX_VIDEO_BYTES).await?;
        Ok(Downloaded {
            inner,
            _permit: permit,
        })
    })
    .await
    .map_err(|_| FAILED)?
}
async fn collect_video(
    store: AssistantStore,
    mut network: NetworkResponse,
    cap: u64,
) -> Result<tempfile::NamedTempFile, String> {
    let response = &mut network.response;
    let encoding = response
        .headers()
        .get(hyper::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok());
    if !response.status().is_success()
        || encoding.is_some_and(|v| !v.eq_ignore_ascii_case("identity"))
    {
        return Err(FAILED.into());
    }
    if let Some(length) = response.headers().get(hyper::header::CONTENT_LENGTH) {
        if length
            .to_str()
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .is_none_or(|v| v > cap)
        {
            return Err(FAILED.into());
        }
    }
    let mut file = blocking(move || store.video_temp()).await?;
    let mut total = 0_u64;
    while let Some(frame) = response.body_mut().frame().await {
        let frame = frame.map_err(|_| FAILED)?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        total = total.checked_add(chunk.len() as u64).ok_or(FAILED)?;
        if total > cap {
            return Err(FAILED.into());
        }
        file = blocking(move || {
            file.write_all(&chunk).map_err(|_| FAILED)?;
            Ok(file)
        })
        .await?;
    }
    blocking(move || {
        file.as_file_mut().sync_all().map_err(|_| FAILED)?;
        file.seek(SeekFrom::Start(0)).map_err(|_| FAILED)?;
        let mut head = [0_u8; 12];
        file.read_exact(&mut head).map_err(|_| FAILED)?;
        if &head[4..8] != b"ftyp" {
            return Err(FAILED.into());
        }
        file.seek(SeekFrom::Start(0)).map_err(|_| FAILED)?;
        Ok(file)
    })
    .await
}

#[cfg(test)]
mod tests;
