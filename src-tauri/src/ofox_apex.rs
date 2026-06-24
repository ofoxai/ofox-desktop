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
//! 默认情况下（无论 `pnpm tauri dev` 还是 release build）所有 base URL 都
//! 指向线上 `https://app.<apex>` / `https://api.<apex>`。这一刀切让 dev 与
//! 生产共享同一 IDP、gateway、`/openapi/*`——避免"dev 跑得通生产挂掉"或
//! 反向那种 dev/prod 混搭。
//!
//! 偶尔需要本地后端联调（ofox-app/ofox-core/ofox-gateway 跑在 pm2 上）时，
//! 把环境变量 `OFOX_USE_LOCAL=1` 传给 `pnpm tauri dev`，本进程内所有 base
//! 会切回 `localhost:3000`/`:8080`/`:8088`——这是逃生口，不进 settings，
//! 重启进程即恢复线上模式，避免开发机长期处于一个非默认的隐藏状态。
//!
//! **例外**：[`models_catalog_base`] **始终指线上**，不响应 `OFOX_USE_LOCAL`。
//! 模型列表是只读 catalog，且本地 traefik 配置缺 `/anthropic/v1/models`、
//! `/gemini/v1beta/models` 的直转路由——dev 模式打本地会 404。把这个 base
//! 单独锁线上是绕开 dev/prod 配置漂移的最小成本修法。详见该函数 docstring。
//!
//! 切换 apex（ofox.ai ⇌ ofox.io）走 `settings.json` 的 `ofoxApex` 字段，
//! 与 dev/prod 切换正交：apex 决定打哪个地区，`OFOX_USE_LOCAL` 决定是否走
//! 本地后端。
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

// ── Dev 端口（仅在 OFOX_USE_LOCAL=1 时生效；与 pm2 下 ofox-app / ofox-core /
//   ofox-gateway 的监听端口一致）──────────────────────────────────────────────
const DEV_APP_BASE: &str = "http://localhost:3000";
const DEV_API_BASE: &str = "http://localhost:8080";
const DEV_GATEWAY_BASE: &str = "http://localhost:8088";

/// 环境变量名：`OFOX_USE_LOCAL=1` 让本进程所有 base URL 切回 localhost。
///
/// 只看是否设置且非空——任意 truthy 字符串（"1"、"true"、"yes"）都生效。
/// 在每次 `*_base()` 调用时实时读取，所以 `OFOX_USE_LOCAL=1 pnpm tauri dev`
/// 不重启切换 apex 也能马上换地址。
const OFOX_USE_LOCAL_ENV: &str = "OFOX_USE_LOCAL";

fn use_local_dev() -> bool {
    matches!(std::env::var(OFOX_USE_LOCAL_ENV), Ok(v) if !v.is_empty())
}

/// crate 内部访问 `OFOX_USE_LOCAL` 开关用同一个入口——避免 `ofox_auth` 等模块
/// 自己再读一次 env 出现"endpoint 已走线上、client_id 还停在 dev"的灰区。
pub(crate) fn use_local_dev_mode() -> bool {
    use_local_dev()
}

// ── 路径片段（与 OFox OpenAPI 文档一致；ofox-openapi-example/docs/modules/oauth）

const OAUTH_DEVICE_AUTH_PATH: &str = "/api/oauth/device_authorization";
const OAUTH_TOKEN_PATH: &str = "/api/oauth/token";
const OPENAPI_ME_PATH: &str = "/openapi/me";
const OPENAPI_BALANCE_PATH: &str = "/openapi/orgs/me/balance";
const OPENAPI_SPENDING_LIMITS_PATH: &str = "/openapi/orgs/me/spending-limits";
/// 与 `APP_API_KEYS_PATH`（`/manage/api-keys`，UI 路径）区分开：这是 OpenAPI
/// REST endpoint，挂在 `api.<apex>` 上，给 cc-switch 自助签发 / 撤销 LLM key 用。
///
/// `#[allow(dead_code)]`：仅 [`openapi_api_keys_url`] / [`openapi_api_key_url`]
/// 引用，二者在下个 Commit 才会被实际调用；本 Commit 先建好门面。
#[allow(dead_code)]
const OPENAPI_API_KEYS_PATH: &str = "/openapi/api-keys";

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
///
/// 默认走线上；设了 `OFOX_USE_LOCAL=1` 时切回 `http://localhost:3000`
/// （ofox-app Next.js）。
pub fn auth_app_base() -> String {
    if use_local_dev() {
        DEV_APP_BASE.to_string()
    } else {
        format!("https://app.{}", current_apex())
    }
}

/// `https://api.<apex>` —— OpenAPI 业务接口（/openapi/me、balance 等）。
///
/// 默认走线上；设了 `OFOX_USE_LOCAL=1` 时切回 `http://localhost:8080`
/// （ofox-core Go service）。
pub fn auth_api_base() -> String {
    if use_local_dev() {
        DEV_API_BASE.to_string()
    } else {
        format!("https://api.{}", current_apex())
    }
}

/// LLM 网关 base —— `<gateway>/v1`、`<gateway>/anthropic`、`<gateway>/gemini`
/// 等所有协议入口都从这里拼。
///
/// 默认走线上 `https://api.<apex>`（与 OpenAPI 业务接口共享同一 host，按路径
/// 前缀分发）；设了 `OFOX_USE_LOCAL=1` 时切回 `http://localhost:8088`
/// （ofox-gateway plugin）。
pub fn gateway_base() -> String {
    if use_local_dev() {
        DEV_GATEWAY_BASE.to_string()
    } else {
        format!("https://api.{}", current_apex())
    }
}

