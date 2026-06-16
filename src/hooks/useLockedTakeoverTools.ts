import { useMemo } from "react";
import {
  BOUND_TOOLS_STORAGE_KEY,
  PROXY_SUPPORTED_TOOLS,
} from "@/config/toolMeta";

/**
 * Tools whose proxy takeover is locked ON while cc-switch is running.
 *
 * Equal to `boundTools ∩ PROXY_SUPPORTED_TOOLS`: a tool is locked only when
 * the user has bound it AND we actually support intercepting it. OpenCode and
 * friends fall outside `PROXY_SUPPORTED_TOOLS`, so even if bound they stay
 * user-controllable (the row already says "暂不支持代理统计").
 *
 * The bound list lives in localStorage under `BOUND_TOOLS_STORAGE_KEY` and is
 * effectively a session-level constant — it changes only during onboarding,
 * which remounts the entire app tree (Onboarding → MainApp). For that reason
 * we read once via `useMemo` rather than subscribing to a `storage` event:
 * the in-window `storage` event doesn't fire for same-tab writes anyway, and
 * any path that mutates the bound list also forces a remount.
 *
 * Consumers (ProxyToggle, ProxyPanel) live deep in App.tsx's tree where
 * threading `boundTools` as a prop would be invasive; this hook gives them a
 * stable, prop-free read.
 */
export function useLockedTakeoverTools(): Set<string> {
  return useMemo(() => {
    try {
      const raw = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
      const bound: string[] = raw ? JSON.parse(raw) : [];
      return new Set(bound.filter((t) => PROXY_SUPPORTED_TOOLS.includes(t)));
    } catch {
      return new Set();
    }
  }, []);
}
