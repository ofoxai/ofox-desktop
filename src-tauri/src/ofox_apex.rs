//! OFox apex（域名根）单一开关面。
//!
//! OFox 平台同时部署在 `ofox.ai`（海外）与 `ofox.io`（国内镜像），同一 cc-switch
//! 桌面端要能根据用户所在地区无缝切换。本模块是这个能力的**唯一**入口：
//!
//!   - [`current_apex`]    返回当前 apex 字符串（`"ofox.ai"` / `"ofox.io"`）
//!   - [`auth_app_base`] / [`auth_api_base`] / [`gateway_base`] / [`marketing_site`]
//!                        返回拼好的 base URL（dev/prod 已分支处理）
//!   - 若干 `*_url` 组合器把常用路径（device authorization、token、wallet、
//!                        register、terms 等）拼到对应 base 上
//!   - [`detect_apex_from_geo`] / [`ensure_apex_resolved`] 实现首次启动时
//!                        通过 ip-api.com 决定默认 apex
//!
//! ### Dev / Prod 分支
//!
//! `cfg!(debug_assertions) == true` （即 `pnpm tauri dev`、`cargo run`）时所有
//! base URL 仍指向 `localhost:3000`/`:8080`/`:8088`——这是 ofox-app / ofox-core /
//! ofox-gateway 在 pm2 下监听的 dev 端口，与之前 `ofox_auth.rs` 与
//! `ofox_endpoints.rs` 里的硬编码常量等价。release build 才会基于 apex 拼线上
//! `https://app.<apex>` / `https://api.<apex>`。
//!
//! 所以**只在线上才能验证 apex 切换的最终效果**。dev 下切换 apex 不会改变实际
//! 请求地址（前端 UI 仍可工作，DB reseed 仍跑通），方便本地开发不被 ip-api 误
//! 判到 ofox.io 卡住。
//!
//! ### 参考
//!
//! 域名规则与 ofox-studio 仓库 `src/shared/config/domains.ts` 保持一致：
//! `APEX_DOMAINS = ["ofox.ai", "ofox.io"]`，`DEFAULT_APEX = "ofox.ai"`，
//! 路径规则 `https://app.<apex>`、`https://api.<apex>`、`https://<apex>`。

use serde::Deserialize;

/// 已知合法 apex 列表。新增 apex 时只改这一处 + ofox-studio 同步即可。
const KNOWN_APEXES: &[&str] = &["ofox.ai", "ofox.io"];

/// 默认 apex。`current_apex()` 在 settings 还没填好时回退到这个值，与
/// ofox-studio 的 `DEFAULT_APEX` 一致。也在 ip-api 探测失败时使用。
const DEFAULT_APEX: &str = "ofox.ai";

// ── Dev 端口（与 ofox_auth.rs / ofox_endpoints.rs 历史常量保持一致）──────────
const DEV_APP_BASE: &str = "http://localhost:3000";
const DEV_API_BASE: &str = "http://localhost:8080";
const DEV_GATEWAY_BASE: &str = "http://localhost:8088";

// ── 路径片段（与 OFox OpenAPI 文档一致；ofox-openapi-example/docs/modules/oauth）

const OAUTH_DEVICE_AUTH_PATH: &str = "/api/oauth/device_authorization";
const OAUTH_TOKEN_PATH: &str = "/api/oauth/token";
const OPENAPI_ME_PATH: &str = "/openapi/me";
const OPENAPI_BALANCE_PATH: &str = "/openapi/orgs/me/balance";
const OPENAPI_SPENDING_LIMITS_PATH: &str = "/openapi/orgs/me/spending-limits";

// 以下路径仅在 Rust 单元测试 / 将来后端外链场景中用到——前端外链直接走
// `src/lib/ofoxUrls.ts`，不经 Rust。allow(dead_code) 收 warning，不删除是为
// 了让 `wallet_url()` 这种自描述命名继续做"语义锚点"，回头要加新 case
// 时不用到处复制粘贴 `/manage/wallet` 字面量。
#[allow(dead_code)]
const APP_REGISTER_PATH: &str = "/register";
#[allow(dead_code)]
const APP_DASHBOARD_PATH: &str = "/dashboard";
#[allow(dead_code)]
const APP_WALLET_PATH: &str = "/manage/wallet";
#[allow(dead_code)]
const APP_API_KEYS_PATH: &str = "/manage/api-keys";

