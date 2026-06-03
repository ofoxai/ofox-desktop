import { useState } from "react";
import LoginPage from "@/components/onboarding/LoginPage";
import ToolDiscoveryPage from "@/components/onboarding/ToolDiscoveryPage";
import SetupCompletePage from "@/components/onboarding/SetupCompletePage";
import { BOUND_TOOLS_STORAGE_KEY, PROXY_SUPPORTED_TOOLS } from "@/config/toolMeta";
import { proxyApi } from "@/lib/api/proxy";
import type { OfoxUserInfo } from "@/lib/api/ofoxAuth";

type Step = "login" | "toolDiscovery" | "setupComplete";

interface OnboardingAppProps {
  onComplete: () => void;
}

export default function OnboardingApp({ onComplete }: OnboardingAppProps) {
  const [step, setStep] = useState<Step>("login");
  const [boundCount, setBoundCount] = useState(0);

  const handleLoginSuccess = (_user: OfoxUserInfo) => {
    setStep("toolDiscovery");
  };

  const handleBind = async (selectedTools: string[]) => {
    localStorage.setItem(BOUND_TOOLS_STORAGE_KEY, JSON.stringify(selectedTools));

    // 为选中的、支持代理的工具开启本地代理
    for (const tool of selectedTools) {
      if (PROXY_SUPPORTED_TOOLS.includes(tool)) {
        try {
          await proxyApi.setProxyTakeoverForApp(tool, true);
        } catch (e) {
          console.error(`Failed to enable proxy for ${tool}:`, e);
        }
      }
    }

    setBoundCount(selectedTools.length);
    setStep("setupComplete");
  };

  switch (step) {
    case "login":
      return <LoginPage onLoginSuccess={handleLoginSuccess} />;
    case "toolDiscovery":
      return (
        <ToolDiscoveryPage
          onBack={() => setStep("login")}
          onBind={handleBind}
        />
      );
    case "setupComplete":
      return (
        <SetupCompletePage
          boundCount={boundCount}
          onOpenConsole={onComplete}
        />
      );
  }
}
