import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { TOOL_META } from "@/config/toolMeta";
import { useToolUninstall } from "@/hooks/useToolUninstall";
import { useUninstallPreview } from "@/hooks/useUninstallPreview";
import type { UninstallResult } from "@/lib/api/toolUninstall";

interface Props {
  tool: string | null;
  onOpenChange: (open: boolean) => void;
  onCompleted: (result: UninstallResult) => void;
}
export function UninstallToolDialog(props: Props) {
  // A fresh instance gives every newly opened tool its own result and operation.
  return props.tool ? (
    <UninstallContent key={props.tool} {...props} tool={props.tool} />
  ) : null;
}
function UninstallContent({
  tool,
  onOpenChange,
  onCompleted,
}: Props & { tool: string }) {
  const { t } = useTranslation();
  const preview = useUninstallPreview(tool);
  const job = useToolUninstall((result) => {
    onCompleted(result);
    // A remaining copy may use a different installer. Refresh its command.
    if (result.status === "stillPresent") preview.retry();
    if (result.status === "removed") toast.success(t("toolUninstall.success"));
  });
  const plan = preview.data;
  const result = job.result;
  const command =
    result && job.running
      ? null
      : result || plan?.blockedReason
        ? (plan?.manualCommand ?? plan?.command)
        : plan?.command;
  const keeps = result?.kept ?? plan?.keeps ?? [];
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!job.running) onOpenChange(open);
      }}
    >
      <DialogContent
        aria-busy={job.running}
        onEscapeKeyDown={(event) => {
          if (job.running) event.preventDefault();
        }}
        className="max-w-lg"
      >
        <DialogHeader>
          <DialogTitle>
            {t("toolUninstall.title", { tool: TOOL_META[tool]?.label ?? tool })}
          </DialogTitle>
          <DialogDescription>
            {t("toolUninstall.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 space-y-3 overflow-y-auto px-[18px] py-4 text-ui">
          {!plan && !preview.error && (
            <p role="status">{t("common.loading")}</p>
          )}
          {(preview.error || job.error) && (
            <p role="alert" className="break-words text-danger">
              {preview.error || job.error}
            </p>
          )}
          {plan && !result && (
            <ol className="list-decimal space-y-2 pl-5">
              {plan.bound && <li>{t("toolUninstall.restore")}</li>}
              <li>
                {t(`toolUninstall.keyPlan.${plan.key}`)}
                {plan.sharedWith.length > 0 &&
                  ` · ${plan.sharedWith.map((id) => TOOL_META[id]?.label ?? id).join(", ")}`}
              </li>
              <li>
                {t("toolUninstall.remove", { source: plan.source })}
                <ul className="mt-1 space-y-1 font-mono text-sub text-ink-muted">
                  {plan.removes.map((path) => (
                    <li key={path} className="break-all">
                      {path}
                    </li>
                  ))}
                </ul>
              </li>
            </ol>
          )}
          {plan?.blockedReason && !result && (
            <p role="alert" className="rounded-ctl bg-warn-soft p-3 text-warn">
              {t(`toolUninstall.blocked.${plan.blockedReason}`)}
            </p>
          )}
          {plan && !plan.installed && <p>{t("toolUninstall.notInstalled")}</p>}
          {result && (
            <div
              role="status"
              className="space-y-2 rounded-ctl bg-surface-alt p-3"
            >
              <p
                className={
                  result.status === "removed" ? "text-ok" : "text-warn"
                }
              >
                {t(
                  result.status === "removed"
                    ? "toolUninstall.success"
                    : "toolUninstall.partial",
                )}
              </p>
              {result.unbind && <p>{t("toolUninstall.unbound")}</p>}
              <p>{t(`toolUninstall.keyResult.${result.key}`)}</p>
              {result.remainingPath && (
                <code className="block break-all text-sub">
                  {result.remainingPath}
                </code>
              )}
              {result.unbind?.keptKeys?.length ? (
                <p className="break-all">
                  {t("toolUninstall.keptChanges", {
                    keys: result.unbind.keptKeys.join(", "),
                  })}
                </p>
              ) : null}
              {result.unbind?.warnings?.map((warning, index) => (
                <p key={index} className="text-warn">
                  {t(`unbind.warning.${warning.code}`, {
                    defaultValue: warning.code,
                    keys: result.unbind?.keptKeys.join(", "),
                  })}
                </p>
              ))}
            </div>
          )}
          {command && (
            <div className="flex items-start gap-2 rounded-ctl bg-pill p-2">
              <code className="min-w-0 flex-1 break-all text-sub">
                {command}
              </code>
              <CopyButton
                text={command}
                aria-label={t("toolUninstall.copyCommand")}
              />
            </div>
          )}
          {keeps.length > 0 && (
            <div>
              <p className="font-medium">{t("toolUninstall.keeps")}</p>
              <ul className="mt-1 space-y-1 font-mono text-sub text-ink-muted">
                {keeps.map((path) => (
                  <li key={path} className="break-all">
                    {path}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {job.running && (
            <p role="status">
              {job.progress
                ? t("toolUninstall.progress", {
                    step: job.progress.step,
                    total: job.progress.totalSteps,
                    name: t(`toolUninstall.step.${job.progress.step}`),
                  })
                : t("toolUninstall.starting")}
            </p>
          )}
          {job.logs.length > 0 && (
            <pre
              aria-label={t("toolUninstall.logs")}
              className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-ctl bg-pill p-2 text-note text-ink-muted"
            >
              {job.logs.join("\n")}
            </pre>
          )}
        </div>
        <DialogFooter>
          <Button
            variant="secondary"
            disabled={job.running}
            onClick={() => onOpenChange(false)}
          >
            {t(result ? "common.close" : "common.cancel")}
          </Button>
          {!job.running &&
            (preview.error || (!result && plan?.blockedReason)) && (
              <Button variant="secondary" onClick={preview.retry}>
                {t("common.retry")}
              </Button>
            )}
          {plan?.installed && !plan.blockedReason && !result && (
            <Button
              variant="danger"
              disabled={job.running}
              onClick={() => void job.run(tool)}
            >
              {t("toolUninstall.confirm")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
