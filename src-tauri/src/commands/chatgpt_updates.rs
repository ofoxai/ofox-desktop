//! ChatGPT 桌面 App（OpenAI Codex）的最新版本检测。
//!
//! macOS 读 App 内置 Sparkle feed（`codexSparkleFeedUrl`），Windows 读 App 自己
//! 做 Store 更新判断用的 manifest。两边版本号格式不同（macOS 三段、MSIX 四段且
//! 第三段不同源），只在同平台内比较，也不走 semver。
//!
//! 下面三个 URL 是 OpenAI 的固定基础设施地址，不属于 AGENTS.md 里按 ofox apex
//! 派生 URL 的规则，属于显式例外。个性化灰度接口
//! `updates.oaistatic.com/codex/app/appcast?installation_id=…` 故意不用：公开 feed
//! 可能比某台机器实际收到的版本更新，这一点在 update_reason 里说明。

// Linux 上 chatgpt 分支直接返回 unsupported，整个模块都用不到。
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

use std::cmp::Ordering;
use std::time::Duration;

use quick_xml::events::Event;
use quick_xml::Reader;

pub(crate) const MAC_APPCAST_ARM64_URL: &str =
    "https://persistent.oaistatic.com/codex-app-prod/appcast.xml";
pub(crate) const MAC_APPCAST_X64_URL: &str =
    "https://persistent.oaistatic.com/codex-app-prod/appcast-x64.xml";
pub(crate) const WINDOWS_MANIFEST_URL: &str =
    "https://persistent.oaistatic.com/codex-app-prod/windows-store-update.json";

/// Microsoft Store Product ID —— winget msstore 源 + ms-windows-store URI 都用这个。
pub(crate) const STORE_PRODUCT_ID: &str = "9PLM9XGG6VKS";

/// AppxPackage 名——`Get-AppxPackage -Name` 查询用。
pub(crate) const PACKAGE_NAME: &str = "OpenAI.Codex";

/// PackageFamilyName——拼 AUMID（`PackageFamilyName!AppId`）用。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) const PACKAGE_FAMILY: &str = "OpenAI.Codex_2p2nqsd0c76g0";

const FETCH_TIMEOUT: Duration = Duration::from_secs(7);
const MAX_FEED_BYTES: usize = 1024 * 1024;

/// `shell:AppsFolder\<AUMID>`——启动 Store App 的目标，检测与启动共用。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn windows_launch_target() -> String {
    format!("shell:AppsFolder\\{PACKAGE_FAMILY}!App")
}

/// 纯数字点分比较，短的一侧补 0（`26.928.2636.0 == 26.928.2636`）。任一侧不合法返回 None。
pub(crate) fn compare_dotted(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (dotted_parts(a)?, dotted_parts(b)?);
    let part = |parts: &[u64], index: usize| parts.get(index).copied().unwrap_or(0);
    Some(
        (0..a.len().max(b.len()))
            .map(|index| part(&a, index).cmp(&part(&b, index)))
            .find(|order| order.is_ne())
            .unwrap_or(Ordering::Equal),
    )
}

fn dotted_parts(version: &str) -> Option<Vec<u64>> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    version
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        })
        .collect()
}

#[derive(Default)]
struct AppcastItem {
    short_version: Option<String>,
    minimum_os: Option<String>,
    arm64_only: bool,
}

impl AppcastItem {
    fn set(&mut self, name: &[u8], value: &str) {
        match name {
            b"shortVersionString" => self.short_version = Some(value.to_string()),
            b"minimumSystemVersion" => self.minimum_os = Some(value.to_string()),
            b"hardwareRequirements" => {
                self.arm64_only = value.split(',').any(|item| item.trim() == "arm64")
            }
            _ => {}
        }
    }

    fn installable_version(self, host_arch: &str, host_os: Option<&str>) -> Option<String> {
        if self.arm64_only && host_arch != "aarch64" {
            return None;
        }
        if let (Some(host), Some(minimum)) = (host_os, self.minimum_os.as_deref()) {
            if compare_dotted(host, minimum) == Some(Ordering::Less) {
                return None;
            }
        }
        let version = self.short_version?;
        dotted_parts(&version).map(|_| version)
    }
}

