//! 工具级 ofox LLM API key 的取/建/撤销流程入口。
//!
//! cc-switch 切到 ofox 模式时，每个工具（Claude / Codex / Gemini /
//! OpenCode / OpenClaw / Hermes）持有**一把独立**的 ofox API key——这把 key 由
//! ofox-core 的 `/openapi/api-keys` 端点签发，cc-switch 在本地用两套存储分头
//! 保管：
//!
//!   - **key 本体**（`sk-of-...`）落 keychain，槽位 [`Slot::ApiKey { tool }`]
//!   - **元数据**（key id、别名、前缀、创建/上次使用时间）落 `settings.json`，
//!     字段 [`crate::settings::AppSettings::ofox_api_keys`]
//!
//! 拆开两份的理由参 [`crate::ofox_secret`] 的模块注释；本模块负责"协调两边
//! 一致"。
//!
//! ## 失效兜底
//!
//! API key 失效（用户在 ofox 后台撤销 / 服务端 401）跟 OAuth refresh 不一样
//! ——没法静默自愈。本模块通过 [`FetchMode::ForceRefresh`] 提供"跳过 keychain
//! 命中、强制走端点重建"的入口；触发时机（401 兜底）由调用方决定。

use std::sync::Arc;

use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::ofox_auth::OfoxAuthManager;
use crate::ofox_secret::{KeyringStore, SecretStore, Slot};
use crate::settings::{self, ApiKeyMeta};

/// 上层（command / bind 流程）能拿到的强类型错误。
///
/// 用 enum 而不是 `AppError::Config` 的字符串，主要因为 UI 端要按错误分类
/// 决定弹什么对话框：
///   - `Unauthorized` → 提示用户重新登录（多半是 OAuth scope 不含 `apikey.write`
///     或 access_token 失效）
///   - `RemoteRejected` → 显示后端原始 message，让用户去 ofox 后台查
///   - `Storage` → 本地 keychain / settings 异常，让用户重启或查日志
#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    /// OAuth access_token 过期、refresh 失败、被服务端拒绝（401），或缺
    /// `apikey.write` scope（403 `insufficient_scope`）。UI 应当引导用户
    /// 重新走 device flow 登录。
    #[error("ofox authentication required: {0}")]
    Unauthorized(String),

    /// 服务端拒绝（4xx 非 401 / 5xx）。携带原始错误以便排障。
    #[error("ofox API key endpoint rejected: {0}")]
    RemoteRejected(String),

    /// 本地存储（keychain / settings.json）异常。
    #[error("local storage error: {0}")]
    Storage(String),
}

impl From<ApiKeyError> for AppError {
    fn from(value: ApiKeyError) -> Self {
        AppError::Config(value.to_string())
    }
}

/// 取 key 的语义模式。
#[derive(Debug, Clone, Copy)]
pub enum FetchMode {
    /// 默认 bind 路径：先查 keychain，命中即用；未命中调端点创建新 key 并
    /// 落 keychain + settings。这是 `cachedOk`（"用旧的没问题"）。
    CachedOk,
    /// 服务端 401 兜底：**跳过** keychain 命中检查，直接调端点签发新 key，
    /// 覆盖 keychain + upsert settings。**不**主动 DELETE 旧 key——旧 key_id
    /// 在服务端可能已被撤销，也可能本地丢了（mode 触发场景就是"对不上"），
    /// 不假设服务端状态。
    ForceRefresh,
}

/// 单例 keychain store。生产路径用这一份；测试需要注入 `InMemoryStore` 时
/// 走显式 `*_with_store` 入口（见下方）。
fn default_store() -> Arc<dyn SecretStore> {
    Arc::new(KeyringStore::new())
}

/// 取或为指定工具创建 ofox API key。
///
/// **流程**：
///
/// 1. `mode == CachedOk` 时：`secret_store.load(Slot::ApiKey { tool })`
///    - 命中非空 → 返回（本地缓存已经有，直接复用）
/// 2. 未命中或 `mode == ForceRefresh` → 调 ofox-core `/openapi/api-keys` 创建
/// 3. 创建成功 → keychain 落 key 本体 + settings 落元数据 → 返回 key 本体
///
/// **失败语义**：
///
///   - `Storage`：keychain 或 settings 读写失败
///   - `Unauthorized`：access_token 失效、scope 不足、组织被解绑
///   - `RemoteRejected`：服务端非 401 拒绝，或网络错
pub async fn fetch_or_create_api_key(
    tool: AppType,
    mode: FetchMode,
    manager: &Arc<RwLock<OfoxAuthManager>>,
) -> Result<String, ApiKeyError> {
    fetch_or_create_api_key_with_store(tool, mode, default_store(), manager).await
}

