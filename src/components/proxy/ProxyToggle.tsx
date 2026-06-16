/**
 * 代理模式切换开关组件
 *
 * 放置在主界面头部，用于一键启用/关闭代理模式
 * 启用时自动接管 Live 配置，关闭时恢复原始配置
 */

import { Radio, Loader2 } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { useLockedTakeoverTools } from "@/hooks/useLockedTakeoverTools";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import type { AppId } from "@/lib/api";

interface ProxyToggleProps {
  className?: string;
  activeApp: AppId;
}

export function ProxyToggle({ className, activeApp }: ProxyToggleProps) {
  const { t } = useTranslation();
  const { isRunning, takeoverStatus, setTakeoverForApp, isPending, status } =
    useProxyStatus();
  const lockedTools = useLockedTakeoverTools();
  // Locked tools have their takeover forced ON by MainApp.tsx's reconciliation
  // sweep. Surfacing the Switch here would invite a turn-off path that the
  // sweep immediately undoes — confusing UX. Hide the Switch entirely for
  // these tools; the pulsing Radio indicator + tooltip carry the meaning.
  const isLocked = lockedTools.has(activeApp);

  const handleToggle = async (checked: boolean) => {
    try {
      await setTakeoverForApp({ appType: activeApp, enabled: checked });
    } catch (error) {
      console.error("[ProxyToggle] Toggle takeover failed:", error);
    }
  };

  const takeoverEnabled = takeoverStatus?.[activeApp] || false;

  const appLabel =
    activeApp === "claude"
      ? "Claude"
      : activeApp === "codex"
        ? "Codex"
        : activeApp === "gemini"
          ? "Gemini"
          : "OpenCode";

  const tooltipText = isLocked
    ? t("proxy.takeover.tooltip.locked", {
        appLabel,
        defaultValue: `${appLabel} 已锁定接管：cc-switch 运行期间自动启用`,
      })
    : takeoverEnabled
      ? isRunning
        ? t("proxy.takeover.tooltip.active", {
            appLabel,
            address: status?.address,
            port: status?.port,
            defaultValue: `${appLabel} 已接管 - ${status?.address}:${status?.port}\n切换该应用供应商为热切换`,
          })
        : t("proxy.takeover.tooltip.broken", {
            appLabel,
            defaultValue: `${appLabel} 已接管，但代理服务未运行`,
          })
      : t("proxy.takeover.tooltip.inactive", {
          appLabel,
          defaultValue: `接管 ${appLabel} 的 Live 配置，让该应用请求走本地代理`,
        });

  return (
    <div
      className={cn(
        "flex items-center gap-1 px-1.5 h-8 rounded-lg bg-muted/50 transition-all",
        className,
      )}
      title={tooltipText}
    >
      {isPending ? (
        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
      ) : (
        <Radio
          className={cn(
            "h-4 w-4 transition-colors",
            // Locked tools always render as "on" — the sweep keeps them that
            // way regardless of the live `takeoverEnabled` flag, which can
            // briefly read false during the reconciliation round-trip.
            isLocked || takeoverEnabled
              ? "text-emerald-500 animate-pulse"
              : "text-muted-foreground",
          )}
        />
      )}
      {!isLocked && (
        <Switch
          checked={takeoverEnabled}
          onCheckedChange={handleToggle}
          disabled={isPending}
        />
      )}
    </div>
  );
}
