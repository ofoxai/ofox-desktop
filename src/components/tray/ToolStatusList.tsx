import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import { ChevronRight } from "lucide-react";
import { useModelSelection } from "@/hooks/useModelSelection";
import {
  QuickModelPicker,
  modelNotes,
} from "@/components/model-picker/QuickModelPicker";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { Button } from "@/components/ui/button";
import { ListCard } from "@/components/ui/list";
import { EffortSlider } from "@/components/model-picker/EffortSlider";
import type { ToolModelSnapshot } from "@/lib/api/modelSelection";
import { cn } from "@/lib/utils";
import { OFOX_START_BINDING_EVENT, openMainWindow } from "./trayActions";

interface ToolStatusListProps {
  boundTools: string[];
  disabled?: boolean;
}

/** magpie's panel card: no border, a hairline ring and a breath of shadow. */
const PANEL_CARD =
  "rounded-[10px] border-0 shadow-[0_0_0_1px_rgba(0,0,0,.05),0_1px_2px_rgba(0,0,0,.04)] dark:shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)]";

/** What a shut row says about its model, and in what tone. */
function summary(
  snapshot: ToolModelSnapshot | undefined,
  loading: boolean,
  t: TFunction,
): { text: string; warn?: boolean; faint?: boolean } {
  if (!snapshot)
    return loading
      ? { text: "…", faint: true }
      : { text: t("tray.readFailed"), warn: true };
  const { binding, managedSelection: selection } = snapshot;
  if (binding.status === "paused")
    return { text: t("modelExperience.pausedShort"), warn: true };
  if (binding.status === "unknown")
    return { text: t("tray.readFailed"), warn: true };
  if (binding.status !== "configured")
    return { text: t(`toolLifecycle.binding.${binding.status}`), warn: true };
  if (selection.kind === "models")
    return {
      text: t("modelExperience.modelCount", { count: selection.models.length }),
    };
  return selection.model
    ? { text: selection.model }
    : { text: t("modelExperience.chooseModel"), faint: true };
}

/**
 * The bound tools as magpie's panel lists agents: one quiet 36px line each,
 * the model in words, and a row's controls only once it is opened, one row
 * at a time, in place.
 */
export default function ToolStatusList({
  boundTools,
  disabled = false,
}: ToolStatusListProps) {
  const { t } = useTranslation();
  const rows = boundTools
    .filter((id) => TOOL_META[id])
    .sort((a, b) => TOOL_ORDER.indexOf(a) - TOOL_ORDER.indexOf(b));
  const modelState = useModelSelection(rows);
  const snapshotByTool = new Map(
    modelState.data?.tools.map((snapshot) => [snapshot.toolId, snapshot]),
  );
  const [openId, setOpenId] = useState<string | null>(null);
  const openMain = (request?: string) =>
    void openMainWindow(request).catch((e) => {
      console.error("[ToolStatusList] open main window failed", e);
      toast.error(t("uiCopy.openMainFailed"));
    });

  if (rows.length === 0)
    return (
      <ListCard
        className={cn(
          PANEL_CARD,
          "flex flex-col items-center divide-y-0 px-4 py-6 text-center",
        )}
      >
        <p className="text-ui font-medium text-ink">{t("tray.noToolsTitle")}</p>
        <p className="mb-3 mt-1 text-note text-ink-muted">
          {t("tray.noToolsHint")}
        </p>
        <Button
          variant="primary"
          onClick={() => openMain(OFOX_START_BINDING_EVENT)}
        >
          {t("tray.bindTools")}
        </Button>
      </ListCard>
    );

  return (
    <ListCard className={PANEL_CARD}>
      {rows.map((id) => {
        const snapshot = snapshotByTool.get(id);
        const said = summary(snapshot, modelState.isPending, t);
        const open = openId === id;
        const panelId = `tray-tool-${id}`;
        const meta = TOOL_META[id];
        const notes = modelNotes(snapshot, id, t);
        return (
          <div key={id} className={cn(open && "bg-card-2")}>
            <button
              type="button"
              aria-expanded={open}
              aria-controls={panelId}
              onClick={() => setOpenId(open ? null : id)}
              className="flex h-9 w-full items-center gap-2.5 px-3 text-left transition-colors hover:bg-pill/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand/50"
            >
              <ToolBadge toolId={id} size={18} />
              <span className="min-w-0 max-w-[45%] flex-none truncate text-ui font-medium text-ink">
                {meta.label}
              </span>
              <span
                className={cn(
                  "min-w-0 flex-1 truncate text-right text-ctl",
                  said.warn
                    ? "text-warn"
                    : said.faint
                      ? "text-ink-faint"
                      : "text-ink-muted",
                )}
              >
                {said.text}
              </span>
              <ChevronRight
                aria-hidden
                className={cn(
                  "h-3 w-3 flex-none text-ink-faint transition-transform duration-200 ease-soft motion-reduce:transition-none",
                  open && "rotate-90",
                )}
              />
            </button>
            {open && (
              <div id={panelId} className="space-y-2 pb-3 pl-[40px] pr-3">
                {snapshot?.binding.status === "paused" ? (
                  <>
                    <p className="text-note text-ink-muted">
                      {t("modelExperience.paused")}
                    </p>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => openMain()}
                    >
                      {t("tray.openOfox")}
                    </Button>
                  </>
                ) : (
                  <>
                    <QuickModelPicker
                      toolId={id}
                      snapshot={snapshot}
                      loading={modelState.isPending}
                      error={modelState.isError}
                      disabled={disabled}
                      onRetry={() => void modelState.refetch()}
                      variant="field"
                      label={t("tray.model")}
                    />
                    {snapshot?.binding.status === "configured" && (
                      <EffortSlider
                        toolId={id}
                        snapshot={snapshot}
                        disabled={disabled}
                      />
                    )}
                    {notes.length > 0 && (
                      <p className="truncate text-note text-ink-muted">
                        {notes.join(" · ")}
                      </p>
                    )}
                  </>
                )}
              </div>
            )}
          </div>
        );
      })}
    </ListCard>
  );
}
