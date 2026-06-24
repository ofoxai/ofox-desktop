//! 官方供应商种子数据
//!
//! 启动时调用 `Database::init_default_official_providers` 把这些条目
//! 写入 `providers` 表，让所有用户都能看到一个"一键切回官方"的入口。
//!
//! 字段与前端预设保持一致，参见：
//! - `src/config/claudeProviderPresets.ts`（"Claude Official"）
//! - `src/config/codexProviderPresets.ts`（"OpenAI Official"）
//! - `src/config/geminiProviderPresets.ts`（"Google Official"）

use crate::app_config::AppType;
use crate::ofox_endpoints::{anthropic_base_url, gemini_base_url, openai_v1_base_url};

/// 单条官方供应商种子定义。
pub(crate) struct OfficialProviderSeed {
    pub id: &'static str,
    pub app_type: AppType,
    pub name: &'static str,
    pub website_url: &'static str,
    pub icon: &'static str,
    pub icon_color: &'static str,
    /// settings_config 的 JSON 字符串，每个 app 结构不同。
    pub settings_config_json: &'static str,
}

/// Claude / Codex / Gemini 三个应用的官方预设。
///
/// id 固定，便于幂等检查；name 直接用英文原名（与前端预设一致），不做 i18n。
pub(crate) const OFFICIAL_SEEDS: &[OfficialProviderSeed] = &[
    OfficialProviderSeed {
        id: "claude-official",
        app_type: AppType::Claude,
        name: "Claude Official",
        website_url: "https://www.anthropic.com/claude-code",
        icon: "anthropic",
        icon_color: "#D4915D",
        // 空 env 让用户走 Claude CLI 默认认证流程
        settings_config_json: r#"{"env":{}}"#,
    },
    OfficialProviderSeed {
        id: "codex-official",
        app_type: AppType::Codex,
        name: "OpenAI Official",
        website_url: "https://chatgpt.com/codex",
        icon: "openai",
        icon_color: "#00A67E",
        // 空 auth + 空 config 让用户走 ChatGPT Plus/Pro OAuth
        settings_config_json: r#"{"auth":{},"config":""}"#,
    },
    OfficialProviderSeed {
        id: "gemini-official",
        app_type: AppType::Gemini,
        name: "Google Official",
        website_url: "https://ai.google.dev/",
        icon: "gemini",
        icon_color: "#4285F4",
        // 空 env + 空 config 让用户走 Google OAuth
        settings_config_json: r#"{"env":{},"config":{}}"#,
    },
];

/// 判断给定的 provider id 是否属于内置官方种子。
///
/// 单一事实源：直接扫描 `OFFICIAL_SEEDS`，避免在多处重复维护 id 列表。
pub(crate) fn is_official_seed_id(id: &str) -> bool {
    OFFICIAL_SEEDS.iter().any(|seed| seed.id == id)
}

/// 判断给定的 provider id 是否属于 OfoxAI 种子。
///
/// 用 `OFOX_SEED_IDS` 静态白名单而不是 [`ofox_seeds`]：判 id 时不需要每次都
/// 重新构建整个 `Vec<OfoxProviderSeed>`（含 `format!` 进 settings_config 的
/// 所有 base_url），避免一个轻量判定路径被 apex 切换引起的重建拖慢。
pub(crate) fn is_ofox_seed_id(id: &str) -> bool {
    OFOX_SEED_IDS.contains(&id)
}

/// 判断给定的 provider id 是否属于任意内置种子（官方 + OfoxAI）。
///
/// 用于 `has_non_official_seed_provider`：排除所有内置种子后，判断是否存在用户手动创建的供应商。
pub(crate) fn is_builtin_seed_id(id: &str) -> bool {
    is_official_seed_id(id) || is_ofox_seed_id(id)
}

// ---------------------------------------------------------------------------
// OfoxAI 种子数据
// ---------------------------------------------------------------------------

/// OfoxAI 供应商种子定义。
///
/// 与官方种子结构相同，但 `meta_json` 携带 `providerType: "ofox"` 标记，
/// 前端据此启用自动模型获取、端点锁定等 OfoxAI 专属行为。
///
/// 字段是 `String`（不是 `&'static str`）——因为各 settings_config 里的
/// `base_url` 要在运行时按 [`crate::ofox_apex`] 当前 apex + dev/prod 拼出来，
/// dev/prod 切换或用户切区域时只改那一个开关即可，不必再各 seed 手动同步。
pub(crate) struct OfoxProviderSeed {
    pub id: &'static str,
    pub app_type: AppType,
    pub name: &'static str,
    pub website_url: &'static str,
    pub icon: &'static str,
    pub icon_color: &'static str,
    pub settings_config_json: String,
    pub meta_json: &'static str,
}

