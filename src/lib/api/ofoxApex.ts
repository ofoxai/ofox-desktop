import { invoke } from "@tauri-apps/api/core";

import type { OfoxApex } from "@/lib/ofoxUrls";

/**
 * Tauri command wrappers for OFox apex (region) state.
 *
 * The backend (`src-tauri/src/commands/ofox_apex.rs`) is the single writer:
 * it knows how to atomically perform "logout + reseed providers + persist
 * settings + notify UI" in one shot. The front end never mutates `ofoxApex`
 * via `settingsApi.save` directly — that would skip the side effects and
 * leave bound tools pointing at the wrong gateway.
 */

/** 读后端当前 apex。前端 hook 在挂载时调一次，并监听 `ofox-apex-changed` 事件。 */
export async function ofoxGetApex(): Promise<OfoxApex> {
  const value = await invoke<string>("ofox_get_apex");
  // 后端理论上只会返回白名单值，但保守一下：万一返回了未知字符串（升级回滚
  // 等极端情形），fallback 到 ofox.ai 比抛错更友好。
  if (value === "ofox.ai" || value === "ofox.io") return value;
  return "ofox.ai";
}

/**
 * 切换 apex。后端会：
 *   1. 校验 next 在白名单；
 *   2. 若与当前相同直接 return；
 *   3. 写 settings → logout → reseed all `ofox-*` provider rows；
 *   4. emit `ofox-apex-changed` + `ofox-reauth-requested`（MainApp 接 reauth 跳 LoginPage）。
 *
 * 调用方拿到 reject 后只需 toast——副作用要么全做、要么没做、要么半做但已 emit
 * reauth；UI 跟随事件即可。
 */
export async function ofoxSetApex(next: OfoxApex): Promise<void> {
  await invoke("ofox_set_apex", { nextApex: next });
}

/** 后端 emit 的 apex 切换事件名，供前端 hook 监听。 */
export const OFOX_APEX_CHANGED_EVENT = "ofox-apex-changed";