#[allow(dead_code)]
const SITE_TERMS_PATH: &str = "/terms";
#[allow(dead_code)]
const SITE_PRIVACY_PATH: &str = "/privacy";

/// ip-api.com 接口超时。短一点避免国内 dev 用户冷启动被卡 30 秒。失败兜底是
/// `ofox.ai`，所以网络差时至多多等 5 秒就能进 LoginPage。
const GEO_PROBE_TIMEOUT_SECS: u64 = 5;

// ─────────────────────────────────────────────────────────────────────────
// Apex 解析与持久化
// ─────────────────────────────────────────────────────────────────────────

/// 当前 apex 字符串。
///
/// 决策优先级：
///   1. `settings.json` 里 `ofoxApex == Some("ofox.io")` → 返回 `"ofox.io"`
///   2. `settings.json` 里 `ofoxApex == Some("ofox.ai")` → 返回 `"ofox.ai"`
///   3. 其他情况（None、未知值在 normalize 已被清掉）→ 返回 [`DEFAULT_APEX`]
///
/// 返回 `&'static str`：因为只有两个合法值都是 `&'static`，`match` 后 leak
/// 静态字面量即可，无堆分配。这个函数会在大量请求构造路径时被调用，零分配很
/// 重要。
pub fn current_apex() -> &'static str {
    let settings = crate::settings::get_settings();
    match settings.ofox_apex.as_deref() {
        Some("ofox.io") => "ofox.io",
        Some("ofox.ai") => "ofox.ai",
        _ => DEFAULT_APEX,
    }
}

/// 给定 apex 是否是合法的已知值。供 Tauri 命令做参数校验。
pub fn is_known_apex(s: &str) -> bool {
    KNOWN_APEXES.contains(&s)
}

// ─────────────────────────────────────────────────────────────────────────
// Base URL 组合器（dev/prod 分支的唯一来源）
// ─────────────────────────────────────────────────────────────────────────

/// `https://app.<apex>` —— OAuth IDP 与控制台 (sign-in / consent / device flow)。
/// Dev 模式打 `http://localhost:3000`（ofox-app Next.js）。
pub fn auth_app_base() -> String {
    if cfg!(debug_assertions) {
        DEV_APP_BASE.to_string()
    } else {
        format!("https://app.{}", current_apex())
    }
}

/// `https://api.<apex>` —— OpenAPI 业务接口（/openapi/me、balance 等）。
/// Dev 模式打 `http://localhost:8080`（ofox-core Go service）。
pub fn auth_api_base() -> String {
    if cfg!(debug_assertions) {
        DEV_API_BASE.to_string()
    } else {
        format!("https://api.{}", current_apex())
    }
}

/// LLM 网关 base —— `<gateway>/v1`、`<gateway>/anthropic`、`<gateway>/gemini`
/// 等所有协议入口都从这里拼。
/// Dev 模式打 `http://localhost:8088`（ofox-gateway plugin）。
pub fn gateway_base() -> String {
    if cfg!(debug_assertions) {
        DEV_GATEWAY_BASE.to_string()
    } else {
        // 线上 api.<apex> 与 OpenAPI 业务接口是同一个 Traefik 入口分流的；网关
        // 与业务接口在 release 共享同一 host，按路径前缀分发。
        format!("https://api.{}", current_apex())
    }
}

