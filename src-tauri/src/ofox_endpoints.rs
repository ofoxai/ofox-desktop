//! OfoxAI 网关端点单一开关。
//!
//! Ofox 后端是多服务编排的（auth IDP、OpenAPI、LLM gateway 是不同进程），
//! 本地开发时它们分别监听 3000/8080/8088，线上由 Traefik 统一暴露在
//! `api.ofox.ai`/`app.ofox.ai`。这里只管 **LLM 网关** 的基础 URL：所有
//! ofox-* 种子的 base_url 与 ping 探测都从这里取，避免 dev/prod 分裂时
//! 漏改某个常量导致 token 在 dev IDP 签发但被打到 prod 网关 Redis 验签
//! 失败（"Invalid or expired token"）。
//!
//! Auth IDP 的端点（device authorization、token、user info）继续放在
//! `ofox_auth.rs` 里——那是另一个独立的开关面，混在一起反而难维护。
//!
//! 发布前把 `OFOX_GATEWAY_BASE_URL` 切回 `https://api.ofox.ai`。

/// LLM 网关的基础 URL。dev 指向本地 gateway plugin（:8088），prod 指向
/// `https://api.ofox.ai`。**没有结尾斜杠**——需要拼路径的调用方自行加 `/v1`、
/// `/anthropic`、`/gemini` 等子路径。
///
/// 与 `ofox_auth.rs` 里的 dev/prod 切换保持同步——目前那边也指向 localhost，
/// 发布时一起改。
// TODO: 发布前改回 "https://api.ofox.ai"
pub const OFOX_GATEWAY_BASE_URL: &str = "http://localhost:8088";

/// `<gateway>/v1` —— OpenAI 兼容协议（codex / opencode / openclaw / hermes）。
pub fn openai_v1_base_url() -> String {
    format!("{OFOX_GATEWAY_BASE_URL}/v1")
}

/// `<gateway>/anthropic` —— Claude Code 走的 Anthropic 协议前缀。
/// 用于 `ANTHROPIC_BASE_URL`，CLI 自己会拼上 `/v1/messages`。
pub fn anthropic_base_url() -> String {
    format!("{OFOX_GATEWAY_BASE_URL}/anthropic")
}

/// `<gateway>/gemini` —— Gemini CLI 走的 Gemini 协议前缀。
/// 用于 `GOOGLE_GEMINI_BASE_URL`，CLI 自己会拼上 `/v1beta/...`。
pub fn gemini_base_url() -> String {
    format!("{OFOX_GATEWAY_BASE_URL}/gemini")
}
