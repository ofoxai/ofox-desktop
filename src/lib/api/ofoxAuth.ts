import { invoke } from "@tauri-apps/api/core";

/**
 * Wallet snapshot from `/openapi/orgs/me/balance`. All amounts are floats
 * in `currency` (USD by default). Every field is nullable because the
 * upstream schema is still in flux — render "—" for missing values.
 */
export interface OfoxBalance {
  balance?: number | null;
  total?: number | null;
  used?: number | null;
  currency?: string | null;
}

/**
 * Distilled `/openapi/orgs/me/spending-limits` payload.
 *
 * - `forbidden: true` ⇒ caller is `member` role; UI should show "无权限".
 * - `monthlyUsedUsd === null` ⇒ org has no `period: "monthly"` limit
 *   configured; UI should show "—".
 */
export interface OfoxSpending {
  forbidden: boolean;
  monthly_used_usd?: number | null;
  monthly_limit_usd?: number | null;
}

export interface OfoxUserInfo {
  email: string | null;
  name: string | null;
  org_id: string | null;
  avatar_url: string | null;
  /** Latest wallet snapshot. Omitted when balance fetch failed at the source. */
  balance?: OfoxBalance | null;
  /** Best-effort monthly spend. Omitted when the endpoint errored. */
  spending?: OfoxSpending | null;
}

/**
 * Mirrors `OfoxAuthState` on the Rust side (rename_all = "lowercase").
 *
 * - `active`    — tokens are valid, all flows work.
 * - `expired`   — refresh_token rejected; UI must prompt re-login.
 * - `logged_out` — no tokens; first-run / post-logout state.
 */
export type OfoxAuthStateValue = "active" | "expired" | "loggedout";

export interface OfoxAuthStatus {
  state: OfoxAuthStateValue;
  user: OfoxUserInfo | null;
}

export interface OfoxDeviceCodeResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string | null;
  expires_in: number;
  interval: number;
}

/** Event name emitted by the backend when refresh_token is rejected. */
export const OFOX_AUTH_EXPIRED_EVENT = "ofox-auth-expired";
/** Event name emitted by the backend after a successful re-login. */
export const OFOX_AUTH_RESTORED_EVENT = "ofox-auth-restored";

export async function ofoxStartLogin(): Promise<OfoxDeviceCodeResponse> {
  return invoke<OfoxDeviceCodeResponse>("ofox_start_login");
}

export async function ofoxPollForToken(
  deviceCode: string,
): Promise<OfoxUserInfo | null> {
  return invoke<OfoxUserInfo | null>("ofox_poll_for_token", { deviceCode });
}

export async function ofoxGetUserInfo(): Promise<OfoxUserInfo | null> {
  return invoke<OfoxUserInfo | null>("ofox_get_user_info");
}

export async function ofoxGetAuthStatus(): Promise<OfoxAuthStatus> {
  return invoke<OfoxAuthStatus>("ofox_get_auth_status");
}

export async function ofoxIsAuthenticated(): Promise<boolean> {
  return invoke<boolean>("ofox_is_authenticated");
}

export async function ofoxLogout(): Promise<void> {
  return invoke("ofox_logout");
}
