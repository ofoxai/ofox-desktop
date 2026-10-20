import { ArrowDownToLine, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { TOOL_META } from "@/config/toolMeta";
import type { BoundTool } from "@/hooks/useBoundTools";
import { batchable, useInlineToolUpdates } from "@/hooks/useInlineToolUpdates";

interface UpdateAllLineProps {
  /** The installed tools in the list. */
  tools: BoundTool[];
  updates: ReturnType<typeof useInlineToolUpdates>;
}

/**
 * magpie's "Update all" line above the list: with two or more tools behind,
 * it says which and updates them one after another; while that runs, which
 * one it is on.
 */
export function UpdateAllLine({ tools, updates }: UpdateAllLineProps) {
  const { t } = useTranslation();
  const { state } = updates;
  const behind = tools.filter(
    (tool) => !tool.installationError && updates.updateFor(tool),
  );
  const batch = behind.filter((tool) => batchable(updates.updateFor(tool)!));
  const running = state.batch && state.queue.length > 1;
  if (!running && behind.length < 2) return null;

  const now = state.busy ?? state.queue[0];
  return (
    <div
      role="status"
      className="mx-0.5 mb-2.5 flex min-h-[22px] items-center gap-2.5 text-nav text-ink-2"
    >
      <span className="min-w-0 flex-1 truncate">
        {running
          ? t("toolUpdates.batchProgress", {
              tool: TOOL_META[now]?.label ?? now,
              index: state.queue.indexOf(now) + 1,
              count: state.queue.length,
            })
          : t("toolUpdates.manyAvailable", {
              count: behind.length,
              names: behind.map((tool) => tool.label).join(", "),
            })}
      </span>
      {running ? (
        <Loader2
          aria-hidden
          className="h-3.5 w-3.5 flex-none animate-spin text-ink-muted"
        />
      ) : (
        batch.length >= 2 && (
          <Button
            variant="pill"
            disabled={state.batch}
            onClick={() => void updates.updateAll(batch)}
            title={batch
              .map((tool) => {
                const update = updates.updateFor(tool);
                return `${tool.label}: ${update?.version} → ${update?.latest_version}`;
              })
              .join("\n")}
            className="h-5 flex-none gap-1 pl-1.5 pr-2"
          >
            <ArrowDownToLine aria-hidden className="h-3 w-3 text-ink-muted" />
            {t("toolUpdates.updateAll")}
          </Button>
        )
      )}
    </div>
  );
}
