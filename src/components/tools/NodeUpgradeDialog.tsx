import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { TOOL_META } from "@/config/toolMeta";
import type { NodePrompt } from "@/hooks/useToolInstall";

interface NodeUpgradeDialogProps {
  prompt: NodePrompt | null;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * 工具要求更新的 Node.js 时，说清楚原因和后果，让用户决定要不要升级：会运行
 * 什么、要不要管理员授权、哪些工具要重装。Ofox 升级不了的，只给手动做法。
 */
export default function NodeUpgradeDialog({
  prompt,
  onConfirm,
  onCancel,
}: NodeUpgradeDialogProps) {
  const { t } = useTranslation();
  if (!prompt) return null;
  const { requirement } = prompt;
  const label = (id: string) => TOOL_META[id]?.label ?? id;
  const tool = label(prompt.toolId);
  const manager = t(
    `toolInstall.nodeUpgrade.manager.${requirement.manager ?? "unknown"}`,
  );

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-w-md" zIndex="alert">
        <DialogHeader className="space-y-3 border-b-0 bg-transparent pb-0">
          <DialogTitle className="flex items-center gap-2 text-lg font-semibold">
            <AlertTriangle className="h-5 w-5 text-orange-500" />
            {t("toolInstall.nodeUpgrade.title", { tool })}
          </DialogTitle>
          <DialogDescription className="text-sm leading-relaxed">
            {t("toolInstall.nodeUpgrade.body", {
              tool,
              required: requirement.required,
              current: requirement.current,
              manager,
            })}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2 text-sm">
          <p>
            {t(
              requirement.canUpgrade
                ? "toolInstall.nodeUpgrade.willRun"
                : "toolInstall.nodeUpgrade.manualOnly",
            )}
          </p>
          {requirement.manual && (
            <code className="block whitespace-pre-wrap break-all rounded-md bg-muted px-3 py-2 text-xs">
              {requirement.manual}
            </code>
          )}
          {requirement.canUpgrade && requirement.needsAdmin && (
            <p className="text-muted-foreground">
              {t("toolInstall.nodeUpgrade.needsAdmin")}
            </p>
          )}
          {requirement.canUpgrade && requirement.reinstall.length > 0 && (
            <p className="text-muted-foreground">
              {t("toolInstall.nodeUpgrade.reinstall", {
                tools: requirement.reinstall
                  .map(label)
                  .join(t("toolInstall.nodeUpgrade.listSeparator")),
              })}
            </p>
          )}
        </div>
        <DialogFooter className="flex gap-2 border-t-0 bg-transparent pt-2 sm:justify-end">
          {requirement.canUpgrade ? (
            <>
              <Button variant="outline" onClick={onCancel}>
                {t("common.cancel")}
              </Button>
              <Button onClick={onConfirm}>
                {t("toolInstall.nodeUpgrade.confirm")}
              </Button>
            </>
          ) : (
            <Button onClick={onCancel}>
              {t("toolInstall.nodeUpgrade.ok")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
