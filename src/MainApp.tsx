import { useEffect, useState } from "react";
import OnboardingApp from "./OnboardingApp";
import ConsolePage from "@/components/console/ConsolePage";
import { ofoxIsAuthenticated } from "@/lib/api/ofoxAuth";
import { BOUND_TOOLS_STORAGE_KEY } from "@/config/toolMeta";

type AppState = "loading" | "onboarding" | "console";

export default function MainApp() {
  const [appState, setAppState] = useState<AppState>("loading");
  const [boundTools, setBoundTools] = useState<string[]>([]);

  useEffect(() => {
    ofoxIsAuthenticated()
      .then((authenticated) => {
        if (authenticated) {
          // 从 localStorage 读取已绑定的工具
          try {
            const stored = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
            if (stored) {
              setBoundTools(JSON.parse(stored));
            }
          } catch {
            // ignore parse error
          }
          setAppState("console");
        } else {
          setAppState("onboarding");
        }
      })
      .catch(() => {
        setAppState("onboarding");
      });
  }, []);

  if (appState === "loading") {
    return null;
  }

  if (appState === "onboarding") {
    return (
      <OnboardingApp
        onComplete={() => {
          try {
            const stored = localStorage.getItem(BOUND_TOOLS_STORAGE_KEY);
            if (stored) {
              setBoundTools(JSON.parse(stored));
            }
          } catch {
            // ignore
          }
          setAppState("console");
        }}
      />
    );
  }

  return <ConsolePage boundTools={boundTools} />;
}