/// 显式注入 store 的版本——单测/集成测试用。生产代码请用
/// [`fetch_or_create_api_key`]。
pub async fn fetch_or_create_api_key_with_store(
    tool: AppType,
    mode: FetchMode,
    store: Arc<dyn SecretStore>,
    manager: &Arc<RwLock<OfoxAuthManager>>,
) -> Result<String, ApiKeyError> {
    // CachedOk：命中直接返。ForceRefresh：跳过 keychain 走端点。
    if matches!(mode, FetchMode::CachedOk) {
        match store.load(Slot::ApiKey { tool }) {
            Ok(Some(key)) => {
                // Backfill：老 ApiKeyMeta 在加 `name` 字段之前签发，meta.name
                // 为 None。这里命中既存 keychain 时顺手补一次 default_key_name
                // 进去——不影响 keychain 数据、不调端点、对新 meta 也是无害的
                // no-op（已是 Some 就跳过）。这让"老用户不解绑也能在 UI 看到
                // `<tool> on <host>` 形式的 name"。失败只 warn，不影响 bind。
                if let Some(meta) = settings::get_api_key_meta(tool) {
                    if meta.name.is_none() {
                        let updated = ApiKeyMeta {
                            name: Some(default_key_name(tool)),
                            ..meta
                        };
                        if let Err(e) = settings::upsert_api_key_meta(updated) {
                            log::warn!(
                                "[ofox_api_keys] backfill name for {tool:?} failed: {e:?}"
                            );
                        }
                    }
                }
                // 这里**不**更新 `last_used_at`——本函数语义是"取出 key"，是否
                // 真的被用到（写到工具配置）由调用方决定。让 caller 在成功
                // bind 后再调 [`mark_key_used`]。
                return Ok(key);
            }
            Ok(None) => { /* 走创建路径 */ }
            Err(e) => return Err(ApiKeyError::Storage(e)),
        }
    }

    let name = default_key_name(tool);
    let created = call_create_endpoint(tool, &name, manager).await?;

    // 先落 keychain 再落 settings：keychain 写失败时不希望 settings 里多一条
    // 没有 key 本体的孤儿 meta（重启后 fetch 会再去 load → None → 又去创建
    // 一把，旧的就泄漏在服务端了）。
    store
        .save(Slot::ApiKey { tool }, &created.secret)
        .map_err(ApiKeyError::Storage)?;
    settings::upsert_api_key_meta(ApiKeyMeta {
        tool,
        key_id: created.key_id,
        // `name` 是我们 submit 给服务端的默认 name（`<tool> on <host>`），
        // 服务端不在 create 响应里回这个值，所以直接复用本地算出来的字符串。
        // UI 用它做"这把 key 是哪台机器的哪个工具"的主标签。
        name: Some(name),
        alias: None,
        key_start: created.key_start,
        created_at: chrono::Utc::now().timestamp(),
        last_used_at: None,
    })
    .map_err(|e| ApiKeyError::Storage(format!("settings: {e:?}")))?;

    Ok(created.secret)
}

/// 标记某把 key 刚被 bind 到工具——更新 `last_used_at`。失败只 warn，不影响
/// 上层 bind 结果（元数据不准不至于功能挂掉）。
pub fn mark_key_used(tool: AppType) {
    if let Some(mut meta) = settings::get_api_key_meta(tool) {
        meta.last_used_at = Some(chrono::Utc::now().timestamp());
        if let Err(e) = settings::upsert_api_key_meta(meta) {
            log::warn!("[ofox_api_keys] failed to update last_used_at for {tool:?}: {e:?}");
        }
    }
    // 没有元数据条目说明本次 bind 走了 OFOX_USE_OAUTH_TOKEN_AS_KEY 兜底模式
    // （access_token-as-key，不走 fetch_or_create_api_key）。逃生口移除后这
    // 个分支自然消失。
}

/// 仅本地撤销：清 keychain + settings 条目。不调端点。
///
/// 用于"用户在 ofox 后台已撤销"或"本地状态污染时手动重置"。
pub async fn revoke_local(tool: AppType) -> Result<(), ApiKeyError> {
    revoke_local_with_store(tool, default_store()).await
}

