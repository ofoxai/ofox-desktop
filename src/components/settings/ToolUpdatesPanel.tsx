import { useTranslation } from "react-i18next";
import { ArrowUpCircle, ExternalLink, Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { settingsApi } from "@/lib/api";
import {
  checkToolUpdates,
  updateTools,
  useToolUpdates,
} from "@/hooks/useToolUpdates";

export function ToolUpdatesPanel() {
  const { t } = useTranslation();
  const state = useToolUpdates();
  const available = state.tools.filter(
    (tool) => tool.update_status === "available" && tool.update_supported,
  );
  const pending = state.tools.filter(
    (tool) => tool.update_status === "available",
  );
  const orderedTools = [...TOOL_ORDER].sort(
    (a, b) =>
      Number(pending.some((tool) => tool.name === b)) -
      Number(pending.some((tool) => tool.name === a)),
  );
  return (
    <section className="space-y-3" aria-label={t("toolUpdates.title")}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-medium">{t("toolUpdates.title")}</h3>
        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={state.checking || state.batch}
            onClick={() => void checkToolUpdates()}
          >
            <RefreshCw
              className={`mr-1 h-3.5 w-3.5 ${state.checking ? "animate-spin" : ""}`}
            />
            {t(state.checking ? "toolUpdates.checking" : "toolUpdates.check")}
          </Button>
          <Button
            size="sm"
            className="bg-orange-500 text-white hover:bg-orange-600"
            disabled={state.checking || state.batch || !available.length}
            onClick={() => void updateTools(available.map((tool) => tool.name))}
          >
            {t("toolUpdates.updateAll")}
            {available.length > 0 ? ` (${available.length})` : ""}
          </Button>
        </div>
      </div>
      {pending.length > 0 && (
        <div
          role="status"
          className="flex items-start gap-2 rounded-lg border border-orange-300 bg-orange-50 p-3 text-sm text-orange-900 dark:border-orange-500/50 dark:bg-orange-500/10 dark:text-orange-200"
        >
          <ArrowUpCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div>
            <p className="font-semibold">
              {t("toolUpdates.pendingCount", { count: pending.length })}
            </p>
            <p className="mt-1 text-xs">
              {t("toolUpdates.actionCounts", {
                automatic: available.length,
                manual: pending.length - available.length,
              })}
            </p>
          </div>
        </div>
      )}
      {state.error && (
        <p role="alert" className="text-xs text-destructive">
          {t("toolUpdates.failed")}
        </p>
      )}
      <div className="grid gap-3 sm:grid-cols-2">
        {orderedTools.map((name) => {
          const tool = state.tools.find((tool) => tool.name === name);
          const result = state.results[name];
          const busy = state.busy === name;
          const hasUpdate = tool?.update_status === "available";
          const appManaged =
            tool?.installationKind === "desktopApp" || name === "workbuddy";
          return (
            <div
              key={name}
              data-tool={name}
              className={`min-w-0 space-y-3 rounded-lg border p-3 ${hasUpdate ? "border-orange-400 bg-orange-50/60 ring-1 ring-orange-400/30 dark:bg-orange-500/10" : "border-border"}`}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-medium">
                  {TOOL_META[name].label}
                </span>
                {hasUpdate && !busy && (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-orange-100 px-2 py-0.5 text-xs font-semibold text-orange-800 dark:bg-orange-500/20 dark:text-orange-200">
                    <ArrowUpCircle className="h-3 w-3" />
                    {t("toolUpdates.newVersion")}
                  </span>
                )}
                {busy && (
                  <Loader2
                    className="h-4 w-4 animate-spin"
                    aria-label={t("toolUpdates.updating")}
                  />
                )}
              </div>
              <div className="text-xs text-muted-foreground">
                {t("toolUpdates.version", { version: tool?.version ?? "—" })}
                {tool?.latest_version && (
                  <span
                    className={`ml-2 ${hasUpdate ? "font-semibold text-orange-700 dark:text-orange-300" : ""}`}
                  >
                    {t("toolUpdates.latest", { version: tool.latest_version })}
                  </span>
                )}
              </div>
              <p className="text-xs" aria-live="polite">
                {t(
                  `toolUpdates.${busy ? "updating" : state.checking ? "checking" : appManaged ? "appManaged" : (tool?.update_status ?? "unchecked")}`,
                )}
              </p>
              {hasUpdate && tool?.update_source === "pnpm" && (
                <p className="text-xs text-muted-foreground">
                  {t("toolUpdates.pnpmPolicy")}
                </p>
              )}
              {appManaged ? (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    void settingsApi
                      .openExternal(
                        name === "workbuddy"
                          ? "https://www.workbuddy.cn/work/"
                          : "https://chatgpt.com/download/",
                      )
                      .catch(() =>
                        toast.error(t("settings.openReleaseNotesFailed")),
                      )
                  }
                >
                  {t("toolUpdates.download")}
                </Button>
              ) : (tool?.update_status === "available" ||
                  tool?.update_status === "broken") &&
                tool.update_supported ? (
                <Button
                  size="sm"
                  className="w-full bg-orange-500 text-white hover:bg-orange-600"
                  disabled={state.batch || state.checking}
                  onClick={() => void updateTools([name])}
                >
                  <ArrowUpCircle className="mr-1.5 h-4 w-4" />
                  {t(
                    tool.update_status === "broken"
                      ? "toolUpdates.repair"
                      : "toolUpdates.update",
                  )}
                </Button>
              ) : (hasUpdate || tool?.update_status === "broken") &&
                !tool?.update_supported ? (
                <div className="space-y-2">
                  <p className="text-xs text-muted-foreground">
                    {t("toolUpdates.manual")}
                  </p>
                  <Button
                    size="sm"
                    variant="outline"
                    className="w-full border-orange-400 text-orange-700 dark:text-orange-300"
                    onClick={() =>
                      void settingsApi
                        .openExternal(TOOL_META[name].projectUrl)
                        .catch(() =>
                          toast.error(t("settings.openReleaseNotesFailed")),
                        )
                    }
                  >
                    <ExternalLink className="mr-1.5 h-4 w-4" />
                    {t("toolUpdates.manualAction")}
                  </Button>
                </div>
              ) : null}
              {result && (
                <p role="status" className="text-xs">
                  {t(`toolUpdates.result.${result}`)}
                </p>
              )}
              {tool?.executable_path ||
              tool?.update_reason ||
              state.logs[name]?.length ? (
                <details className="text-xs text-muted-foreground">
                  <summary className="cursor-pointer">
                    {t("toolUpdates.details")}
                  </summary>
                  <pre className="mt-2 max-h-36 overflow-auto whitespace-pre-wrap break-all text-[11px]">
                    {[
                      tool?.update_source,
                      tool?.executable_path,
                      tool?.update_reason,
                      ...(state.logs[name] ?? []),
                    ]
                      .filter(Boolean)
                      .join("\n")}
                  </pre>
                </details>
              ) : null}
            </div>
          );
        })}
      </div>
    </section>
  );
}
