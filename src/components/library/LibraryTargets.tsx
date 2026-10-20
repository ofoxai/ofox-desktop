import { Folder } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ResourceTarget } from "@/lib/api/library";
import { ToolBadge } from "@/components/tools/ToolBadge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { toolLabel } from "./libraryText";

export type TargetDelta = { addTools: string[]; withdrawTools: string[] };
export function LibraryTargets({
  name,
  targets,
  delta,
  observedTools = [],
  disabledTools = [],
  busy,
  onToggle,
}: {
  name: string;
  targets: ResourceTarget[];
  delta?: TargetDelta;
  observedTools?: string[];
  /** Observed tools whose matching entry is switched off there. */
  disabledTools?: string[];
  busy: boolean;
  onToggle: (target: ResourceTarget) => void;
}) {
  const { t } = useTranslation();
  return (
    <TooltipProvider delayDuration={150}>
      <div
        className="flex flex-wrap items-center gap-1"
        aria-label={t("library.workbench.assignments", { name })}
      >
        {targets.map((target) => {
          const changed =
            delta?.addTools.includes(target.toolId) ||
            delta?.withdrawTools.includes(target.toolId);
          const chosen =
            delta?.addTools.includes(target.toolId) ||
            (!!target.desired && !delta?.withdrawTools.includes(target.toolId));
          const shared =
            target.status === "shared" ||
            (!target.desired && target.readable === true);
          const blocked = !target.available && !target.desired;
          const state = changed
            ? "pending"
            : shared
              ? "shared"
              : ["conflict", "needsReview", "unreadable", "failed"].includes(
                    target.status,
                  )
                ? "attention"
                : target.status === "applied" && target.desired
                  ? "applied"
                  : target.desired
                    ? "pending"
                    : observedTools.includes(target.toolId)
                      ? "observed"
                      : "unassigned";
          const off =
            state === "observed" && disabledTools.includes(target.toolId);
          const label = `${name} · ${toolLabel(target.toolId, t)} · ${t(state === "observed" ? (off ? "library.workbench2.observedDisabled" : "library.workbench2.observed") : `library.workbench.${state}`)}`;
          return (
            <Tooltip key={target.toolId}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  aria-label={label}
                  data-state={state}
                  aria-pressed={shared ? undefined : !!chosen}
                  aria-disabled={busy || blocked}
                  onClick={() => {
                    if (!busy && !blocked) onToggle(target);
                  }}
                  className={cn(
                    "relative grid h-8 w-8 place-items-center rounded-ctl border border-line bg-card outline-none transition-colors hover:bg-pill focus-visible:ring-2 focus-visible:ring-brand",
                    chosen && "border-brand/50 bg-brand/10",
                    shared && "border-dashed",
                    changed && "ring-1 ring-brand",
                    state === "observed" && "border-line bg-pill",
                  )}
                >
                  <span aria-hidden>
                    {target.toolId === "shared" ? (
                      <Folder className="h-5 w-5 text-ink-muted" />
                    ) : (
                      <ToolBadge
                        toolId={target.toolId}
                        size={20}
                        variant="plain"
                        dimmed={
                          !chosen && !shared && (state !== "observed" || off)
                        }
                      />
                    )}
                  </span>
                  {["applied", "pending"].includes(state) && (
                    <span
                      aria-hidden
                      className="absolute -bottom-1 -right-1 rounded-full bg-card px-0.5 text-micro text-brand"
                    >
                      {state === "applied" ? "✓" : "±"}
                    </span>
                  )}
                  {state === "attention" && (
                    <span
                      aria-hidden
                      className="absolute -right-1 -top-1 rounded-full bg-card px-1 text-micro text-warn"
                    >
                      !
                    </span>
                  )}
                  {shared && (
                    <span
                      aria-hidden
                      className="absolute -bottom-1 -right-1 rounded-full bg-card px-0.5 text-micro text-ink-muted"
                    >
                      ↗
                    </span>
                  )}
                  {state === "observed" && (
                    <span
                      aria-hidden
                      className="absolute -bottom-1 -right-1 rounded-full bg-card px-0.5 text-micro text-ink-muted"
                    >
                      ◇
                    </span>
                  )}
                </button>
              </TooltipTrigger>
              <TooltipContent>
                <p>{label}</p>
                {state === "observed" && (
                  <p>
                    {t(
                      off
                        ? "library.workbench2.observedDisabledHint"
                        : "library.workbench2.observedHint",
                    )}
                  </p>
                )}
                {target.readers && target.readers.length > 1 && (
                  <p>
                    {t("library.readers", {
                      tools: target.readers
                        .map((tool) => toolLabel(tool, t))
                        .join(", "),
                    })}
                  </p>
                )}
                {(blocked || shared || target.disabledReason) && (
                  <p>
                    {blocked
                      ? t("library.nativeCliRequired")
                      : shared
                        ? t("library.sharedReadable")
                        : t(target.disabledReason!, {
                            defaultValue: t("library.unknown"),
                          })}
                  </p>
                )}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </TooltipProvider>
  );
}