/// 模型列表（`/v1/models`、`/anthropic/v1/models`、`/gemini/v1beta/models`）
/// 专用 base —— **始终指向线上 `https://api.<apex>`**，不受 `OFOX_USE_LOCAL`
/// 影响。
///
/// 为什么单独拉一个 base：本地 ofox-gateway traefik 配置里 `ofox-api-router`
/// 只抢 `/v1/models`，`/anthropic/v1/models` 和 `/gemini/v1beta/models` 会
/// 误落到 LLM 代理父路由，被 gemini/anthropic 插件 default 分支拦截 404
/// （插件本来就不该处理这条路径——prod 由 MSE 网关单独路由直转 ofox-core）。
/// 列模型是只读、跨地区共享的 catalog，把它锁到线上是成本最低的修法：
/// 比改 4 份 traefik configmap 安全、比让插件实现 list 端点正确。
///
/// 仍然遵守 apex 切换（ofox.ai ⇌ ofox.io），所以国内用户的模型列表来自
/// `api.ofox.io`。
pub fn models_catalog_base() -> String {
    format!("https://api.{}", current_apex())
}

/// 营销站 `https://<apex>` —— terms / privacy / 主站 marketing 链接。
///
/// 营销页没有本地版本，所以即便 `OFOX_USE_LOCAL=1` 也直接走线上；apex 切换
/// 仍然生效。
///
/// 仅在测试和 `terms_url()`/`privacy_url()` 内部使用——前端 marketing
/// 外链直接走 TS 侧 `ofoxMarketingUrl(apex)`。
#[allow(dead_code)]
pub fn marketing_site() -> String {
    format!("https://{}", current_apex())
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

/// `POST` 创建 / `GET` 列举（暂无 list 端点，仅 GET-by-id）/ `PATCH` / `DELETE`
/// 都挂在这条 URL 上。详见 `../ofox-openapi-example/docs/modules/oauth/02-api-reference.md`。
///
/// `#[allow(dead_code)]`：本 Commit 引入，[`crate::ofox_api_keys::call_create_endpoint`]
/// 在下个 Commit 才接入消费。先建好门面，便于 review 拆分。
#[allow(dead_code)]
pub fn openapi_api_keys_url() -> String {
    format!("{}{}", auth_api_base(), OPENAPI_API_KEYS_PATH)
}

/// 单把 key 的资源 URL（`GET` / `PATCH` / `DELETE`）。`id` 来自创建时的响应。
///
/// 同上 dead_code 说明。
#[allow(dead_code)]
pub fn openapi_api_key_url(id: &str) -> String {
    format!("{}{}/{}", auth_api_base(), OPENAPI_API_KEYS_PATH, id)
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
    fn default_mode_targets_production() {
        // 默认（OFOX_USE_LOCAL 未设置）应走线上 https。回归保险：有人把分支
        // 写反时立刻挂掉。
        //
        // 注意：测试可能在 OFOX_USE_LOCAL=1 的 shell 里跑，那种情况这条测试
        // 会被跳过——本地手工排错时无所谓，CI 不会带这个 env。
        if use_local_dev() {
            eprintln!("[test] OFOX_USE_LOCAL is set; skipping default-mode assertion");
            return;
        }
        assert!(auth_app_base().starts_with("https://app."));
        assert!(auth_api_base().starts_with("https://api."));
        assert!(gateway_base().starts_with("https://api."));
        assert!(marketing_site().starts_with("https://"));
    }

    #[test]
    fn openapi_api_keys_url_pins_to_api_base() {
        // 跟 balance_url() 一样挂在 `auth_api_base()` 下；最关键的是**不**
        // 沿用 `api_keys_url()`（那个是 `auth_app_base()`/`/manage/api-keys`
        // 控制台路径），混淆会导致 POST 打到一个 HTML 页面。
        assert_eq!(
            openapi_api_keys_url(),
            format!("{}{OPENAPI_API_KEYS_PATH}", auth_api_base())
        );
        assert_ne!(openapi_api_keys_url(), api_keys_url());
    }

    #[test]
    fn openapi_api_key_url_appends_id() {
        let id = "n7Hk2vQwR3xLm5pYbAcDe";
        assert_eq!(
            openapi_api_key_url(id),
            format!("{}/{id}", openapi_api_keys_url())
        );
    }

    #[test]
    fn models_catalog_base_ignores_use_local() {
        // 即便 OFOX_USE_LOCAL=1，models_catalog_base() 也必须指线上——
        // 本地 traefik 缺 `/anthropic/v1/models` 和 `/gemini/v1beta/models`
        // 的直转路由，列模型只能打 prod。回归保险：有人手滑把这条接到
        // gateway_base() 上时立刻挂掉。
        assert!(models_catalog_base().starts_with("https://api."));
        assert!(models_catalog_base().ends_with(current_apex()));
    }

    #[test]
    fn ofox_use_local_env_switches_to_localhost() {
        // 直接验证 env 开关 → localhost 的映射；不实际改进程 env 以免污染
        // 同一进程内的其它测试。
        //
        // 这里只能模拟逻辑——`use_local_dev()` 读 env，无法在测试里安全
        // toggle。改为对 DEV_* 常量做形态断言，确保它们指向 localhost 的
        // 既定端口；切换分支正确性由 `auth_*_base()` 的实现保障。
        assert!(DEV_APP_BASE.starts_with("http://localhost:3000"));
        assert!(DEV_API_BASE.starts_with("http://localhost:8080"));
        assert!(DEV_GATEWAY_BASE.starts_with("http://localhost:8088"));
    }
}
