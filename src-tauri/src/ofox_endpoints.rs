//! OfoxAI LLM 网关协议入口（OpenAI / Anthropic / Gemini 兼容前缀）。
//!
//! 网关 base 由 [`crate::ofox_apex::gateway_base`] 单一开关：dev 指向
//! `http://localhost:8088`（ofox-gateway plugin），release 指向当前 apex 的
//! `https://api.<apex>`。本模块只负责拼出三个具体协议前缀，不再持有任何
//! 域名常量——这样 dev/prod 与 ofox.ai/ofox.io 切换都收敛到 `ofox_apex`，避免
//! 旧版"两份 const 漏改一处"那种 token-签发地与网关验签地不一致的问题。
//!
//! Auth IDP 的端点（device authorization、token、user info、balance、
//! spending-limits）在 `ofox_apex` 里有专门的组合器，不过这边——它们与 LLM
//! 网关是不同的服务，混在一起反而难维护。

/// `<gateway>/v1` —— OpenAI 兼容协议（codex / opencode / openclaw / hermes）。
pub fn openai_v1_base_url() -> String {
    format!("{}/v1", crate::ofox_apex::gateway_base())
}

/// `<gateway>/anthropic` —— Claude Code 走的 Anthropic 协议前缀。
/// 用于 `ANTHROPIC_BASE_URL`，CLI 自己会拼上 `/v1/messages`。
pub fn anthropic_base_url() -> String {
    format!("{}/anthropic", crate::ofox_apex::gateway_base())
}

/// `<gateway>/gemini` —— Gemini CLI 走的 Gemini 协议前缀。
/// 用于 `GOOGLE_GEMINI_BASE_URL`，CLI 自己会拼上 `/v1beta/...`。
pub fn gemini_base_url() -> String {
    format!("{}/gemini", crate::ofox_apex::gateway_base())
}