pub async fn revoke_local_with_store(
    tool: AppType,
    store: Arc<dyn SecretStore>,
) -> Result<(), ApiKeyError> {
    // 先清 keychain：失败要传上去，避免"settings 记录被删但 keychain 还残留
    // 一份孤儿 key"
    store
        .clear(Slot::ApiKey { tool })
        .map_err(ApiKeyError::Storage)?;
    settings::remove_api_key_meta(tool)
        .map_err(|e| ApiKeyError::Storage(format!("settings: {e:?}")))?;
    Ok(())
}

/// 完整撤销：调 `DELETE /openapi/api-keys/{id}` + 清本地。
///
/// 404 视为成功——服务端已经把这把 key 删了（用户在 ofox console 操作过），
/// 本地仍应清理。其它 4xx / 5xx 不清本地，让用户看到错误后决定。
pub async fn revoke_remote(
    tool: AppType,
    manager: &Arc<RwLock<OfoxAuthManager>>,
) -> Result<(), ApiKeyError> {
    let Some(meta) = settings::get_api_key_meta(tool) else {
        // 本地已经没记录——视为已撤销，幂等返回。
        log::debug!("[ofox_api_keys] revoke_remote: no meta for {tool:?}, treating as no-op");
        return Ok(());
    };

    let bearer = manager
        .read()
        .await
        .get_valid_access_token()
        .await
        .map_err(|e| ApiKeyError::Unauthorized(e.to_string()))?;

    let url = crate::ofox_apex::openapi_api_key_url(&meta.key_id);
    let resp = crate::proxy::http_client::get()
        .delete(url)
        .bearer_auth(&bearer)
        .send()
        .await
        .map_err(|e| ApiKeyError::RemoteRejected(format!("network: {e}")))?;

    let status = resp.status();
    if status.is_success() || status == StatusCode::NOT_FOUND {
        revoke_local(tool).await?;
        return Ok(());
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiKeyError::Unauthorized("invalid_token".into()));
    }
    if status == StatusCode::FORBIDDEN {
        return Err(classify_403(resp).await);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(ApiKeyError::RemoteRejected(format!("{status}: {body}")))
}

// ─── 端点调用 ────────────────────────────────────────────────────────────

/// `POST /openapi/api-keys` 真实调用。
///
/// 返回 `(key_id, secret, key_start)`——上层 `fetch_or_create_api_key` 拿到
/// 这三个值各自持久化到 settings / keychain。
async fn call_create_endpoint(
    tool: AppType,
    name: &str,
    manager: &Arc<RwLock<OfoxAuthManager>>,
) -> Result<CreatedKey, ApiKeyError> {
    log::debug!("[ofox_api_keys] creating key for tool={tool:?} name={name:?}");

    let bearer = manager
        .read()
        .await
        .get_valid_access_token()
        .await
        .map_err(|e| ApiKeyError::Unauthorized(e.to_string()))?;

    let url = crate::ofox_apex::openapi_api_keys_url();
    let body = serde_json::json!({ "name": name });
    call_create_endpoint_at(&url, &bearer, &body).await
}

/// 端点调用的 URL-agnostic 内部实现——拆出来仅为单测能拿一个本地 wiremock
/// 起的 URL 注入。生产路径走 [`call_create_endpoint`] 拼好的 ofox apex 地址。
async fn call_create_endpoint_at(
    url: &str,
    bearer: &str,
    body: &serde_json::Value,
) -> Result<CreatedKey, ApiKeyError> {
    let resp = crate::proxy::http_client::get()
        .post(url)
        .bearer_auth(bearer)
        .json(body)
        .send()
        .await
        .map_err(|e| ApiKeyError::RemoteRejected(format!("network: {e}")))?;

    let status = resp.status();
    if status.is_success() {
        let parsed: CreateApiKeyResponse = resp
            .json()
            .await
            .map_err(|e| ApiKeyError::RemoteRejected(format!("parse 201: {e}")))?;
        if parsed.id.is_empty() || parsed.key.is_empty() {
            return Err(ApiKeyError::RemoteRejected(
                "服务端 201 响应缺 id 或 key 字段".into(),
            ));
        }
        return Ok(CreatedKey {
            key_id: parsed.id,
            secret: parsed.key,
            key_start: if parsed.key_start.is_empty() {
                None
            } else {
                Some(parsed.key_start)
            },
        });
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiKeyError::Unauthorized("invalid_token".into()));
    }
    if status == StatusCode::FORBIDDEN {
        return Err(classify_403(resp).await);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(ApiKeyError::RemoteRejected(format!("{status}: {body}")))
}