/// 营销站 `https://<apex>` —— terms / privacy / 主站 marketing 链接。
/// Dev 模式直接走线上 `https://ofox.ai`：营销页没有本地版本，dev 跑联调时
/// 也不会真的点这些链接。
///
/// 仅在测试和 `terms_url()`/`privacy_url()` 内部使用——前端 marketing
/// 外链直接走 TS 侧 `ofoxMarketingUrl(apex)`。
#[allow(dead_code)]
pub fn marketing_site() -> String {
    if cfg!(debug_assertions) {
        format!("https://{DEFAULT_APEX}")
    } else {
        format!("https://{}", current_apex())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 高层路径组合器（OAuth 流程）
// ─────────────────────────────────────────────────────────────────────────

pub fn device_auth_url() -> String {
    format!("{}{}", auth_app_base(), OAUTH_DEVICE_AUTH_PATH)
}

pub fn token_url() -> String {
    format!("{}{}", auth_app_base(), OAUTH_TOKEN_PATH)
}

pub fn user_info_url() -> String {
    format!("{}{}", auth_api_base(), OPENAPI_ME_PATH)
}

pub fn balance_url() -> String {
    format!("{}{}", auth_api_base(), OPENAPI_BALANCE_PATH)
}

pub fn spending_limits_url() -> String {
    format!("{}{}", auth_api_base(), OPENAPI_SPENDING_LIMITS_PATH)
}

// ─────────────────────────────────────────────────────────────────────────
// 高层路径组合器（外链 / 控制台 / 营销）
// ─────────────────────────────────────────────────────────────────────────
//
// 这一组目前**仅在 Rust 单元测试中被引用**。前端打开外链都是从 TS 侧调
// `settingsApi.openExternal(ofoxWalletUrl(apex))`（见 `src/lib/ofoxUrls.ts`），
// 不经 Rust。保留这些 helpers 是为了将来后端要做"systray 直接打开钱包"
// 这类场景时不必再到处拼字符串——dead_code allow 收掉 warning。

#[allow(dead_code)]
pub fn register_url() -> String {
    format!("{}{}", auth_app_base(), APP_REGISTER_PATH)
}

#[allow(dead_code)]
pub fn dashboard_url() -> String {
    format!("{}{}", auth_app_base(), APP_DASHBOARD_PATH)
}

#[allow(dead_code)]
pub fn wallet_url() -> String {
    format!("{}{}", auth_app_base(), APP_WALLET_PATH)
}

#[allow(dead_code)]
pub fn api_keys_url() -> String {
    format!("{}{}", auth_app_base(), APP_API_KEYS_PATH)
}

#[allow(dead_code)]
pub fn terms_url() -> String {
    format!("{}{}", marketing_site(), SITE_TERMS_PATH)
}

#[allow(dead_code)]
pub fn privacy_url() -> String {
    format!("{}{}", marketing_site(), SITE_PRIVACY_PATH)
}

// ─────────────────────────────────────────────────────────────────────────
// ip-api 探测：首次启动决定默认 apex
// ─────────────────────────────────────────────────────────────────────────

/// ip-api.com 的响应结构（仅取我们用到的字段）。完整 schema 见
/// http://ip-api.com/json，免费版用 HTTP 即可，HTTPS 需要付费 plan。
#[derive(Debug, Deserialize)]
struct IpApiResponse {
    /// ISO-3166-1 alpha-2 国家代码，如 `"CN"`、`"US"`、`"JP"`。
    /// 失败时 `status == "fail"`，`country_code` 不返回，所以用 Option。
    #[serde(rename = "countryCode")]
    country_code: Option<String>,
}

/// 探测当前网络出口所在国家，决定默认 apex。
///
/// 规则：`countryCode == "CN"` → `"ofox.io"`，其他/失败 → `"ofox.ai"`。
///
/// 任何错误（DNS、超时、JSON 解析、HTTP 非 2xx）都吞掉、记一条 warn，返回
/// `DEFAULT_APEX`——首次安装如果网络糟糕，至少能进 LoginPage 给用户手动切的
/// 机会，不会卡死。
///
/// `client` 由调用方传入，复用全局带代理配置的 `reqwest::Client`
/// （`crate::proxy::http_client::get()`）。这点对国内用户尤其重要：他们如果
/// 配了 HTTP 代理走出墙，ip-api 看到的是代理出口 IP，会判成海外——这恰好
/// 是用户的真实意图，不应该绕过代理。
pub async fn detect_apex_from_geo(client: &reqwest::Client) -> &'static str {
    let resp = match client
        .get("http://ip-api.com/json")
        .timeout(std::time::Duration::from_secs(GEO_PROBE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[OfoxApex] ip-api probe failed: {e}; falling back to {DEFAULT_APEX}");
            return DEFAULT_APEX;
        }
    };

    if !resp.status().is_success() {
        log::warn!(
            "[OfoxApex] ip-api returned non-success status {}; falling back to {DEFAULT_APEX}",
            resp.status()
        );
        return DEFAULT_APEX;
    }

    let parsed: IpApiResponse = match resp.json().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[OfoxApex] ip-api JSON parse failed: {e}; falling back to {DEFAULT_APEX}");
            return DEFAULT_APEX;
        }
    };

    match parsed.country_code.as_deref() {
        Some("CN") => {
            log::info!("[OfoxApex] ip-api countryCode=CN → ofox.io");
            "ofox.io"
        }
        Some(code) => {
            log::info!("[OfoxApex] ip-api countryCode={code} → ofox.ai");
            "ofox.ai"
        }
        None => {
            log::warn!(
                "[OfoxApex] ip-api response missing countryCode; falling back to {DEFAULT_APEX}"
            );
            DEFAULT_APEX
        }
    }
}

