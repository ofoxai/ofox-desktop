import { useState } from "react";
import LoginPage from "@/components/onboarding/LoginPage";
import ToolDiscoveryPage from "@/components/onboarding/ToolDiscoveryPage";
import SetupCompletePage from "@/components/onboarding/SetupCompletePage";
import { bindTools } from "@/lib/bindTools";
import type { OfoxUserInfo } from "@/lib/api/ofoxAuth";

type Step = "login" | "toolDiscovery" | "setupComplete";

interface OnboardingAppProps {
  /**
   * Where to enter the wizard. Defaults to `"login"` (first-time / logged-out
   * users). MainApp passes `"toolDiscovery"` when the user already has a valid
   * OAuth session but has not bound any tools yet — typically because they
   * cleared local state, reinstalled, or ran the app on a fresh machine. In
   * that case the LoginPage would be a confusing dead-end (they're already
   * logged in), so we drop them directly into the tool picker.
   */
  initialStep?: Step;
  onComplete: () => void;
}

export default function OnboardingApp({
  initialStep = "login",
  onComplete,
}: OnboardingAppProps) {
  const [step, setStep] = useState<Step>(initialStep);
  const [boundCount, setBoundCount] = useState(0);

  const handleLoginSuccess = (_user: OfoxUserInfo) => {
    setStep("toolDiscovery");
  };

  const handleBind = async (selectedTools: string[]) => {
    // bindTools 返回真实成功列表（失败的工具已 toast 警告并被剔除）。
    // 用 succeeded.length 做完成页计数才不会在部分失败时谎报"绑了 N 个"。
    const succeeded = await bindTools(selectedTools);
    setBoundCount(succeeded.length);
    setStep("setupComplete");
  };

  switch (step) {
    case "login":
      return <LoginPage onLoginSuccess={handleLoginSuccess} />;
    case "toolDiscovery":
      // If we entered the wizard at toolDiscovery (already-logged-in user with
      // empty bind list), the LoginPage isn't a meaningful "back" target — the
      // user can't unsign-in from here. Hide the back button in that case so
      // we don't strand them on a screen that does nothing.
      return (
        <ToolDiscoveryPage
          onBack={
            initialStep === "login" ? () => setStep("login") : undefined
          }
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