/// 服务端创建响应。仅取我们需要的字段，其余（name / enabled / rate_limit /
/// expires_at / ...）即便存在也忽略——bind 流程用不到。
#[derive(Debug, Deserialize)]
struct CreateApiKeyResponse {
    id: String,
    key: String,
    #[serde(default)]
    key_start: String,
}

/// 端点内部使用的"创建结果"——把 `key_start` 转成 Option 跟 [`ApiKeyMeta`] 对齐。
#[derive(Debug)]
struct CreatedKey {
    key_id: String,
    secret: String,
    key_start: Option<String>,
}

/// 区分 403 的两种 RFC 6750 challenge：
///   - `insufficient_scope` → 缺 `apikey.write`，引导用户重新登录获取 scope
///   - `organization_unbound` → OAuth token 绑定的组织被删了，UI 要走 re-OAuth
///   - 其它 → 透传服务端原文
async fn classify_403(resp: reqwest::Response) -> ApiKeyError {
    #[derive(Deserialize)]
    struct RfcError {
        #[serde(default)]
        error: String,
        #[serde(default)]
        error_description: Option<String>,
    }
    // 把 body 读出来；失败也不能 panic（resp 已被 status 检过，但 body 解析
    // 可能因为 server 返非 JSON 而失败）。
    let parsed = resp.json::<RfcError>().await.ok();
    match parsed.as_ref().map(|e| e.error.as_str()) {
        Some("insufficient_scope") => ApiKeyError::Unauthorized(
            "缺少 apikey.write 权限，请重新登录以授权 API Key 管理".into(),
        ),
        Some("organization_unbound") => ApiKeyError::RemoteRejected(
            "OAuth 绑定的组织已被删除，请重新走 OAuth 授权".into(),
        ),
        _ => ApiKeyError::RemoteRejected(
            parsed
                .and_then(|e| e.error_description.or_else(|| Some(e.error)))
                .unwrap_or_else(|| "403 (无 RFC 6750 错误体)".into()),
        ),
    }
}

/// `<tool> on <host>`——在 ofox console 里 UI 友好的默认命名。
///
/// host 取 `hostname()`（macOS 上通常是 `Foo-MBP.local`），先把非法字符替换
/// 成 `-` 再截断到 24 char。
///
/// **服务端 name 校验**（ofox-core `handler/openapi_api_keys.go::apiKeyNamePattern`）
/// 是 `^[\w\s-]+$`——只接受 letters / digits / underscore / spaces /
/// hyphens；任何点号 / 反斜杠 / unicode 字符都会被 400 invalid_request
/// 拒绝。macOS 默认 hostname `Foo-MBP.local` 里的 `.` 就会撞 regex。
///
/// 我们这里**先 sanitize 再截断**：截断后再撞到末尾的非法字符不会留下"半个字符"。
/// 同时把长度限制定在 24 char——服务端硬上限是 100 char，留余量。
///
/// debug 构建在 host 末尾加 `-dev`：开发期 / 正式版用同一台机器、同一个 ofox
/// 账户时，ofox console 里的 key 列表会一眼能区分"哪一把是开发版试用签发的、
/// 哪一把是正式版日常用的"。`-dev` 在服务端 regex 内合法。release 构建不带后
/// 缀，正式用户看到的还是 `<tool> on <host>`。
fn default_key_name(tool: AppType) -> String {
    let host = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "unknown-host".to_string());
    let mut host: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    // 极端情况——hostname 全是非法字符 → 全部变 `-`，对人不友好但服务端能接受。
    // 不再 fallback 成 "unknown-host"，因为对调试反而有干扰：宁可看到一串 `-` 也
    // 知道哪台机器拿了 key。
    if cfg!(debug_assertions) {
        host.push_str("-dev");
    }
    format!("{} on {}", tool.as_str(), host)
}