/// 启动钩子：如果 settings 里 `ofoxApexResolved == Some(true)`，立刻返回；
/// 否则跑一次 ip-api 探测、把结果连同 `resolved=true` 持久化到 settings.json。
///
/// 持久化失败只记 warn 不抛——下次启动会重试探测，体验是"多一次 5 秒延迟"，
/// 不会卡死任何流程。
///
/// 探测完成后通过 `app` emit `ofox-apex-changed` 事件，前端 hook 据此重读。
/// 即便此时还没有打开任何窗口，emit 也是非阻塞的（Tauri 内部 channel 缓冲）。
pub async fn ensure_apex_resolved(app: &tauri::AppHandle, client: &reqwest::Client) {
    let already_resolved = crate::settings::get_settings()
        .ofox_apex_resolved
        .unwrap_or(false);
    if already_resolved {
        log::debug!("[OfoxApex] already resolved (apex={}); skip probe", current_apex());
        return;
    }

    let detected = detect_apex_from_geo(client).await;

    if let Err(e) = crate::settings::mutate_settings(|s| {
        s.ofox_apex = Some(detected.to_string());
        s.ofox_apex_resolved = Some(true);
    }) {
        log::warn!("[OfoxApex] persist apex={detected} failed: {e}; will re-probe next launch");
        return;
    }

    log::info!("[OfoxApex] resolved apex={detected} (persisted)");

    use tauri::Emitter;
    if let Err(e) = app.emit("ofox-apex-changed", detected) {
        log::warn!("[OfoxApex] emit ofox-apex-changed failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_apex_matches_whitelist() {
        assert!(is_known_apex("ofox.ai"));
        assert!(is_known_apex("ofox.io"));
        assert!(!is_known_apex("ofox.evil"));
        assert!(!is_known_apex(""));
        // case-sensitive on purpose——normalize 在 settings 那层做，这里只校验
        // 已 normalize 过的值。
        assert!(!is_known_apex("OFOX.AI"));
    }

    #[test]
    fn url_composers_share_apex_in_release() {
        // dev 模式跑 cargo test 时这个测试只是冒烟：所有 base 都是 localhost。
        // 关键是 device_auth_url() / token_url() 都从 auth_app_base() 拼，
        // 不会出现一个走 ofox.ai 一个走 ofox.io 的情况。
        assert_eq!(
            device_auth_url(),
            format!("{}{OAUTH_DEVICE_AUTH_PATH}", auth_app_base())
        );
        assert_eq!(token_url(), format!("{}{OAUTH_TOKEN_PATH}", auth_app_base()));
        assert_eq!(
            wallet_url(),
            format!("{}{APP_WALLET_PATH}", auth_app_base())
        );
    }

    #[test]
    fn dev_mode_uses_localhost() {
        // 这个测试在 cargo test 下永远是 dev——`cfg!(debug_assertions)` true。
        // 显式 assert 防回归：有人把 dev 分支去掉就会立刻挂。
        if cfg!(debug_assertions) {
            assert!(auth_app_base().starts_with("http://localhost:3000"));
            assert!(auth_api_base().starts_with("http://localhost:8080"));
            assert!(gateway_base().starts_with("http://localhost:8088"));
        }
    }
}
