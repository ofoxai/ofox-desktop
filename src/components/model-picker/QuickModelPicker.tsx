import { useRef, useState } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Loader2, Layers, RefreshCw } from "lucide-react";
import { ModelPicker, MultiModelPicker } from "./ModelPicker";
import { MenuChevron, menuTriggerClassName } from "@/components/ui/menu-select";
import {
  useModelCatalog,
  savedMessageKey,
  useSaveModelSelection,
} from "@/hooks/useModelSelection";
import type { ToolModelSnapshot } from "@/lib/api/modelSelection";
import { cn } from "@/lib/utils";

interface QuickModelPickerProps {
  snapshot?: ToolModelSnapshot;
  toolId: string;
  disabled?: boolean;
  loading?: boolean;
  error?: boolean;
  onRetry: () => void;
  compact?: boolean;
  /**
   * "field": a tool row's model column, filled edge to edge. Every state is
   * drawn inside the chip so the row never grows; the row's sub-line carries
   * {@link modelNotes}.
   */
  variant?: "menu" | "field";
  /** A faint name inside the field before its value, as magpie's `.k`. */
  label?: string;
}

/**
 * magpie's `.field`: a filled 25px chip, ringed in brand while open. It fills
 * its column so every chevron lines up down the list.
 */
const fieldClassName =
  "h-[25px] w-full max-w-none flex-none gap-[5px] rounded-[6px] border-0 bg-pill pl-2 pr-[5px] text-nav font-medium hover:bg-pill-hover data-[state=open]:bg-pill-hover data-[state=open]:shadow-[inset_0_0_0_1px_rgb(var(--brand))]";
/** A field whose catalog or last save failed: the reason is in its title. */
const fieldWarnClassName = "shadow-[inset_0_0_0_1px_rgb(var(--warn)/0.5)]";

/**
 * What a tool row's sub-line says about its model beside the key: what the
 * tool actually runs, and why Ofox cannot set it.
 */