/// 从 Sparkle appcast 里挑出本机可装的最高版本（`shortVersionString`）。
/// 只读 `<item>` 的直接子节点，嵌套的 delta enclosure 和 CDATA 说明都不看。
pub(crate) fn parse_appcast(
    xml: &str,
    host_arch: &str,
    host_os: Option<&str>,
) -> Result<Option<String>, String> {
    let mut reader = Reader::from_str(xml);
    let mut best: Option<String> = None;
    // Depth below the current <item>; None while outside any item.
    let mut item_depth: Option<usize> = None;
    // Local name of the direct <item> child whose text is being read.
    let mut field: Option<Vec<u8>> = None;
    let mut item = AppcastItem::default();
    loop {
        match reader
            .read_event()
            .map_err(|err| format!("Invalid appcast XML: {err}"))?
        {
            Event::Start(tag) => {
                if let Some(depth) = item_depth {
                    field = (depth == 0).then(|| tag.local_name().as_ref().to_vec());
                    item_depth = Some(depth + 1);
                } else if tag.local_name().as_ref() == b"item" {
                    item_depth = Some(0);
                    item = AppcastItem::default();
                }
            }
            Event::End(_) => match item_depth {
                Some(0) => {
                    if let Some(version) =
                        std::mem::take(&mut item).installable_version(host_arch, host_os)
                    {
                        if best.as_deref().is_none_or(|best| {
                            compare_dotted(&version, best) == Some(Ordering::Greater)
                        }) {
                            best = Some(version);
                        }
                    }
                    item_depth = None;
                }
                Some(depth) => {
                    field = None;
                    item_depth = Some(depth - 1);
                }
                None => {}
            },
            Event::Text(text) if item_depth == Some(1) => {
                if let Some(name) = field.as_deref() {
                    let value = text
                        .decode()
                        .map_err(|err| format!("Invalid appcast text: {err}"))?;
                    item.set(name, value.trim());
                }
            }
            Event::Eof => return Ok(best),
            _ => {}
        }
    }
}

pub(crate) fn macos_feed_url(arch: &str) -> &'static str {
    if arch == "aarch64" {
        MAC_APPCAST_ARM64_URL
    } else {
        MAC_APPCAST_X64_URL
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreManifest {
    schema_version: u32,
    build_version: String,
    store_product_id: String,
    package_identity: String,
}

/// 校验 manifest 确实描述的是 ChatGPT 正式版 Store 包，返回 `buildVersion`。
pub(crate) fn parse_windows_manifest(body: &str) -> Result<String, String> {
    let manifest: StoreManifest =
        serde_json::from_str(body).map_err(|err| format!("Invalid Store manifest: {err}"))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "Unsupported Store manifest schema {}",
            manifest.schema_version
        ));
    }
    if !manifest
        .store_product_id
        .eq_ignore_ascii_case(STORE_PRODUCT_ID)
        || manifest.package_identity != PACKAGE_NAME
    {
        return Err(format!(
            "Store manifest describes {} / {}, not ChatGPT",
            manifest.store_product_id, manifest.package_identity
        ));
    }
    if dotted_parts(&manifest.build_version).is_none() {
        return Err(format!(
            "Invalid Store build version {:?}",
            manifest.build_version
        ));
    }
    Ok(manifest.build_version)
}

pub(crate) struct DesktopUpdateFields {
    pub latest_version: Option<String>,
    pub update_status: &'static str,
    pub update_source: Option<String>,
    pub update_supported: bool,
    pub update_reason: Option<String>,
}

/// 已安装桌面 App 的更新字段。`latest = None` 表示调用方没联网（include_latest=false）。
/// 查询失败不报 failed，也不冒充 current：保留「请在客户端内检查更新」并在 reason 里写原因。
pub(crate) fn desktop_update_fields(
    source: &str,
    installed: &str,
    latest: Option<Result<String, String>>,
    one_click: bool,
) -> DesktopUpdateFields {
    let update_source = Some(source.to_string());
    let latest = match latest {
        None => {
            return DesktopUpdateFields {
                latest_version: None,
                update_status: "unchecked",
                update_source,
                update_supported: false,
                update_reason: None,
            }
        }
        Some(Err(error)) => {
            return DesktopUpdateFields {
                latest_version: None,
                update_status: "appManaged",
                update_source,
                update_supported: false,
                update_reason: Some(format!("Latest-version check failed: {error}")),
            }
        }
        Some(Ok(latest)) => latest,
    };
    let (update_status, update_reason) = match compare_dotted(&latest, installed) {
        Some(Ordering::Greater) => ("available", Some(rollout_note(source).to_string())),
        Some(_) => ("current", None),
        None => (
            "unknown",
            Some(format!(
                "Cannot compare installed {installed} with latest {latest}"
            )),
        ),
    };
    DesktopUpdateFields {
        latest_version: Some(latest),
        update_status,
        update_source,
        update_supported: one_click && update_status == "available",
        update_reason,
    }
}