/// 6 个 OfoxAI 种子的 id 白名单。`is_ofox_seed_id` 判定时只需要扫这个轻量
/// 列表，不必触发 [`ofox_seeds`] 的字符串拼接。新增 ofox-* seed 时同步两边。
pub(crate) const OFOX_SEED_IDS: &[&str] = &[
    "ofox-claude",
    "ofox-codex",
    "ofox-gemini",
    "ofox-opencode",
    "ofox-openclaw",
    "ofox-hermes",
];

/// 切换 OFox apex 后，把所有 ofox-* provider 的 settings_config 用**当前 apex**
/// 的 seed 模板重新覆盖一遍。
///
/// **覆盖而不是 patch**：codex 那条 seed 把 `base_url` 嵌在 TOML 字符串里
/// （不是 JSON path），用 regex 改 base_url 既脆又难维护；直接拿当前 apex
/// 重新拼整段 settings_config，模板是唯一事实源，不会有"哪个字段我忘了改"
/// 的暗坑。
///
/// 调用约束：
/// - 必须在 [`crate::ofox_apex::current_apex`] 已经被切到**目标**值之后调用——
///   `ofox_seeds()` 内部会按当前值拼 base_url，顺序错了等于没切。
/// - 必须在 OFox logout（清 token）**之前或之后**都行：新 bind 直写架构下
///   ofox-* provider 的 settings_config 里 `*_KEY`/`*_TOKEN` 字段永远是
///   seed 模板的空字符串——本函数自身不读/写 token，token 注入只发生在
///   `commands/ofox_auth.rs::bind_tool_to_ofox_internal` 内部、写到工具
///   真实配置文件而非 DB。
///
/// 部分失败处理：单个 row 写失败 → log warn 继续下一个，整体返回成功条数。
/// 半残总比整段崩好。
pub fn reseed_ofox_providers_with_current_apex(
    db: &crate::database::Database,
) -> Result<usize, crate::error::AppError> {
    let mut updated = 0_usize;
    for seed in ofox_seeds() {
        let app_type_str = seed.app_type.as_str();

        // 仅处理已存在的 row：用户可能手动删除过某条 ofox-* seed（被
        // `init_default_ofox_providers` 的 flag 保护），那种情况下不应该
        // 重新插入——尊重用户的删除决定。
        let existing = match db.get_provider_by_id(seed.id, app_type_str) {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::debug!(
                    "[ofox_apex_reseed] {}/{} not in DB (deleted by user?), skip",
                    app_type_str,
                    seed.id
                );
                continue;
            }
            Err(e) => {
                log::warn!(
                    "[ofox_apex_reseed] read {}/{} failed: {e}; skip",
                    app_type_str,
                    seed.id
                );
                continue;
            }
        };

        let settings_config: serde_json::Value =
            match serde_json::from_str(&seed.settings_config_json) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!(
                        "[ofox_apex_reseed] seed JSON for {} invalid: {e}; skip",
                        seed.id
                    );
                    continue;
                }
            };

        // 把旧值留一条 debug 日志便于事后追查（包括误切回的回滚）。
        log::debug!(
            "[ofox_apex_reseed] {}/{} OLD settings_config: {}",
            app_type_str,
            seed.id,
            existing.settings_config
        );

        if let Err(e) = db.update_provider_settings_config(app_type_str, seed.id, &settings_config)
        {
            log::warn!(
                "[ofox_apex_reseed] write {}/{} failed: {e}; skip",
                app_type_str,
                seed.id
            );
            continue;
        }

        updated += 1;
        log::info!(
            "[ofox_apex_reseed] reseeded {}/{} with current apex",
            app_type_str,
            seed.id
        );
    }
    Ok(updated)
}