export function modelNotes(
  snapshot: ToolModelSnapshot | undefined,
  toolId: string,
  t: TFunction,
): string[] {
  if (!snapshot || snapshot.binding.status === "unknown") return [];
  const selection = snapshot.managedSelection;
  const observed = snapshot.observedSelection.model;
  // ChatGPT only shares Codex's default, so what it runs is always news.
  // A model set says nothing about which one ran, except OpenCode's TUI route.
  const differs =
    toolId === "chatgpt" ||
    (selection.kind === "single"
      ? observed !== selection.model
      : toolId === "opencode");
  return [
    observed && differs
      ? t("modelExperience.observedShort", { model: observed })
      : "",
    snapshot.canWrite === false ? t("modelExperience.inAppModel") : "",
  ].filter(Boolean);
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
  variant = "menu",
  label,
}: QuickModelPickerProps) {
  const field = variant === "field";
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
  // A model set is ticked in its list and saved when the list closes, against
  // the snapshot it opened on.
  const [draft, setDraft] = useState<string[] | null>(null);
  const openedOn = useRef<ToolModelSnapshot | null>(null);
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
  const mixed = !!snapshot?.scopeModels?.length && !selectedModel;

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
      toast.success(t(savedMessageKey(toolId)));
    } catch (error) {
      const message = t("modelExperience.saveFailed", { error: String(error) });
      setSaveError(message);
      toast.error(message);
    } finally {
      saveLock.current = false;
    }
  };

  if (!snapshot || snapshot.binding.status === "unknown") {
    if (field)
      return loading ? (
        <div
          role="status"
          aria-label={t("modelExperience.loading")}
          className={cn(menuTriggerClassName, fieldClassName, "animate-pulse")}
        />
      ) : (
        <button
          type="button"
          onClick={onRetry}
          title={t("modelExperience.retry")}
          className={cn(
            menuTriggerClassName,
            fieldClassName,
            "font-normal text-ink-muted",
            error && fieldWarnClassName,
          )}
        >
          <span className="min-w-0 flex-1 truncate text-left">
            {t("modelExperience.stateFailed")}
          </span>
          <RefreshCw aria-hidden className="h-3 w-3 flex-none opacity-70" />
        </button>
      );
    return (
      <div
        className="text-note text-ink-muted"
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
  }

  if (snapshot.binding.status === "paused")
    return field ? (
      <div
        title={t("modelExperience.paused")}
        className={cn(
          menuTriggerClassName,
          fieldClassName,
          "font-normal text-ink-muted",
        )}
      >
        <span className="min-w-0 flex-1 truncate text-left">
          {t("modelExperience.pausedShort")}
        </span>
      </div>
    ) : (
      <div role="status" className="text-note text-ink-muted">
        {t("modelExperience.paused")}
      </div>
    );

  const chatgptObserved = toolId === "chatgpt" && (
    <div className="mb-1 truncate text-ink-muted">
      {t("modelExperience.observed", {
        model: observed || t("modelExperience.unknown"),
      })}
    </div>
  );
  const singleNotes = (
    <>
      {toolId === "chatgpt" && (
        <div className="mb-1 text-ink-muted">
          {t("modelExperience.writableDefault")}
        </div>
      )}
      {mixed && (
        <div className="mb-1 text-ink-muted">
          {t("modelExperience.mixedScopes")}
        </div>
      )}
    </>
  );
  // The field says in its title what the menu look says in lines around it.
  const fieldTitle =
    [
      toolId === "chatgpt" && t("modelExperience.writableDefault"),
      mixed && t("modelExperience.mixedScopes"),
      catalog.error && t("modelExperience.catalogFailed"),
      saveError,
    ]
      .filter(Boolean)
      .join("\n") || undefined;
  const fieldClass =
    field &&
    cn(fieldClassName, (catalog.error || saveError) && fieldWarnClassName);
  const fieldLabel = field && label && (
    <span className="mr-1.5 flex-none font-normal text-ink-faint">{label}</span>
  );
  const fieldTrailing = save.isPending ? (
    <Loader2
      aria-hidden
      className="h-3 w-3 flex-none animate-spin opacity-70"
    />
  ) : undefined;
  const shownModels = draft ?? selectedModels;
  const finishSet = () => {
    const next = draft;
    const base = openedOn.current;
    setDraft(null);
    openedOn.current = null;
    if (!next || !base || base.managedSelection.kind !== "models") return;
    const before = base.managedSelection.models;
    if (
      next.length === before.length &&
      next.every((id, i) => id === before[i])
    )
      return;
    if (!next.length) {
      toast.warning(t("modelExperience.chooseModels"));
      return;
    }
    if (
      toolId === "workbuddy" &&
      (!!catalog.error ||
        next.some((id) => !catalog.models.some((model) => model.id === id)))
    ) {
      toast.error(t("modelExperience.missingCatalogModels"));
      return;
    }
    void persist(base, { kind: "models", models: next });
  };
  const picker = multi ? (
    <MultiModelPicker
      id={`model-set-${toolId}`}
      selectedIds={shownModels}
      onChange={setDraft}
      models={catalog.models}
      loading={catalog.loading}
      onFetch={catalog.fetch}
      disabled={!writable || save.isPending}
      onOpenChange={(open) => {
        if (open) {
          openedOn.current = snapshot;
          setDraft([...selectedModels]);
          setSaveError(null);
        } else finishSet();
      }}
      field={{
        label: t("modelExperience.modelCount", { count: shownModels.length }),
        className: cn("max-w-full", !field && "w-auto flex-none", fieldClass),
        content: (
          <>
            {fieldLabel}
            <Layers aria-hidden className="h-[13px] w-[13px] flex-none" />
            <span
              className={cn("min-w-0 truncate", field && "flex-1 text-left")}
            >
              {t("modelExperience.modelCount", { count: shownModels.length })}
            </span>
            {field ? (fieldTrailing ?? <MenuChevron />) : <MenuChevron />}
          </>
        ),
      }}
    />
  ) : (
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
      disabled={!writable || save.isPending}
      triggerClassName={fieldClass || undefined}
      className={field ? "w-full" : undefined}
      trailing={field ? fieldTrailing : undefined}
      prefix={fieldLabel}
      title={field ? fieldTitle : undefined}
      busy={field && save.isPending}
    />
  );

  if (field)
    return (
      <div className="min-w-0">
        {picker}
        {(saveError || catalog.error) && (
          <span role="alert" className="sr-only">
            {saveError ?? t("modelExperience.catalogFailed")}
          </span>
        )}
      </div>
    );

  return (
    <div className={compact ? "mt-1 text-note" : "mt-1.5 max-w-sm text-note"}>
      {chatgptObserved}
      {multi ? (
        picker
      ) : (
        <div>
          {singleNotes}
          {picker}
        </div>
      )}
      {snapshot.canWrite === false && (
        <p className="mt-1 text-ink-muted">
          {t("modelCompatibility.desktopCodex")}
        </p>
      )}
      {save.isPending && (
        <div
          role="status"
          className="mt-1 flex items-center gap-1 text-ink-muted"
        >
          <Loader2 className="h-3 w-3 animate-spin" />
          {t("modelExperience.saving")}
        </div>
      )}
      {toolId !== "chatgpt" &&
        observed &&
        observed !== selectedModel &&
        (toolId === "opencode" || !multi) && (
          <div className="mt-1 truncate text-ink-muted">
            {t("modelExperience.observed", { model: observed })}
          </div>
        )}
      {catalog.error && (
        <div role="alert" className="mt-1 text-ink-muted">
          {t("modelExperience.catalogFailed")}{" "}
          <button type="button" onClick={catalog.fetch} className="underline">
            {t("modelExperience.retry")}
          </button>
        </div>
      )}
      {saveError && (
        <div role="alert" className="mt-1 text-danger">
          {saveError}
        </div>
      )}
    </div>
  );
}
