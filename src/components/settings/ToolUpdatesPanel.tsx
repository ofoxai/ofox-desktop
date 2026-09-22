import { useTranslation } from "react-i18next";
import { Loader2, RefreshCw } from "lucide-react";
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
            disabled={state.checking || state.batch || !available.length}
            onClick={() => void updateTools(available.map((tool) => tool.name))}
          >
            {t("toolUpdates.updateAll")}
          </Button>
        </div>
      </div>
      {state.error && (
        <p role="alert" className="text-xs text-destructive">
          {t("toolUpdates.failed")}
        </p>
      )}
      <div className="grid gap-3 sm:grid-cols-2">
        {TOOL_ORDER.map((name) => {
          const tool = state.tools.find((tool) => tool.name === name);
          const result = state.results[name];
          const busy = state.busy === name;
          const appManaged =
            tool?.installationKind === "desktopApp" || name === "workbuddy";
          return (
            <div key={name} className="min-w-0 space-y-2 rounded-lg border p-3">
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-medium">
                  {TOOL_META[name].label}
                </span>
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
                  <span className="ml-2">
                    {t("toolUpdates.latest", { version: tool.latest_version })}
                  </span>
                )}
              </div>
              <p className="text-xs" aria-live="polite">
                {t(
                  `toolUpdates.${busy ? "updating" : state.checking ? "checking" : appManaged ? "appManaged" : (tool?.update_status ?? "unchecked")}`,
                )}
              </p>
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
              ) : tool?.update_status === "available" &&
                tool.update_supported ? (
                <Button
                  size="sm"
                  disabled={state.batch || state.checking}
                  onClick={() => void updateTools([name])}
                >
                  {t("toolUpdates.update")}
                </Button>
              ) : tool?.version && !tool.update_supported ? (
                <p className="text-xs text-muted-foreground">
                  {t("toolUpdates.manual")}
                </p>
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
