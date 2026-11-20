import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
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
  onEnterLocal?: () => void;
}

export default function OnboardingApp({
  initialStep = "login",
  onComplete,
  onEnterLocal,
}: OnboardingAppProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>(initialStep);
  const [boundTools, setBoundTools] = useState<string[]>([]);
  const [binding, setBinding] = useState(false);
  const bindingInFlight = useRef(false);

  // Even from local management, a sign-in goes on to binding.
  const handleLoginSuccess = (_user: OfoxUserInfo) => {
    setStep("toolDiscovery");
  };

  const handleBind = async (selectedTools: string[]) => {
    if (bindingInFlight.current || selectedTools.length === 0) return;
    bindingInFlight.current = true;
    setBinding(true);
    try {
      // The result includes previous bindings; only this selection can prove
      // completion. Keep failed selections available for an explicit retry.
      const succeeded = await bindTools(selectedTools);
      setBoundTools(succeeded);
      if (selectedTools.every((tool) => succeeded.includes(tool))) {
        setStep("setupComplete");
      }
    } catch {
      toast.error(t("uiCopy.bindFailed"));
    } finally {
      bindingInFlight.current = false;
      setBinding(false);
    }
  };

  switch (step) {
    case "login":
      return (
        <LoginPage
          onLoginSuccess={handleLoginSuccess}
          onEnterLocal={onEnterLocal}
        />
      );
    case "toolDiscovery":
      // If we entered the wizard at toolDiscovery (already-logged-in user with
      // empty bind list), the LoginPage isn't a meaningful "back" target — the
      // user can't unsign-in from here. Hide the back button in that case so
      // we don't strand them on a screen that does nothing.
      return (
        <ToolDiscoveryPage
          onBack={initialStep === "login" ? () => setStep("login") : undefined}
          onBind={handleBind}
          onEnterLocal={onEnterLocal}
          binding={binding}
        />
      );
    case "setupComplete":
      return (
        <SetupCompletePage boundTools={boundTools} onOpenConsole={onComplete} />
      );
  }
}