fn rollout_note(source: &str) -> &'static str {
    if source == "sparkle" {
        "Public Sparkle feed; ChatGPT may roll this release out gradually"
    } else {
        "Microsoft Store may offer this build later than OpenAI's manifest"
    }
}

async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<String, String> {
    let response = client
        .get(url)
        .timeout(timeout)
        .header("User-Agent", "ofox-desktop")
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|err| format!("GET {url} failed: {err}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_FEED_BYTES as u64)
    {
        return Err(format!("{url} is larger than {MAX_FEED_BYTES} bytes"));
    }
    let body = response
        .bytes()
        .await
        .map_err(|err| format!("Reading {url} failed: {err}"))?;
    if body.len() > MAX_FEED_BYTES {
        return Err(format!("{url} is larger than {MAX_FEED_BYTES} bytes"));
    }
    String::from_utf8(body.to_vec()).map_err(|_| format!("{url} is not UTF-8"))
}

pub(crate) async fn fetch_macos_latest(
    client: &reqwest::Client,
    url: &str,
    host_arch: &str,
    host_os: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let xml = fetch_text(client, url, timeout).await?;
    parse_appcast(&xml, host_arch, host_os)?
        .ok_or_else(|| format!("No release in {url} can run on this Mac"))
}

pub(crate) async fn fetch_windows_latest(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<String, String> {
    parse_windows_manifest(&fetch_text(client, url, timeout).await?)
}

#[cfg(target_os = "macos")]
fn host_macos_version() -> Option<String> {
    super::chatgpt_app::macos_product_version().ok()
}

#[cfg(not(target_os = "macos"))]
fn host_macos_version() -> Option<String> {
    None
}

/// 本机平台对应的最新版本。
pub(crate) async fn fetch_latest_for_host(client: &reqwest::Client) -> Result<String, String> {
    if cfg!(target_os = "macos") {
        let arch = std::env::consts::ARCH;
        let host_os = host_macos_version();
        fetch_macos_latest(
            client,
            macos_feed_url(arch),
            arch,
            host_os.as_deref(),
            FETCH_TIMEOUT,
        )
        .await
    } else if cfg!(target_os = "windows") {
        fetch_windows_latest(client, WINDOWS_MANIFEST_URL, FETCH_TIMEOUT).await
    } else {
        Err("ChatGPT desktop updates are only tracked on macOS and Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FEED_HEAD: &str = r#"<?xml version='1.0' encoding='utf-8'?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
    <channel>
        <title>Codex</title>"#;
    const FEED_TAIL: &str = "\n    </channel>\n</rss>";

    fn item(version: &str, build: &str, min_os: &str, arm64_only: bool) -> String {
        let hardware = if arm64_only {
            "<sparkle:hardwareRequirements>arm64</sparkle:hardwareRequirements>"
        } else {
            ""
        };
        format!(
            r#"
        <item>
            <title>{version}</title>
            <sparkle:version>{build}</sparkle:version>
            <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>{min_os}</sparkle:minimumSystemVersion>
            {hardware}
            <enclosure url="https://example.invalid/{version}.zip" length="1" type="application/octet-stream" />
        </item>"#
        )
    }

    fn feed(items: &[String]) -> String {
        format!("{FEED_HEAD}{}{FEED_TAIL}", items.concat())
    }

    #[test]
    fn dotted_compare_is_numeric_and_zero_padded() {
        assert_eq!(
            compare_dotted("26.928.21956", "26.924.22138"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_dotted("26.1000.1", "26.999.9"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_dotted("26.928.2636.0", "26.928.2636"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_dotted(" v26.924.1 ", "26.924.2"),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn dotted_compare_rejects_non_numeric_parts() {
        for bad in ["", "26..1", "26.9x", "+1.2", "1.2-beta"] {
            assert_eq!(compare_dotted(bad, "1.0"), None, "{bad:?}");
            assert_eq!(compare_dotted("1.0", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn appcast_parses_real_feed_item_with_deltas() {
        let xml = format!(
            "{FEED_HEAD}{}{FEED_TAIL}",
            r#"
        <item>
            <title>26.928.21956</title>
            <pubDate>Wed, 30 Sep 2026 05:40:37 +0000</pubDate>
            <sparkle:version>12404</sparkle:version>
            <sparkle:shortVersionString>26.928.21956</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>13.0</sparkle:minimumSystemVersion>
            <sparkle:hardwareRequirements>arm64</sparkle:hardwareRequirements>
            <enclosure url="https://persistent.oaistatic.com/codex-app-prod/ChatGPT-darwin-arm64-26.928.21956.zip" length="682593617" type="application/octet-stream" sparkle:edSignature="T3fY" />
            <sparkle:deltas>
                <enclosure url="https://persistent.oaistatic.com/codex-app-prod/ChatGPT12404-12246-arm64.delta" sparkle:deltaFrom="12246" length="21359970" type="application/octet-stream" sparkle:edSignature="dxY4" />
            </sparkle:deltas>
        </item>"#
        );
        assert_eq!(
            parse_appcast(&xml, "aarch64", Some("15.6.1")).unwrap(),
            Some("26.928.21956".into())
        );
    }

    #[test]
    fn appcast_picks_highest_version_not_first_item() {
        let xml = feed(&[
            item("26.924.22138", "11645", "13.0", true),
            item("26.928.21956", "12404", "13.0", true),
            item("26.803.81509", "6415", "12.0", true),
        ]);
        assert_eq!(
            parse_appcast(&xml, "aarch64", None).unwrap(),
            Some("26.928.21956".into())
        );
    }

    #[test]
    fn appcast_skips_arm64_only_items_on_intel() {
        let xml = feed(&[
            item("26.928.21956", "12404", "13.0", true),
            item("26.924.22138", "11645", "13.0", false),
        ]);
        assert_eq!(
            parse_appcast(&xml, "x86_64", None).unwrap(),
            Some("26.924.22138".into())
        );
        assert_eq!(
            parse_appcast(
                &feed(&[item("26.928.21956", "12404", "13.0", true)]),
                "x86_64",
                None
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn appcast_skips_items_requiring_a_newer_macos() {
        let xml = feed(&[
            item("26.928.21956", "12404", "15.0", true),
            item("26.924.22138", "11645", "13.0", true),
        ]);
        assert_eq!(
            parse_appcast(&xml, "aarch64", Some("14.6")).unwrap(),
            Some("26.924.22138".into())
        );
        assert_eq!(
            parse_appcast(&xml, "aarch64", None).unwrap(),
            Some("26.928.21956".into())
        );
    }

    #[test]
    fn appcast_ignores_cdata_and_nested_elements() {
        let xml = format!(
            "{FEED_HEAD}{}{FEED_TAIL}",
            r#"
        <item>
            <description><![CDATA[<sparkle:shortVersionString>99.0</sparkle:shortVersionString>]]></description>
            <sparkle:shortVersionString>26.928.21956</sparkle:shortVersionString>
            <sparkle:deltas>
                <sparkle:shortVersionString>98.0</sparkle:shortVersionString>
            </sparkle:deltas>
        </item>"#
        );
        assert_eq!(
            parse_appcast(&xml, "aarch64", None).unwrap(),
            Some("26.928.21956".into())
        );
    }

    #[test]
    fn appcast_malformed_is_error_and_empty_is_none() {
        assert!(parse_appcast("<rss><channel><item></channel></rss>", "aarch64", None).is_err());
        assert_eq!(parse_appcast(&feed(&[]), "aarch64", None).unwrap(), None);
    }

    #[test]
    fn macos_feed_follows_the_build_architecture() {
        assert_eq!(macos_feed_url("aarch64"), MAC_APPCAST_ARM64_URL);
        assert_eq!(macos_feed_url("x86_64"), MAC_APPCAST_X64_URL);
    }

    const MANIFEST: &str = r#"{"schemaVersion":1,"buildVersion":"26.928.2636.0","storeProductId":"9PLM9XGG6VKS","packageIdentity":"OpenAI.Codex"}"#;

    #[test]
    fn windows_manifest_accepts_the_prod_store_package() {
        assert_eq!(parse_windows_manifest(MANIFEST).unwrap(), "26.928.2636.0");
    }

    #[test]
    fn windows_manifest_rejects_other_packages_schemas_and_versions() {
        for bad in [
            MANIFEST.replace("9PLM9XGG6VKS", "9N8CJ4W95TBZ"),
            MANIFEST.replace("OpenAI.Codex", "OpenAI.CodexBeta"),
            MANIFEST.replace("\"schemaVersion\":1", "\"schemaVersion\":2"),
            MANIFEST.replace("26.928.2636.0", "latest"),
            "not json".to_string(),
        ] {
            assert!(parse_windows_manifest(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn desktop_fields_skip_the_network_when_unchecked() {
        let fields = desktop_update_fields("sparkle", "26.924.22138", None, false);
        assert_eq!(fields.update_status, "unchecked");
        assert_eq!(fields.latest_version, None);
        assert_eq!(fields.update_source.as_deref(), Some("sparkle"));
        assert!(!fields.update_supported);
    }

    #[test]
    fn desktop_fields_keep_in_app_guidance_when_the_lookup_fails() {
        let fields = desktop_update_fields(
            "msstore",
            "26.924.2600.0",
            Some(Err("HTTP 503".into())),
            true,
        );
        assert_eq!(fields.update_status, "appManaged");
        assert_eq!(fields.latest_version, None);
        assert!(!fields.update_supported);
        assert!(fields.update_reason.unwrap().contains("HTTP 503"));
    }

    #[test]
    fn desktop_fields_report_available_and_one_click_support() {
        let latest = || Some(Ok("26.928.21956".to_string()));
        let manual = desktop_update_fields("sparkle", "26.924.22138", latest(), false);
        assert_eq!(manual.update_status, "available");
        assert_eq!(manual.latest_version.as_deref(), Some("26.928.21956"));
        assert!(!manual.update_supported);
        assert!(manual.update_reason.is_some());
        let one_click = desktop_update_fields("msstore", "26.924.22138", latest(), true);
        assert!(one_click.update_supported);
    }

    #[test]
    fn desktop_fields_report_current_and_unknown() {
        let current = desktop_update_fields(
            "msstore",
            "26.928.2636.0",
            Some(Ok("26.928.2636".into())),
            true,
        );
        assert_eq!(current.update_status, "current");
        assert!(!current.update_supported);
        let unknown =
            desktop_update_fields("sparkle", "dev", Some(Ok("26.928.21956".into())), false);
        assert_eq!(unknown.update_status, "unknown");
        assert!(!unknown.update_supported);
    }

    async fn mock_feeds() -> (MockServer, reqwest::Client) {
        let server = MockServer::start().await;
        for (route, response) in [
            (
                "/appcast.xml",
                ResponseTemplate::new(200).set_body_string(feed(&[item(
                    "26.928.21956",
                    "12404",
                    "13.0",
                    true,
                )])),
            ),
            (
                "/manifest.json",
                ResponseTemplate::new(200).set_body_string(MANIFEST),
            ),
            (
                "/error",
                ResponseTemplate::new(503).set_body_string(MANIFEST),
            ),
            (
                "/bad",
                ResponseTemplate::new(200).set_body_string("not a feed"),
            ),
            (
                "/slow",
                ResponseTemplate::new(200)
                    .set_body_string(MANIFEST)
                    .set_delay(Duration::from_secs(1)),
            ),
        ] {
            Mock::given(path(route))
                .respond_with(response)
                .mount(&server)
                .await;
        }
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        (server, client)
    }

    #[tokio::test]
    async fn macos_lookup_reads_the_feed_and_reports_failures() {
        let (server, client) = mock_feeds().await;
        let timeout = Duration::from_millis(200);
        let url = |route: &str| format!("{}/{route}", server.uri());
        assert_eq!(
            fetch_macos_latest(&client, &url("appcast.xml"), "aarch64", None, timeout)
                .await
                .unwrap(),
            "26.928.21956"
        );
        let error = fetch_macos_latest(&client, &url("error"), "aarch64", None, timeout)
            .await
            .unwrap_err();
        assert!(error.contains("503"), "{error}");
        for route in ["bad", "slow"] {
            assert!(
                fetch_macos_latest(&client, &url(route), "aarch64", None, timeout)
                    .await
                    .is_err(),
                "{route}"
            );
        }
    }

    #[tokio::test]
    async fn windows_lookup_reads_the_manifest_and_reports_failures() {
        let (server, client) = mock_feeds().await;
        let timeout = Duration::from_millis(200);
        let url = |route: &str| format!("{}/{route}", server.uri());
        assert_eq!(
            fetch_windows_latest(&client, &url("manifest.json"), timeout)
                .await
                .unwrap(),
            "26.928.2636.0"
        );
        let error = fetch_windows_latest(&client, &url("error"), timeout)
            .await
            .unwrap_err();
        assert!(error.contains("503"), "{error}");
        for route in ["bad", "slow"] {
            assert!(
                fetch_windows_latest(&client, &url(route), timeout)
                    .await
                    .is_err(),
                "{route}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "Live OpenAI feed probe; run manually to catch format drift"]
    async fn manual_live_chatgpt_feeds_parse() {
        let client = reqwest::Client::new();
        for arch in ["aarch64", "x86_64"] {
            let latest =
                fetch_macos_latest(&client, macos_feed_url(arch), arch, None, FETCH_TIMEOUT)
                    .await
                    .unwrap();
            println!("macOS {arch}: {latest}");
        }
        let latest = fetch_windows_latest(&client, WINDOWS_MANIFEST_URL, FETCH_TIMEOUT)
            .await
            .unwrap();
        println!("Windows: {latest}");
    }
}
