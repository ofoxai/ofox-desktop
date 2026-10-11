import { useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ArrowRight, ChevronRight } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { TOOL_META } from "@/config/toolMeta";
import { settingsApi } from "@/lib/api";
import type { NodePrompt } from "@/hooks/useToolInstall";

interface NodeUpgradeDialogProps {
  prompt: NodePrompt | null;
  onConfirm: () => void;
  onCancel: () => void;
}

/** 步骤里说「用 X 升级」时的名字；Windows 官方安装包由 winget 升级。 */
const UPGRADER: Record<string, string> = {
  fnm: "fnm",
  nvm: "nvm",
  "nvm-windows": "nvm-windows",
  volta: "Volta",
  homebrew: "Homebrew",
  nodejs: "winget",
};

const primaryButton =
  "rounded-md bg-gradient-to-r from-orange-400 to-orange-500 px-4 py-2 text-sm font-medium text-white shadow-sm hover:from-orange-500 hover:to-orange-600 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-orange-400/50 focus-visible:ring-offset-2 focus-visible:ring-offset-background";
const secondaryButton =
  "rounded-md px-4 py-2 text-sm text-muted-foreground hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-orange-400/50";

function Step({
  index,
  title,
  children,
}: {
  index: number;
  title: string;
  children?: ReactNode;
}) {
  return (
    <li className="flex gap-3">
      <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-orange-100 text-xs font-semibold text-orange-600 dark:bg-orange-500/15 dark:text-orange-400">
        {index}
      </span>
      <div className="min-w-0 space-y-1">
        <p className="font-medium leading-5">{title}</p>
        {children && (
          <div className="text-xs leading-relaxed text-muted-foreground">
            {children}
          </div>
        )}
      </div>
    </li>
  );
}

/**
 * 工具要求更新的 Node.js 时，说清楚差多少、Ofox 会做哪几步、为什么要重装其它
 * 工具，让用户决定要不要升级。完整命令折叠起来，想看再展开。Ofox 升级不了的，
 * 指向 Node.js 官网。
 */
export default function NodeUpgradeDialog({
  prompt,
  onConfirm,
  onCancel,
}: NodeUpgradeDialogProps) {
  const { t } = useTranslation();
  const primaryRef = useRef<HTMLButtonElement>(null);
  if (!prompt) return null;
  const { requirement } = prompt;
  const label = (id: string) => TOOL_META[id]?.label ?? id;
  const tool = label(prompt.toolId);
  const managerKey = requirement.manager ?? "unknown";
  const managedBy = t(`toolInstall.nodeUpgrade.manager.${managerKey}`);
  const upgrader = UPGRADER[managerKey] ?? managerKey;
  const reinstall = requirement.canUpgrade ? requirement.reinstall : [];
  const openDownload = () =>
    void settingsApi
      .openExternal(t("toolInstall.nodeUpgrade.downloadUrl"))
      .catch(() => toast.error(t("notifications.openLinkFailed")));

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent
        className="sm:max-w-[480px]"
        zIndex="alert"
        // 默认会聚焦到第一个可聚焦元素（折叠的命令），落在主按钮上更自然。
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          primaryRef.current?.focus();
        }}
      >
        <DialogHeader>
          <DialogTitle>
            {t("toolInstall.nodeUpgrade.title", { tool })}
          </DialogTitle>
          <DialogDescription>
            {t("toolInstall.nodeUpgrade.summary", { tool })}
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5 text-sm">
          <div className="flex items-start gap-4 rounded-lg border border-border-default bg-muted/30 px-4 py-3">
            <div className="min-w-0 flex-1">
              <p className="text-xs text-muted-foreground">
                {t("toolInstall.nodeUpgrade.current")}
              </p>
              <p className="mt-1 font-mono text-base font-semibold">
                {requirement.current}
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {managedBy}
              </p>
            </div>
            <ArrowRight className="mt-6 h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <p className="text-xs text-muted-foreground">
                {t("toolInstall.nodeUpgrade.minimum")}
              </p>
              <p className="mt-1 break-all font-mono text-base font-semibold text-orange-600 dark:text-orange-400">
                {requirement.minimum ?? requirement.required}
              </p>
            </div>
          </div>

          {requirement.canUpgrade ? (
            <div className="space-y-3">
              <p className="text-muted-foreground">
                {t("toolInstall.nodeUpgrade.stepsTitle")}
              </p>
              <ol className="space-y-3">
                <Step
                  index={1}
                  title={t("toolInstall.nodeUpgrade.stepUpgrade", {
                    manager: upgrader,
                  })}
                >
                  {requirement.needsAdmin &&
                    t("toolInstall.nodeUpgrade.needsAdmin")}
                </Step>
                {reinstall.length > 0 && (
                  <Step
                    index={2}
                    title={t("toolInstall.nodeUpgrade.stepReinstall", {
                      tools: reinstall
                        .map(label)
                        .join(t("toolInstall.nodeUpgrade.listSeparator")),
                    })}
                  >
                    {t("toolInstall.nodeUpgrade.reinstallWhy", {
                      manager: upgrader,
                    })}
                  </Step>
                )}
                <Step
                  index={reinstall.length > 0 ? 3 : 2}
                  title={t("toolInstall.nodeUpgrade.stepInstall", { tool })}
                />
              </ol>
              {requirement.command && (
                <details className="group">
                  <summary className="flex w-fit cursor-pointer list-none items-center gap-1 rounded text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-orange-400/50 [&::-webkit-details-marker]:hidden">
                    <ChevronRight className="h-3.5 w-3.5 transition-transform group-open:rotate-90" />
                    {t("toolInstall.nodeUpgrade.showCommand")}
                  </summary>
                  <pre className="mt-2 whitespace-pre-wrap break-words rounded-md bg-muted px-3 py-2 font-mono text-xs leading-relaxed">
                    {requirement.command}
                  </pre>
                  {requirement.required && (
                    <p className="mt-2 text-xs text-muted-foreground">
                      {t("toolInstall.nodeUpgrade.fullRequirement", {
                        tool,
                        range: requirement.required,
                      })}
                    </p>
                  )}
                </details>
              )}
            </div>
          ) : (
            <p className="leading-relaxed text-muted-foreground">
              {t("toolInstall.nodeUpgrade.manualOnly", {
                manager: managedBy,
                tool,
              })}
            </p>
          )}
        </div>

        <DialogFooter className="py-4">
          {requirement.canUpgrade ? (
            <>
              <button
                type="button"
                onClick={onCancel}
                className={secondaryButton}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                onClick={onConfirm}
                ref={primaryRef}
                className={primaryButton}
              >
                {t("toolInstall.nodeUpgrade.confirm")}
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                onClick={openDownload}
                className={secondaryButton}
              >
                {t("toolInstall.nodeUpgrade.openDownload")}
              </button>
              <button
                type="button"
                onClick={onCancel}
                ref={primaryRef}
                className={primaryButton}
              >
                {t("toolInstall.nodeUpgrade.ok")}
              </button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