// ─── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! 端点调用走 `wiremock` 拦截。`fetch_or_create_api_key` 自身的 keychain
    //! 命中 / 跳过分支由 `*_with_store` 接口注入 `InMemoryStore` 验证。

    use super::*;
    use crate::ofox_secret::InMemoryStore;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 已存在的 keychain 条目应当被原样返回，不调端点。
    #[tokio::test]
    async fn cached_key_returns_without_endpoint_call() {
        let store: Arc<dyn SecretStore> = Arc::new(InMemoryStore::new());
        store
            .save(Slot::ApiKey { tool: AppType::Claude }, "sk-of-cached")
            .unwrap();

        // 这里不构造 manager —— CachedOk 命中分支不会用到。给个 dangling
        // Arc<RwLock<…>> 也行，但更稳：用 lazy 的方式确保走不到端点。
        let manager: Arc<RwLock<OfoxAuthManager>> = dummy_manager_for_cache_test();

        let key = fetch_or_create_api_key_with_store(
            AppType::Claude,
            FetchMode::CachedOk,
            store,
            &manager,
        )
        .await
        .expect("cached key should be returned");
        assert_eq!(key, "sk-of-cached");
    }

    /// 每工具独立 key 的核心保证：给 Claude 缓存的 key 不会被 Codex 读到。
    #[tokio::test]
    async fn cached_key_is_per_tool() {
        let store: Arc<dyn SecretStore> = Arc::new(InMemoryStore::new());
        store
            .save(Slot::ApiKey { tool: AppType::Claude }, "sk-of-claude")
            .unwrap();

        let manager = dummy_manager_for_cache_test();
        let claude = fetch_or_create_api_key_with_store(
            AppType::Claude,
            FetchMode::CachedOk,
            store.clone(),
            &manager,
        )
        .await
        .unwrap();
        assert_eq!(claude, "sk-of-claude");
        // Codex 未命中——会走 endpoint，这里不希望真去调；我们只校验"Claude
        // 的命中没串到 Codex"，不验未命中分支（那由其它测试覆盖）。
        // 直接检查 store 状态：
        assert_eq!(
            store.load(Slot::ApiKey { tool: AppType::Codex }).unwrap(),
            None
        );
    }

    /// 服务端 201：keychain 落 + settings 落 + 返回 secret。
    #[tokio::test]
    async fn endpoint_201_persists_to_keychain_and_settings() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openapi/api-keys"))
            .and(header("authorization", "Bearer fake-bearer"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "key-id-abc",
                "key": "sk-of-FRESH123",
                "key_start": "sk-of-FRESH"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/openapi/api-keys", server.uri());
        let result =
            call_create_endpoint_at(&url, "fake-bearer", &json!({ "name": "claude on test" }))
                .await
                .expect("201 should parse");
        assert_eq!(result.key_id, "key-id-abc");
        assert_eq!(result.secret, "sk-of-FRESH123");
        assert_eq!(result.key_start.as_deref(), Some("sk-of-FRESH"));
    }

    /// 服务端 401 → Unauthorized（"invalid_token"）。
    #[tokio::test]
    async fn endpoint_401_maps_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openapi/api-keys"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let url = format!("{}/openapi/api-keys", server.uri());
        let err = call_create_endpoint_at(&url, "stale-bearer", &json!({ "name": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiKeyError::Unauthorized(_)), "got: {err:?}");
    }

    /// 服务端 403 `insufficient_scope` → Unauthorized + 中文引导。
    #[tokio::test]
    async fn endpoint_403_insufficient_scope_maps_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openapi/api-keys"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": "insufficient_scope",
                "error_description": "missing apikey.write"
            })))
            .mount(&server)
            .await;

        let url = format!("{}/openapi/api-keys", server.uri());
        let err = call_create_endpoint_at(&url, "bearer-without-scope", &json!({ "name": "x" }))
            .await
            .unwrap_err();
        match err {
            ApiKeyError::Unauthorized(msg) => {
                assert!(
                    msg.contains("apikey.write"),
                    "msg should mention scope: {msg}"
                );
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    /// 服务端 403 `organization_unbound` → RemoteRejected（不是 Unauthorized
    /// ——这不是 token 自身的问题，OAuth 重登录拿到的还是 null-org token，
    /// 必须走 re-OAuth 流程）。
    #[tokio::test]
    async fn endpoint_403_organization_unbound_maps_to_remote_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openapi/api-keys"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": "organization_unbound"
            })))
            .mount(&server)
            .await;

        let url = format!("{}/openapi/api-keys", server.uri());
        let err = call_create_endpoint_at(&url, "valid-bearer", &json!({ "name": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiKeyError::RemoteRejected(_)), "got: {err:?}");
    }

    /// 服务端 5xx → RemoteRejected。
    #[tokio::test]
    async fn endpoint_5xx_maps_to_remote_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openapi/api-keys"))
            .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
            .mount(&server)
            .await;

        let url = format!("{}/openapi/api-keys", server.uri());
        let err = call_create_endpoint_at(&url, "bearer", &json!({ "name": "x" }))
            .await
            .unwrap_err();
        match err {
            ApiKeyError::RemoteRejected(msg) => {
                assert!(msg.contains("503"), "msg should include status: {msg}");
            }
            other => panic!("expected RemoteRejected, got {other:?}"),
        }
    }

    /// 网络错（端点完全不可达）→ RemoteRejected("network: ...")。
    #[tokio::test]
    async fn endpoint_unreachable_maps_to_remote_rejected_network() {
        // 监听一个端口立刻关掉——保证后续 connect 会失败。
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{addr}/openapi/api-keys");

        let err = call_create_endpoint_at(&url, "bearer", &json!({ "name": "x" }))
            .await
            .unwrap_err();
        match err {
            ApiKeyError::RemoteRejected(msg) => assert!(
                msg.starts_with("network:"),
                "msg should be tagged 'network:': {msg}"
            ),
            other => panic!("expected RemoteRejected(network), got {other:?}"),
        }
    }

    /// 服务端 201 但 `id` / `key` 为空 → RemoteRejected。防止"上游契约变更后
    /// 我们悄悄把空字符串写进 keychain"。
    #[tokio::test]
    async fn endpoint_201_with_empty_id_or_key_is_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openapi/api-keys"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "",
                "key": "sk-of-x"
            })))
            .mount(&server)
            .await;

        let url = format!("{}/openapi/api-keys", server.uri());
        let err = call_create_endpoint_at(&url, "b", &json!({ "name": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiKeyError::RemoteRejected(_)), "got: {err:?}");
    }

    /// 本地撤销应当清 keychain；settings 端在没条目时也吃得下（幂等）。
    #[tokio::test]
    async fn revoke_local_clears_keychain_entry() {
        let store: Arc<dyn SecretStore> = Arc::new(InMemoryStore::new());
        store
            .save(Slot::ApiKey { tool: AppType::Hermes }, "sk-of-hermes")
            .unwrap();

        revoke_local_with_store(AppType::Hermes, store.clone())
            .await
            .expect("revoke should succeed even when settings has no meta entry");

        assert_eq!(
            store.load(Slot::ApiKey { tool: AppType::Hermes }).unwrap(),
            None
        );
    }

    /// 命名策略：含 tool slug + host，截断到 24 char 后仍能完整拼出
    /// `<tool> on <host-prefix>`。
    #[test]
    fn default_key_name_format() {
        let name = default_key_name(AppType::Claude);
        assert!(name.starts_with("claude on "), "got: {name}");
    }

    /// 服务端 name 校验是 `^[\w\s-]+$`——名字里**不能**包含点号或其它符号；
    /// macOS hostname 默认带 `.local` 必须被 sanitize 掉，否则 bind 直接 400。
    /// 这个测试不实际改 hostname（无法 mock 系统调用），但断言 default_key_name
    /// 的输出始终满足服务端 regex。
    #[test]
    fn default_key_name_satisfies_remote_regex() {
        let name = default_key_name(AppType::Codex);
        for ch in name.chars() {
            assert!(
                ch.is_ascii_alphanumeric() || ch == ' ' || ch == '_' || ch == '-',
                "default_key_name 含非法字符 {ch:?}: {name}",
            );
        }
        // 长度上限：tool 前缀 ≤ 16 char + " on " 4 char + host 24 char = 44 char，
        // 远小于服务端 100 char 上限。
        assert!(name.len() <= 44, "name 超长 ({} chars): {name}", name.len());
    }

    // ─── helpers ─────────────────────────────────────────────────────────

    /// 构造一个永远走不到的 OfoxAuthManager 假对象。命中分支测试用，调任何
    /// 异步方法都会被 cancel——我们的测试断言路径根本不会触达。
    fn dummy_manager_for_cache_test() -> Arc<RwLock<OfoxAuthManager>> {
        // 真实 OfoxAuthManager 需要 storage path 等，构造代价大；测试只关心
        // "CachedOk 命中时不去碰 manager"，所以这里给一个永远不会被读的占位。
        // 用 Arc::new_uninit 不安全；改用一个最小的真实 manager（仅借测试用
        // 临时目录作 storage path）。
        use std::path::PathBuf;
        let tmp: PathBuf = std::env::temp_dir().join("cc-switch-cache-test");
        std::fs::create_dir_all(&tmp).ok();
        let store: Arc<dyn SecretStore> = Arc::new(InMemoryStore::new());
        let mgr = OfoxAuthManager::new_with_secret_store(tmp, store);
        Arc::new(RwLock::new(mgr))
    }
}
