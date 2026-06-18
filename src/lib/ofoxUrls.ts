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

/** `https://app.<apex>/register` —— 登录页"免费注册"。 */
export const ofoxRegisterUrl = (a: OfoxApex): string =>
  `https://app.${a}/register`;

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

/** `https://<apex>/terms` —— 登录页底部条款。 */
export const ofoxTermsUrl = (a: OfoxApex): string => `https://${a}/terms`;

/** `https://<apex>/privacy` —— 登录页底部隐私政策。 */
export const ofoxPrivacyUrl = (a: OfoxApex): string => `https://${a}/privacy`;

/** `https://<apex>` —— 营销主站（控制台底部"了解 OFox"等链接）。 */
export const ofoxMarketingUrl = (a: OfoxApex): string => `https://${a}`;

/** `https://api.<apex>` —— LLM 网关 / OpenAPI base，preset 表单 base_url 默认。 */
export const ofoxApiBase = (a: OfoxApex): string => `https://api.${a}`;
