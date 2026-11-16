import { useTranslation } from "react-i18next";
import type { AssistantMcpStep } from "@/lib/api/assistant";
import { LibraryResourceIcon } from "@/components/library/LibraryResourceIcon";

export function McpSteps({ steps }: { steps: AssistantMcpStep[] }) {
  const { t } = useTranslation();
  return (
    <div className="my-2 space-y-1 text-note text-ink-muted">
      <ol aria-label={t("assistant.mcp.steps")} className="space-y-1">
        {steps.map((step) => (
          <li key={step.id} className="flex items-center gap-2">
            <LibraryResourceIcon
              kind="mcp"
              name={step.server ?? step.tool ?? "MCP"}
              recipeId={step.recipeId ?? undefined}
              className="h-4 w-4 shrink-0"
            />
            <span className="min-w-0 break-words">
              {step.server ?? "MCP"}
              {step.tool ? ` · ${step.tool}` : ""}
            </span>
            <span
              className={
                step.status === "failed" || step.status === "unavailable"
                  ? "text-warn"
                  : "text-ink-faint"
              }
            >
              {t(`assistant.mcp.${step.reason ?? step.status}`)}
            </span>
          </li>
        ))}
      </ol>
      <p className="text-ink-faint">{t("assistant.mcp.externalCost")}</p>
    </div>
  );
}
