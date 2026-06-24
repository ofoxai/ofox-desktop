import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import OnboardingApp from "./OnboardingApp";
import ConsolePage from "@/components/console/ConsolePage";
import {
  OFOX_AUTH_EXPIRED_EVENT,
  OFOX_AUTH_RESTORED_EVENT,
  ofoxGetAuthStatus,
} from "@/lib/api/ofoxAuth";
import { BOUND_TOOLS_STORAGE_KEY, PROXY_SUPPORTED_TOOLS } from "@/config/toolMeta";
import { proxyApi } from "@/lib/api/proxy";

type AppState = "loading" | "onboarding" | "console";

/**
 * Read the persisted bound-tool list synchronously. Used both at top-level
 * routing time (deciding onboarding vs console for an already-logged-in user)
 * and by the locked-takeover reconciliation effect.
 */
function readBoundToolsFromStorage(): string[] {
  try {
    const stored = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
    if (!stored) return [];
    const parsed = JSON.parse(stored);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

/**
 * Top-level router for the main window.
 *
 * Drives three states off the backend's auth snapshot:
 *   - `active`    → Console
 *   - `expired`   → Onboarding (LoginPage), so the user can re-authenticate
 *                   without having to dig through a settings menu
 *   - `loggedout` → Onboarding (LoginPage)
 *
 * It listens to three events so the swap happens without manual refresh:
 *   - `ofox-auth-expired`     → drop to onboarding immediately
 *   - `ofox-auth-restored`    → lift back to console (after re-login)
 *   - `ofox-reauth-requested` → tray popover asked us to surface the
 *                                LoginPage even if the current state hasn't
 *                                flipped yet (e.g. user explicitly clicked
 *                                "重新登录" from the popover)
 */
export default function MainApp() {
  const [appState, setAppState] = useState<AppState>("loading");
  const [boundTools, setBoundTools] = useState<string[]>([]);
  // When entering Onboarding from an already-logged-in state with empty
  // bind list, skip the LoginPage and drop straight into the tool picker.
  // `null` while we're loading or while the resolved entry is the default
  // ("login").
  const [onboardingEntry, setOnboardingEntry] = useState<
    "login" | "toolDiscovery"
  >("login");

  const loadBoundTools = () => {
    setBoundTools(readBoundToolsFromStorage());
  };

  // Initial probe.
  useEffect(() => {
    ofoxGetAuthStatus()
      .then((status) => {
        if (status.state === "active") {
          const tools = readBoundToolsFromStorage();
          setBoundTools(tools);
          if (tools.length === 0) {
            // Logged in but no tools bound — happens after a fresh install,
            // wiped local state, or migration from a previous version. The
            // LoginPage would be a no-op (already logged in), so jump
            // straight to the tool picker.
            setOnboardingEntry("toolDiscovery");
            setAppState("onboarding");
          } else {
            setAppState("console");
          }
        } else {
          setOnboardingEntry("login");
          setAppState("onboarding");
        }
      })
      .catch(() => {
        setOnboardingEntry("login");
        setAppState("onboarding");
      });
  }, []);

  // Event-driven transitions.
  useEffect(() => {
    let active = true;
    const offs: Array<() => void> = [];

    const setup = async () => {
      const offExpired = await listen(OFOX_AUTH_EXPIRED_EVENT, () => {
        if (!active) return;
        // Token expired — user must log in again.
        setOnboardingEntry("login");
        setAppState("onboarding");
      });
      const offRestored = await listen(OFOX_AUTH_RESTORED_EVENT, () => {
        if (!active) return;
        // Login succeeded. If the user already has bound tools (returning
        // user), go straight to console; otherwise drop them into the tool
        // picker so onboarding completes end-to-end.
        const tools = readBoundToolsFromStorage();
        setBoundTools(tools);
        if (tools.length === 0) {
          setOnboardingEntry("toolDiscovery");
          setAppState("onboarding");
        } else {
          setAppState("console");
        }
      });
      // The tray popover's "重新登录" button asks us to surface onboarding
      // even when the state was already non-active (e.g. user dismissed and
      // came back).
      const offReauth = await listen("ofox-reauth-requested", () => {
        if (!active) return;
        setOnboardingEntry("login");
        setAppState("onboarding");
      });

      if (!active) {
        offExpired();
        offRestored();
        offReauth();
        return;
      }
      offs.push(offExpired, offRestored, offReauth);
    };

    void setup();
    return () => {
      active = false;
      offs.forEach((off) => off());
    };
  }, []);

  /**
   * 历史：takeover 时代的"自愈"——Console mount 时强行给所有
   * `boundTools ∩ PROXY_SUPPORTED_TOOLS` 开 proxy takeover，保证工具配置
   * 文件始终指向 `127.0.0.1:15721`。
   *
   * **bind 直写改造后这层自愈反而是 bug**：新 bind 路径已经把真 sk-of- 写
   * 进工具配置文件，再叠加一次 `setProxyTakeoverForApp(_, true)` 会把刚写
   * 好的真 token 覆盖回 `PROXY_MANAGED` 占位符。Claude/Codex 都受影响，磁
   * 盘静悄悄从"指 ofox gateway 真 key"变成"指本地 proxy 占位符"——后续 LLM
   * 请求会全部失败。
   *
   * Gemini 仍然走老 takeover 路径（它的 ofox seed 没 LLM token 字段；详见
   * `commands/ofox_auth.rs::ofox_provider_for`）。对 Gemini 保留自愈语义。
   */
  useEffect(() => {
    if (appState !== "console" || boundTools.length === 0) return;
    // 只对仍走老 takeover 路径的工具做自愈——目前只有 Gemini。
    // **保持与后端 `ofox_provider_for`、`bindTools.ts::OFOX_AUTO_BIND_TOOLS`
    // 互补一致**：那两处明确收录的工具（claude/codex/opencode/openclaw/hermes）
    // 走 ofox 直写，自愈必须避开；剩下还在 PROXY_SUPPORTED 里的就是 Gemini。
    const TAKEOVER_ONLY_TOOLS = new Set(["gemini"]);
    const locked = boundTools.filter(
      (t) => PROXY_SUPPORTED_TOOLS.includes(t) && TAKEOVER_ONLY_TOOLS.has(t),
    );
    if (locked.length === 0) return;

    let cancelled = false;
    (async () => {
      try {
        const status = await proxyApi.getProxyTakeoverStatus();
        if (cancelled) return;
        const map = status as unknown as Record<string, boolean>;
        for (const tool of locked) {
          if (cancelled) return;
          if (!map[tool]) {
            try {
              await proxyApi.setProxyTakeoverForApp(tool, true);
            } catch (e) {
              console.error(
                `[MainApp] force-enable takeover for ${tool} failed`,
                e,
              );
            }
          }
        }
      } catch (e) {
        console.error("[MainApp] takeover reconciliation failed", e);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [appState, boundTools]);

  if (appState === "loading") {
    return null;
  }

  if (appState === "onboarding") {
    return (
      <OnboardingApp
        initialStep={onboardingEntry}
        onComplete={() => {
          loadBoundTools();
          setAppState("console");
        }}
      />
    );
  }

  return (
    <ConsolePage
      boundTools={boundTools}
      onBoundToolsChanged={loadBoundTools}
    />
  );
}