/// 6 个应用各一条 OfoxAI 种子。
///
/// settings_config 与前端各 `*ProviderPresets.ts` 中的 OfoxAI 条目保持一致，
/// 但其中所有 `base_url` 字段都从 [`ofox_endpoints`] 的运行时拼接函数取值——
/// dev 自测时网关跑在 `localhost:8088`，线上是 `https://api.<apex>`，apex
/// 切换由 [`crate::ofox_apex::current_apex`] 在每次调用时重新决定。
///
/// **每次调用都会重新构建 Vec**——这是有意的：apex 在运行时可被用户切换，缓存
/// 一份 LazyLock 反而会与切换后的状态不一致。轻量判定路径请用
/// [`OFOX_SEED_IDS`] 而不是这里。
pub(crate) fn ofox_seeds() -> Vec<OfoxProviderSeed> {
    let openai_v1 = openai_v1_base_url(); // <gateway>/v1
    let anthropic = anthropic_base_url(); // <gateway>/anthropic
    let gemini = gemini_base_url(); // <gateway>/gemini

    vec![
        // Claude Code — Anthropic 协议
        OfoxProviderSeed {
            id: "ofox-claude",
            app_type: AppType::Claude,
            name: "OfoxAI",
            website_url: "https://ofox.ai",
            icon: "ofox",
            icon_color: "#D97706",
            settings_config_json: format!(
                r#"{{"env":{{"ANTHROPIC_BASE_URL":"{anthropic}","ANTHROPIC_AUTH_TOKEN":""}}}}"#
            ),
            meta_json: r#"{"providerType":"ofox","apiKeyField":"ANTHROPIC_AUTH_TOKEN"}"#,
        },
        // Codex — OpenAI 兼容协议（TOML 与前端 generateThirdPartyConfig("ofox", ..., "") 保持一致）
        //
        // model 默认填 `bailian/qwen3-coder-plus`：codex CLI v0.140 在
        // `[model_providers.<name>]` 模式下要求 `model` 非空，否则启动报
        // "Thread model is unavailable, wait for the thread to finish syncing
        // or choose a model"。其他 ofox app 的 seed 可以留空（CLI 无此约束），
        // 但 codex 必须有一个 ofox 网关认得的具体名。用户可在"管理工具"对话框
        // 改成别的（写回这里 + 同步到 ~/.codex/config.toml）。
        OfoxProviderSeed {
            id: "ofox-codex",
            app_type: AppType::Codex,
            name: "OfoxAI",
            website_url: "https://ofox.ai",
            icon: "ofox",
            icon_color: "#D97706",
            settings_config_json: format!(
                r#"{{"auth":{{"OPENAI_API_KEY":""}},"config":"model_provider = \"ofox\"\nmodel = \"bailian/qwen3-coder-plus\"\nmodel_reasoning_effort = \"high\"\ndisable_response_storage = true\n\n[model_providers.ofox]\nname = \"ofox\"\nbase_url = \"{openai_v1}\"\nwire_api = \"responses\"\nrequires_openai_auth = true"}}"#
            ),
            meta_json: r#"{"providerType":"ofox"}"#,
        },
        // Gemini CLI — Gemini 协议
        OfoxProviderSeed {
            id: "ofox-gemini",
            app_type: AppType::Gemini,
            name: "OfoxAI",
            website_url: "https://ofox.ai",
            icon: "ofox",
            icon_color: "#D97706",
            // GEMINI_API_KEY 字段在 seed 里留空（占位）——bind 时由
            // `ProxyService::ofox_write_direct_to_live` 把 sk-of- 注入到
            // 工具真实配置文件（~/.gemini/.env），DB 行始终是 seed 模板态。
            // Gemini CLI 同时支持 Google OAuth 与 GEMINI_API_KEY 两种走法，
            // 这里走的是 GEMINI_API_KEY + base URL 重定向到 ofox gateway。
            settings_config_json: format!(
                r#"{{"env":{{"GOOGLE_GEMINI_BASE_URL":"{gemini}","GEMINI_API_KEY":""}}}}"#
            ),
            meta_json: r#"{"providerType":"ofox"}"#,
        },
        // OpenCode — OpenAI Compatible NPM 包
        OfoxProviderSeed {
            id: "ofox-opencode",
            app_type: AppType::OpenCode,
            name: "OfoxAI",
            website_url: "https://ofox.ai",
            icon: "ofox",
            icon_color: "#D97706",
            settings_config_json: format!(
                r#"{{"npm":"@ai-sdk/openai-compatible","name":"OfoxAI","options":{{"baseURL":"{openai_v1}","apiKey":""}},"models":{{}}}}"#
            ),
            meta_json: r#"{"providerType":"ofox"}"#,
        },
        // OpenClaw — OpenAI Completions 协议
        OfoxProviderSeed {
            id: "ofox-openclaw",
            app_type: AppType::OpenClaw,
            name: "OfoxAI",
            website_url: "https://ofox.ai",
            icon: "ofox",
            icon_color: "#D97706",
            settings_config_json: format!(
                r#"{{"baseUrl":"{openai_v1}","apiKey":"","api":"openai-completions","models":[]}}"#
            ),
            meta_json: r#"{"providerType":"ofox"}"#,
        },
        // Hermes — Chat Completions API Mode
        OfoxProviderSeed {
            id: "ofox-hermes",
            app_type: AppType::Hermes,
            name: "OfoxAI",
            website_url: "https://ofox.ai",
            icon: "ofox",
            icon_color: "#D97706",
            settings_config_json: format!(
                r#"{{"name":"ofox","base_url":"{openai_v1}","api_key":"","api_mode":"chat_completions","models":[]}}"#
            ),
            meta_json: r#"{"providerType":"ofox"}"#,
        },
    ]
}
