import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import OnboardingApp from "./OnboardingApp";
import ConsolePage from "@/components/console/ConsolePage";
import {
  OFOX_AUTH_EXPIRED_EVENT,
  OFOX_AUTH_RESTORED_EVENT,
  ofoxGetAuthStatus,
} from "@/lib/api/ofoxAuth";
import { BOUND_TOOLS_STORAGE_KEY } from "@/config/toolMeta";

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

  // 历史："force-enable takeover" 自愈 effect 已在 bind 直写改造完成后整段
  // 移除——6 个支持工具全部走 `ofox_bind_tool`（直写 sk-of- 到工具配置文件、
  // baseURL 指 ofox gateway），没有任何工具再依赖 proxy takeover。再保留这层
  // 自愈反而会把磁盘上真 token 覆盖成 `PROXY_MANAGED` 占位符。
  //
  // proxy server 本身（`crate::proxy::server`）保留但不再被 ofox bind 启动，
  // 留给将来其它非 ofox provider 的延伸使用，超出本工程范围。

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
