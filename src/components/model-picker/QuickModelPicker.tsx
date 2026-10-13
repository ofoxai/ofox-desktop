import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Loader2, Layers } from "lucide-react";
import { ModelPicker, MultiModelPicker } from "./ModelPicker";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  useModelCatalog,
  useSaveModelSelection,
} from "@/hooks/useModelSelection";
import type { ToolModelSnapshot } from "@/lib/api/modelSelection";
import { TOOL_META } from "@/config/toolMeta";

interface QuickModelPickerProps {
  snapshot?: ToolModelSnapshot;
  toolId: string;
  disabled?: boolean;
  loading?: boolean;
  error?: boolean;
  onRetry: () => void;
  compact?: boolean;
}

/** A model change writes only the selected tool, guarded by its snapshot revision. */
export function QuickModelPicker({
  snapshot,
  toolId,
  disabled = false,
  loading = false,
  error = false,
  onRetry,
  compact = false,
}: QuickModelPickerProps) {
  const { t } = useTranslation();
  const catalog = useModelCatalog(
    toolId,
    !!snapshot &&
      snapshot.binding.status === "configured" &&
      snapshot.canWrite !== false &&
      !disabled,
  );
  const save = useSaveModelSelection();
  const saveLock = useRef(false);
  const [editing, setEditing] = useState<ToolModelSnapshot | null>(null);
  const [draft, setDraft] = useState<string[]>([]);
  const [portal, setPortal] = useState<HTMLDivElement | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const multi = snapshot?.managedSelection.kind === "models";
  const writable =
    !!snapshot &&
    snapshot.binding.status === "configured" &&
    snapshot.canWrite !== false &&
    !disabled;
  const selection = snapshot?.managedSelection;
  const selectedModels = selection?.kind === "models" ? selection.models : [];
  const selectedModel = selection?.kind === "single" ? selection.model : "";
  const observed = snapshot?.observedSelection.model;
  const mixed =
    !!snapshot?.scopeModels?.length &&
    !selectedModel &&
    !snapshot.supportsDefault;

  const persist = async (
    base: ToolModelSnapshot,
    next: ToolModelSnapshot["managedSelection"],
  ) => {
    if (saveLock.current) return;
    saveLock.current = true;
    setSaveError(null);
    try {
      const report = await save.mutateAsync({
        tool: toolId,
        selection: next,
        expectedRevision: base.revision,
      });
      if (report.skipped.length) {
        toast.warning(
          t("modelExperience.skippedModels", {
            names: report.skipped
              .map((item) => `${item.id}: ${item.reason}`)
              .join("; "),
          }),
        );
      }
      toast.success(
        t(
          toolId === "chatgpt"
            ? "modelExperience.defaultSaved"
            : "modelExperience.saved",
        ),
      );
      setEditing(null);
    } catch (error) {
      const message = t("modelExperience.saveFailed", { error: String(error) });
      setSaveError(message);
      toast.error(message);
    } finally {
      saveLock.current = false;
    }
  };

  if (!snapshot || snapshot.binding.status === "unknown")
    return (
      <div
        className="text-[11px] text-muted-foreground"
        role={error ? "alert" : "status"}
      >
        {loading
          ? t("modelExperience.loading")
          : t("modelExperience.stateFailed")}
        {!loading && (
          <button type="button" className="ml-2 underline" onClick={onRetry}>
            {t("modelExperience.retry")}
          </button>
        )}
      </div>
    );

  return (
    <div
      className={compact ? "mt-1 text-[11px]" : "mt-1.5 max-w-sm text-[11px]"}
    >
      {toolId === "chatgpt" && (
        <div className="mb-1 truncate text-muted-foreground">
          {t("modelExperience.observed", {
            model: observed || t("modelExperience.unknown"),
          })}
        </div>
      )}
      {multi ? (
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!writable || save.isPending}
          onClick={() => {
            setEditing(snapshot);
            setDraft([...selectedModels]);
            setSaveError(null);
          }}
          className="h-7 max-w-full gap-1.5 text-[11px] font-normal"
        >
          <Layers className="h-3 w-3" />
          {t("modelExperience.modelCount", { count: selectedModels.length })}
        </Button>
      ) : (
        <div className="[&_button[role=combobox]]:h-7 [&_button[role=combobox]]:text-[11px]">
          {toolId === "chatgpt" && (
            <div className="mb-1 text-muted-foreground">
              {t("modelExperience.writableDefault")}
            </div>
          )}
          {mixed && (
            <div className="mb-1 text-muted-foreground">
              {t("modelExperience.mixedScopes")}
            </div>
          )}
          <ModelPicker
            id={`quick-model-${toolId}`}
            value={mixed ? t("modelExperience.mixedSelection") : selectedModel}
            onChange={(model) => {
              if (model !== selectedModel || mixed)
                void persist(snapshot, { kind: "single", model });
            }}
            models={catalog.models}
            loading={catalog.loading}
            onFetch={catalog.fetch}
            allowDefault={snapshot.supportsDefault}
            emptyLabel={
              !snapshot.supportsDefault
                ? t("modelExperience.chooseModel")
                : undefined
            }
            disabled={!writable || save.isPending}
          />
        </div>
      )}
      {snapshot.canWrite === false && (
        <p className="mt-1 text-muted-foreground">
          {t("modelCompatibility.desktopCodex")}
        </p>
      )}
      {save.isPending && (
        <div
          role="status"
          className="mt-1 flex items-center gap-1 text-muted-foreground"
        >
          <Loader2 className="h-3 w-3 animate-spin" />
          {t("modelExperience.saving")}
        </div>
      )}
      {toolId !== "chatgpt" &&
        observed &&
        observed !== selectedModel &&
        (toolId === "opencode" || !multi) && (
          <div className="mt-1 truncate text-muted-foreground">
            {t("modelExperience.observed", { model: observed })}
          </div>
        )}
      {catalog.error && (
        <div role="alert" className="mt-1 text-muted-foreground">
          {t("modelExperience.catalogFailed")}{" "}
          <button type="button" onClick={catalog.fetch} className="underline">
            {t("modelExperience.retry")}
          </button>
        </div>
      )}
      {saveError && !editing && (
        <div role="alert" className="mt-1 text-destructive">
          {saveError}
        </div>
      )}
      <Dialog
        open={!!editing}
        onOpenChange={(open) => {
          if (!open && !save.isPending) setEditing(null);
        }}
      >
        <DialogContent ref={setPortal} className="max-h-[85vh] sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>
              {t("modelExperience.editModelSet", {
                tool: TOOL_META[toolId]?.label ?? toolId,
              })}
            </DialogTitle>
            <DialogDescription>
              {t("modelExperience.multiDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 space-y-4 overflow-y-auto px-6 py-4">
            <MultiModelPicker
              id={`model-set-${toolId}`}
              selectedIds={draft}
              onChange={setDraft}
              models={catalog.models}
              loading={catalog.loading}
              onFetch={catalog.fetch}
              portalContainer={portal}
              disabled={save.isPending}
            />
            {catalog.error && (
              <p role="alert" className="text-sm text-destructive">
                {t("modelExperience.catalogFailed")}
              </p>
            )}
            {saveError && (
              <p role="alert" className="text-sm text-destructive">
                {saveError}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={save.isPending}
              onClick={() => setEditing(null)}
            >
              {t("modelExperience.cancel")}
            </Button>
            <Button
              type="button"
              disabled={
                !writable ||
                save.isPending ||
                !draft.length ||
                (toolId === "workbuddy" &&
                  (!!catalog.error ||
                    draft.some(
                      (id) => !catalog.models.some((model) => model.id === id),
                    )))
              }
              onClick={() => {
                if (editing)
                  void persist(editing, { kind: "models", models: draft });
              }}
            >
              {save.isPending
                ? t("modelExperience.saving")
                : t("modelExperience.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
