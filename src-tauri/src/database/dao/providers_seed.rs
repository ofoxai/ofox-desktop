//! 官方供应商种子数据
//!
//! 启动时调用 `Database::init_default_official_providers` 把这些条目
//! 写入 `providers` 表，让所有用户都能看到一个"一键切回官方"的入口。
//!
//! 字段与前端预设保持一致，参见：
//! - `src/config/claudeProviderPresets.ts`（"Claude Official"）
//! - `src/config/codexProviderPresets.ts`（"OpenAI Official"）
//! - `src/config/geminiProviderPresets.ts`（"Google Official"）

use std::sync::LazyLock;

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
pub(crate) fn is_ofox_seed_id(id: &str) -> bool {
    OFOX_SEEDS.iter().any(|seed| seed.id == id)
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
/// `base_url` 要在运行时按 `ofox_endpoints::OFOX_GATEWAY_BASE_URL` 拼出来，
/// dev/prod 切换时只改那一个常量即可，不必再各 seed 手动同步。
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

/// 6 个应用各一条 OfoxAI 种子。
///
/// settings_config 与前端各 `*ProviderPresets.ts` 中的 OfoxAI 条目保持一致，
/// 但其中所有 `base_url` 字段都从 [`ofox_endpoints`] 的运行时拼接函数取值——
/// dev 自测时网关跑在 `localhost:8088`，线上是 `https://api.ofox.ai`，一处开关。
pub(crate) static OFOX_SEEDS: LazyLock<Vec<OfoxProviderSeed>> = LazyLock::new(|| {
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
            settings_config_json: format!(
                r#"{{"env":{{"GOOGLE_GEMINI_BASE_URL":"{gemini}"}}}}"#
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
});
