import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ListCard, ListFold } from "@/components/ui/list";
import { TOOL_LIST_GRID } from "./BoundToolRow";
import { cn } from "@/lib/utils";
import type { BoundTool } from "@/hooks/useBoundTools";

interface MissingToolsFoldProps {
  tools: BoundTool[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  renderTool: (tool: BoundTool) => ReactNode;
}

const MISSING_TOOLS_ID = "missing-bound-tools";

/**
 * Bound tools that are no longer on this machine, folded under the list.
 * Their bindings are kept, so each row still offers install and 管理.
 */
export function MissingToolsFold({
  tools,
  open,
  onOpenChange,
  renderTool,
}: MissingToolsFoldProps) {
  const { t } = useTranslation();
  if (!tools.length) return null;
  return (
    <div className="mt-2.5">
      <ListFold
        open={open}
        onOpenChange={onOpenChange}
        aria-controls={MISSING_TOOLS_ID}
        note={
          <span className="font-normal">
            · {t("toolLifecycle.bindingRetained")}
          </span>
        }
      >
        {t("toolLifecycle.missingTools", { count: tools.length })}
      </ListFold>
      {open && (
        <ListCard
          id={MISSING_TOOLS_ID}
          className={cn("mt-1.5", TOOL_LIST_GRID)}
        >
          {tools.map(renderTool)}
        </ListCard>
      )}
    </div>
  );
}
