/**
 * OFox URL composers, sourced from the current apex (`ofox.ai` / `ofox.io`).
 *
 * Why a dedicated module: every front-end surface that links into OFox—login
 * page, console, tray popover, provider-presets list—needs the same URL but
 * with a different apex depending on the user's region. Hardcoding the apex
 * in each component is what got us into the "switch region but the recharge
 * button still goes to ofox.ai" mess. These getters make apex follow the
 * single source of truth (`useOfoxApex` hook → `settingsApi.get().ofoxApex`).
 *
 * Backend mirror: `src-tauri/src/ofox_apex.rs` — keep the path constants in
 * sync. Both sides resolve to identical URLs in release; dev mode is handled
 * only on the Rust side (the front end is just opening external browsers,
 * which can't talk to localhost OAuth IDPs anyway).
 */

export type OfoxApex = "ofox.ai" | "ofox.io";

/** 默认 apex——hook 还没读到 settings 时使用。与 ofox-studio 的 DEFAULT_APEX 对齐。 */
export const DEFAULT_OFOX_APEX: OfoxApex = "ofox.ai";

/** `https://app.<apex>/manage/wallet` —— 充值入口（仅 owner / admin 可见）。 */
export const ofoxWalletUrl = (a: OfoxApex): string =>
  `https://app.${a}/manage/wallet`;

/** `https://app.<apex>/manage/api-keys` —— provider preset 默认 apiKeyUrl。 */
export const ofoxApiKeysUrl = (a: OfoxApex): string =>
  `https://app.${a}/manage/api-keys`;

/**
 * `https://app.<apex>/auth/sign-up?utm_source=desktop&utm_medium=app` ——
 * 登录页"免费注册"。
 *
 * 带 UTM 参数标记来源是 desktop 客户端：主站的 GTM 埋点只读 dataLayer、不读
 * 自定义 URL query，所以走 GA4 **原生**识别的 `utm_*`——无需改主站，注册流量
 * 会归入 GA4「流量获取 / 来源·媒介」报告的 `desktop / app`。
 */
export const ofoxRegisterUrl = (a: OfoxApex): string =>
  `https://app.${a}/auth/sign-up?utm_source=desktop&utm_medium=app`;

/** `https://app.<apex>/dashboard` —— 控制台跳转入口。 */
export const ofoxDashboardUrl = (a: OfoxApex): string =>
  `https://app.${a}/dashboard`;

/**
 * `https://app.<apex>/analytics/activity` —— 详细用量明细页。
 *
 * Tray popover 与主窗口"用量详情"按钮使用：dashboard 是用户活动总览，
 * activity 才是用量条目级明细，更贴合"用量详情"语义。
 */
export const ofoxActivityUrl = (a: OfoxApex): string =>
  `https://app.${a}/analytics/activity`;

/**
 * `https://app.<apex>/analytics?apiKeys=<keyId>` —— 单把 API key 维度的数据
 * 统计页。绑定工具行的"数据统计"按钮使用：`keyId` 是该工具绑定时 ofox 签发的
 * key id（见 `ApiKeyMeta.key_id`，经 `ofox_list_api_keys` 暴露到前端）。
 */
export const ofoxAnalyticsUrl = (a: OfoxApex, apiKeyId: string): string =>
  `https://app.${a}/analytics?apiKeys=${encodeURIComponent(apiKeyId)}`;

/** `https://<apex>/zh/terms-of-service` —— 登录页底部条款。 */
export const ofoxTermsUrl = (a: OfoxApex): string =>
  `https://${a}/zh/terms-of-service`;

/** `https://<apex>/privacy` —— 登录页底部隐私政策。 */
export const ofoxPrivacyUrl = (a: OfoxApex): string => `https://${a}/privacy`;

/** `https://<apex>` —— 营销主站（控制台底部"了解 OFox"等链接）。 */
export const ofoxMarketingUrl = (a: OfoxApex): string => `https://${a}`;

/** `https://api.<apex>` —— LLM 网关 / OpenAPI base，preset 表单 base_url 默认。 */
export const ofoxApiBase = (a: OfoxApex): string => `https://api.${a}`;

/**
 * OFox 用户头像的对象存储 CDN 前缀。`/openapi/me` 返回的 `avatar_url`
 * 是这个 bucket 内的相对 key（形如 `GdSPgRS6sTf3KgsOWrhSojnMoJIAC5Eq/xxx.webp`），
 * 需要前端拼上 bucket 域名 + `avatars/` 路径前缀才能加载。
 *
 * 该 bucket 是阿里云 OSS 全球加速域名（oss-accelerate），不随 apex 切换——
 * ofox.ai / ofox.io 共用同一份用户上传资源。
 */
const OFOX_AVATAR_CDN_BASE =
  "https://ofox-uploads.oss-accelerate.aliyuncs.com/avatars/";

/**
 * 解析 `/openapi/me.avatar_url` 为可加载的绝对 URL。
 *
 * 后端约定：返回的 `avatar_url` 始终是 bucket 内的相对 key，前端负责拼前缀。
 * 兼容输入：
 *   - null / undefined / 空串 → null（调用方走首字母 fallback）
 *   - 已经是 `http(s)://` 的完整 URL → 原样返回（兼容未来后端切换到完整 URL）
 *   - 相对 key（前后可能带 `/`）→ 拼接 [`OFOX_AVATAR_CDN_BASE`]
 */
export function ofoxAvatarUrl(raw: string | null | undefined): string | null {
  if (typeof raw !== "string") return null;
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  // 去掉可能存在的前导 `/` 避免拼出 `avatars//xxx`
  const key = trimmed.replace(/^\/+/, "");
  return `${OFOX_AVATAR_CDN_BASE}${key}`;
}
