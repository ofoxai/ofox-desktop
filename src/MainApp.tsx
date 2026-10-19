import { useEffect, useRef, useState } from "react";
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
function readBoundToolsFromStorage(): string[] {
  try {
    const parsed: unknown = JSON.parse(
      localStorage.getItem(BOUND_TOOLS_STORAGE_KEY) ?? "[]",
    );
    return Array.isArray(parsed)
      ? parsed.filter((r): r is string => typeof r === "string")
      : [];
  } catch {
    return [];
  }
}

/**
 * Local management is a route choice, never logout or an empty bind operation.
 * It is only the way through while signed out or after skipping binding: a
 * sign-in always leads to binding, as helping a newcomer get set up comes first.
 */
export default function MainApp() {
  const [appState, setAppState] = useState<AppState>("loading");
  const [boundTools, setBoundTools] = useState<string[]>([]);
  const [localOnly, setLocalOnly] = useState(false);
  const [consoleVisited, setConsoleVisited] = useState(false);
  const [signedIn, setSignedIn] = useState(false);
  const [onboardingEntry, setOnboardingEntry] = useState<
    "login" | "toolDiscovery"
  >("login");
  const [onboardingRevision, setOnboardingRevision] = useState(0);
  const routeVersion = useRef(0);
  const preferLocal = useRef(false);
  const currentScreen = useRef(appState);
  currentScreen.current = appState;
  const loadBoundTools = () => setBoundTools(readBoundToolsFromStorage());
  const showConsole = () => {
    currentScreen.current = "console";
    setConsoleVisited(true);
    setAppState("console");
  };
  const enterLocal = () => {
    routeVersion.current += 1;
    preferLocal.current = true;
    loadBoundTools();
    setLocalOnly(true);
    showConsole();
  };
  const openBinding = () => {
    setOnboardingEntry("toolDiscovery");
    setOnboardingRevision((r) => r + 1);
    setAppState("onboarding");
  };
  // Back to the bound tools, or into the one-click picker when none are bound.
  const afterSignIn = () => {
    routeVersion.current += 1;
    preferLocal.current = false;
    setSignedIn(true);
    const tools = readBoundToolsFromStorage();
    setBoundTools(tools);
    setLocalOnly(false);
    if (tools.length) showConsole();
    else openBinding();
  };
  useEffect(() => {
    const version = ++routeVersion.current;
    let mounted = true;
    void ofoxGetAuthStatus()
      .then((status) => {
        if (!mounted || version !== routeVersion.current) return;
        const active = status.state === "active";
        setSignedIn(active);
        const tools = readBoundToolsFromStorage();
        setBoundTools(tools);
        if (active && tools.length) showConsole();
        else {
          setOnboardingEntry(active ? "toolDiscovery" : "login");
          setAppState("onboarding");
        }
      })
      .catch(() => {
        if (mounted && version === routeVersion.current)
          setAppState("onboarding");
      });
    return () => {
      mounted = false;
      routeVersion.current += 1;
    };
  }, []);
  useEffect(() => {
    let active = true;
    const offs: (() => void)[] = [];
    const subscribe = async (name: string, callback: () => void) => {
      const off = await listen(name, () => {
        if (active) callback();
      });
      if (active) offs.push(off);
      else off();
    };
    void Promise.all([
      subscribe(OFOX_AUTH_EXPIRED_EVENT, () => {
        routeVersion.current += 1;
        setSignedIn(false);
        if (currentScreen.current === "console") enterLocal();
        else {
          setOnboardingEntry("login");
          setOnboardingRevision((r) => r + 1);
          setAppState("onboarding");
        }
      }),
      subscribe(OFOX_AUTH_RESTORED_EVENT, afterSignIn),
      subscribe("ofox-reauth-requested", () => {
        routeVersion.current += 1;
        setSignedIn(false);
        setLocalOnly(true);
        setOnboardingEntry("login");
        setOnboardingRevision((r) => r + 1);
        setAppState("onboarding");
      }),
    ]).catch(() => undefined);
    return () => {
      active = false;
      offs.forEach((off) => off());
    };
  }, []);
  if (appState === "loading") return null;
  const onboarding =
    appState === "onboarding" ? (
      <OnboardingApp
        key={onboardingRevision}
        initialStep={onboardingEntry}
        onEnterLocal={enterLocal}
        onComplete={() => {
          preferLocal.current = false;
          setSignedIn(true);
          loadBoundTools();
          setLocalOnly(false);
          showConsole();
        }}
      />
    ) : null;
  return (
    <>
      {onboarding}
      {consoleVisited && (
        <div hidden={appState !== "console"}>
          <ConsolePage
            active={appState === "console"}
            boundTools={boundTools}
            onBoundToolsChanged={loadBoundTools}
            localOnly={localOnly}
            localSignedIn={signedIn}
            onLoginRequested={() => {
              routeVersion.current += 1;
              if (signedIn) {
                preferLocal.current = false;
                if (readBoundToolsFromStorage().length) {
                  setLocalOnly(false);
                  showConsole();
                  return;
                }
                openBinding();
                return;
              }
              setOnboardingEntry("login");
              setOnboardingRevision((r) => r + 1);
              setAppState("onboarding");
            }}
          />
        </div>
      )}
    </>
  );
}
