import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import {
  Loader2,
  RefreshCw,
  Check,
  ChevronsUpDown,
  Folder,
  Zap,
  CheckCircle2,
  XCircle,
  Github,
  ExternalLink,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { cn } from "@/lib/utils";
import { settingsApi } from "@/lib/api";
import {
  manageToolApi,
  managedToolId,
  TOOL_PROTOCOL,
  type PingResult,
  type CompatibilityProtocol,
  type CompatibilityResult,
  type WorkBuddyEndpointStatus,
} from "@/lib/api/manageTool";
import {
  fetchOfoxModels,
  filterOfoxModelsByProtocol,
  filterOfoxModelsForWorkBuddy,
  pickWorkBuddyCuratedModels,
  toWorkBuddyModelSelection,
  type FetchedModel,
} from "@/lib/api/model-fetch";
import { readBoundTools, unbindTool } from "@/lib/bindTools";
import { ofoxBindApi, type UnbindReport } from "@/lib/api/ofoxBind";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { TOOL_META } from "@/config/toolMeta";
import type { AppId } from "@/lib/api/types";

/** Subset of `ConsolePage`'s `BoundTool` that the dialog actually needs.
 *  Kept structural so we can call it with the full BoundTool object. */
export interface ManageToolTarget {
  id: string;
  abbr: string;
  label: string;
  color: string;
  version: string | null;
  installationKind?: "desktopApp" | "cli";
}

interface ManageToolDialogProps {
  /** Non-null = dialog is open and managing this tool. Null closes. */
  tool: ManageToolTarget | null;
  onOpenChange: (open: boolean) => void;
  /** Fired after a successful save or successful unbind so ConsolePage can
   *  refresh its list (status pill, monthly tokens, presence after unbind). */
  onChanged?: () => void;
}

function toolLabels(ids: string[]): string {
  return ids.map((id) => TOOL_META[id]?.label ?? id).join(", ");
}

/** One-line summary of an unbind result for the toast; paths/counts only. */
function describeUnbindResult(
  report: UnbindReport | null,
  t: TFunction,
): string | undefined {
  if (!report) return undefined;
  const parts: string[] = [];
  const changed = report.restoredKeys.length + report.removedKeys.length;
  if (report.sharedKeptBy.length > 0) {
    parts.push(
      t("unbind.resultShared", { others: toolLabels(report.sharedKeptBy) }),
    );
  } else if (report.legacy) {
    parts.push(t("unbind.resultLegacy"));
  } else if (report.alreadyUnbound) {
    parts.push(t("unbind.nothingToRestore"));
  } else if (report.exactFiles.length > 0) {
    parts.push(t("unbind.resultExact"));
  } else if (changed > 0) {
    parts.push(t("unbind.resultRestored", { count: changed }));
  }
  for (const warning of report.warnings) {
    parts.push(t(`unbind.warning.${warning.code}`, { defaultValue: "" }));
  }
  const text = parts.filter(Boolean).join(" ");
  return text || undefined;
}

function arraysEqual(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

/**
 * Manage-bound-tool dialog launched from each row's "管理" button.
 *
 * Scope: per-tool model selection + path display + unbind. We deliberately
 * do NOT expose a provider switcher or API key field — when a tool is
 * "bound" in the OfoxAI sense, the active provider, base URL, and token
 * are managed by `bindTools()` / `ofox_bind_tool` and aren't user-editable
 * here. The model is the one knob users actually need.
 */
export default function ManageToolDialog({
  tool,
  onOpenChange,
  onChanged,
}: ManageToolDialogProps) {
  const { t } = useTranslation();
  const open = !!tool;
  const protocol = tool ? TOOL_PROTOCOL[tool.id] : undefined;
  const desktopCodex =
    tool?.id === "codex" && tool.installationKind === "desktopApp";
  const projectUrl = tool ? TOOL_META[tool.id]?.projectUrl : undefined;
  const projectLinkLabel = tool
    ? TOOL_META[tool.id]?.projectLinkLabel
    : undefined;

  // Anchors the model-picker's Popover portal inside the dialog so its
  // CommandList stays scrollable. Radix Dialog wraps its content in
  // `react-remove-scroll` with the content node as the only "scroll shard";
  // any popover portaled to body falls outside the shard and has its wheel
  // events swallowed. Pointing the popover's `container` at this ref puts it
  // back inside the shard.
  // A ref's `.current` does not trigger a render. Keep the portal target in
  // state so the first open cannot accidentally portal to `document.body`.
  const [dialogContentNode, setDialogContentNode] =
    useState<HTMLDivElement | null>(null);

  // --- per-tool state ------------------------------------------------------
  // Reset every time we switch tools so the dialog never shows stale data
  // from a previous open.
  const [filePath, setFilePath] = useState<string | null>(null);
  const [filePathError, setFilePathError] = useState<string | null>(null);
  const [currentModel, setCurrentModel] = useState<string>("");
  const [draftModel, setDraftModel] = useState<string>("");
  const [currentModels, setCurrentModels] = useState<string[]>([]);
  const [draftModels, setDraftModels] = useState<string[]>([]);
  // Re-saving an unchanged model is allowed only after the current value was
  // actually read — otherwise an empty draft would wipe the configured model.
  const [currentModelLoaded, setCurrentModelLoaded] = useState(false);
  const [models, setModels] = useState<FetchedModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [workBuddyCatalogError, setWorkBuddyCatalogError] = useState(false);
  const [workBuddyEndpointStatus, setWorkBuddyEndpointStatus] =
    useState<WorkBuddyEndpointStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [unbindConfirming, setUnbindConfirming] = useState(false);
  const [unbindLoading, setUnbindLoading] = useState(false);
  // What the unbind would restore; null until loaded (or if the preview failed).
  const [unbindPreview, setUnbindPreview] = useState<UnbindReport | null>(null);
  // Connectivity probe — null means "未测试". The backend always resolves
  // with a PingResult, so we never put an exception here.
  const [pingResult, setPingResult] = useState<PingResult | null>(null);
  const [pingLoading, setPingLoading] = useState(false);
  const [compatibilityResults, setCompatibilityResults] = useState<
    CompatibilityResult[]
  >([]);
  const [compatibilityLoading, setCompatibilityLoading] = useState(false);
  const [manualProtocol, setManualProtocol] = useState<
    CompatibilityProtocol | ""
  >("");
  const [allowUnverified, setAllowUnverified] = useState(false);
  const compatibilityRequest = useRef(0);
  const [workBuddyTestModel, setWorkBuddyTestModel] = useState("");
  const [workBuddyTestResult, setWorkBuddyTestResult] =
    useState<CompatibilityResult | null>(null);
  const [workBuddyTestLoading, setWorkBuddyTestLoading] = useState(false);
  const workBuddyTestRequest = useRef(0);

  // Today's stats (per-tool, midnight-local-time → now). null = 还没加载完。
  // 「今日统计」整段（含 today-stats fetch、loading state、cell renderer）已在
  // 用户要求下移除 —— ConsolePage 顶部 4 张卡 + 列表中「今日数据」列已经覆盖
  // 这部分信息，dialog 里再展示一遍只是冗余。

  // Initial load when the dialog opens for a given tool.
  useEffect(() => {
    if (!tool) return;
    let cancelled = false;

    setFilePath(null);
    setFilePathError(null);
    setCurrentModel("");
    setDraftModel("");
    setCurrentModels([]);
    setDraftModels([]);
    setCurrentModelLoaded(false);
    setModels([]);
    setModelsLoading(tool.id === "workbuddy");
    setWorkBuddyCatalogError(false);
    setWorkBuddyEndpointStatus(null);
    setSaving(false);
    setUnbindConfirming(false);
    setUnbindLoading(false);
    setUnbindPreview(null);
    setPingResult(null);
    setPingLoading(false);
    setCompatibilityResults([]);
    setCompatibilityLoading(false);
    setManualProtocol("");
    setAllowUnverified(false);
    compatibilityRequest.current += 1;
    setWorkBuddyTestModel("");
    setWorkBuddyTestResult(null);
    setWorkBuddyTestLoading(false);
    workBuddyTestRequest.current += 1;

    const loadPath = async () => {
      try {
        const path = await manageToolApi.getConfigFilePath(tool.id);
        if (!cancelled) setFilePath(path);
      } catch (e) {
        if (!cancelled) setFilePathError(String(e));
      }
    };
    const loadModels = async () => {
      try {
        if (tool.id === "workbuddy") {
          const [managedResult, catalogResult, endpointResult] =
            await Promise.allSettled([
              manageToolApi.getWorkBuddyManagedModels(),
              fetchOfoxModels("openai"),
              manageToolApi.getWorkBuddyEndpointStatus(),
            ]);
          if (!cancelled) {
            if (managedResult.status === "fulfilled") {
              setCurrentModels(managedResult.value);
              setDraftModels(managedResult.value);
              setCurrentModelLoaded(true);
            }
            if (catalogResult.status === "fulfilled") {
              setModels(filterOfoxModelsForWorkBuddy(catalogResult.value));
              setWorkBuddyCatalogError(false);
            } else {
              setWorkBuddyCatalogError(true);
            }
            if (endpointResult.status === "fulfilled") {
              setWorkBuddyEndpointStatus(endpointResult.value);
            }
            setModelsLoading(false);
          }
          if (managedResult.status === "rejected") throw managedResult.reason;
          if (catalogResult.status === "rejected") throw catalogResult.reason;
          return;
        }
        const m = await manageToolApi.getActiveModel(tool.id);
        if (!cancelled) {
          setCurrentModel(m);
          setDraftModel(m);
          setCurrentModelLoaded(true);
        }
      } catch (e) {
        // Soft-fail: an empty model just means "OfoxAI default routing",
        // which is a valid state. Don't block the dialog over it.
        console.warn(`[ManageToolDialog] getActiveModel(${tool.id}) failed`, e);
        if (!cancelled && tool.id === "workbuddy") setModelsLoading(false);
      }
    };
    void Promise.all([loadPath(), loadModels()]);

    return () => {
      cancelled = true;
    };
  }, [tool]);

  // Lazy model fetch for CLI tools. WorkBuddy preloads because its current
  // value is a set that must be reconciled with the compatible catalog.
  //
  // Codex CLI 强绑 responses 协议（codex_config.rs 里 wire_api="responses"
  // 是硬编码），选到只支持 chat/completions 的模型会导致 CLI 报
  // `wire_api not supported`——按端点二次过滤，UI 层就不给用户选到
  // 不兼容的模型。其他固定 Chat 客户端也按端点过滤。
  const requiredEndpoint =
    tool?.id === "codex" || tool?.id === "chatgpt"
      ? "/v1/responses"
      : tool?.id === "openclaw" || tool?.id === "hermes"
        ? "/v1/chat/completions"
        : undefined;
  const fetchModels = useCallback(
    async (forceRefresh = false) => {
      if (!protocol) return;
      setModelsLoading(true);
      try {
        const all = await fetchOfoxModels(protocol, forceRefresh);
        setModels(
          tool?.id === "workbuddy"
            ? filterOfoxModelsForWorkBuddy(all)
            : filterOfoxModelsByProtocol(all, protocol, requiredEndpoint),
        );
        if (tool?.id === "workbuddy") setWorkBuddyCatalogError(false);
      } catch (e) {
        console.error("[ManageToolDialog] fetchOfoxModels failed", e);
        if (tool?.id === "workbuddy") setWorkBuddyCatalogError(true);
        toast.error(`获取模型列表失败：${String(e)}`);
      } finally {
        setModelsLoading(false);
      }
    },
    [protocol, requiredEndpoint, tool?.id],
  );

  const runCompatibility = useCallback(
    async (forceRetest: boolean) => {
      const request = ++compatibilityRequest.current;
      setCompatibilityResults([]);
      setManualProtocol("");
      setAllowUnverified(false);
      const ids =
        tool?.id === "opencode" && draftModel
          ? [draftModel]
          : draftModel && draftModel !== currentModel && !desktopCodex
            ? [draftModel]
            : [];
      if (!tool || ids.length === 0) {
        setCompatibilityLoading(false);
        return;
      }
      setCompatibilityLoading(true);
      const results: CompatibilityResult[] = [];
      for (const id of ids) {
        try {
          results.push(
            await manageToolApi.checkCompatibility(tool.id, id, forceRetest),
          );
        } catch {
          results.push({
            app: tool.id,
            model: id,
            protocol: null,
            status: "inconclusive",
            source: "probe",
            reason: t("modelCompatibility.requestFailed"),
          });
        }
        if (request !== compatibilityRequest.current) return;
        setCompatibilityResults([...results]);
      }
      if (request === compatibilityRequest.current)
        setCompatibilityLoading(false);
    },
    [tool?.id, draftModel, currentModel, desktopCodex, t],
  );

  useEffect(() => {
    if (tool?.id === "workbuddy") return;
    void runCompatibility(false);
    return () => {
      compatibilityRequest.current += 1;
    };
  }, [runCompatibility, tool?.id]);

  useEffect(() => {
    if (tool?.id !== "workbuddy" || draftModels.includes(workBuddyTestModel))
      return;
    workBuddyTestRequest.current += 1;
    setWorkBuddyTestModel(draftModels[0] ?? "");
    setWorkBuddyTestResult(null);
    setWorkBuddyTestLoading(false);
  }, [draftModels, tool?.id, workBuddyTestModel]);

  const handleWorkBuddyTest = useCallback(async () => {
    if (tool?.id !== "workbuddy" || !workBuddyTestModel) return;
    const request = ++workBuddyTestRequest.current;
    setWorkBuddyTestLoading(true);
    setWorkBuddyTestResult(null);
    try {
      const result = await manageToolApi.checkCompatibility(
        "workbuddy",
        workBuddyTestModel,
        true,
      );
      if (request === workBuddyTestRequest.current)
        setWorkBuddyTestResult(result);
    } catch {
      if (request === workBuddyTestRequest.current) {
        setWorkBuddyTestResult({
          app: "workbuddy",
          model: workBuddyTestModel,
          protocol: null,
          status: "inconclusive",
          source: "probe",
          reason: t("modelCompatibility.requestFailed"),
        });
      }
    } finally {
      if (request === workBuddyTestRequest.current)
        setWorkBuddyTestLoading(false);
    }
  }, [tool?.id, workBuddyTestModel, t]);

  const handleOpenFolder = useCallback(async () => {
    if (!tool) return;
    try {
      await settingsApi.openConfigFolder(managedToolId(tool.id) as AppId);
    } catch (e) {
      toast.error(`打开配置目录失败：${String(e)}`);
    }
  }, [tool]);

  const handleOpenProject = useCallback(async () => {
    if (!projectUrl) return;
    try {
      await settingsApi.openExternal(projectUrl);
    } catch (e) {
      toast.error(`打开项目链接失败：${String(e)}`);
    }
  }, [projectUrl]);

  // A selected OpenCode model may keep the same ID while its transport needs
  // to change (for example GLM Responses -> Chat). Allow applying the probe
  // result without forcing the user to pick another model first.
  const isDirty =
    tool?.id === "workbuddy"
      ? !arraysEqual(draftModels, currentModels)
      : draftModel !== currentModel ||
        (tool?.id === "opencode" &&
          !!draftModel &&
          compatibilityResults[0]?.model === draftModel);

  const handleSave = useCallback(async () => {
    if (!tool) return;
    const workBuddy = tool.id === "workbuddy";
    if (!isDirty && !currentModelLoaded) return;
    setSaving(true);
    try {
      if (workBuddy) {
        if (draftModels.length === 0) {
          throw new Error("请至少选择一个 WorkBuddy 模型");
        }
        const selections = draftModels.map((id) =>
          models.find((model) => model.id === id),
        );
        if (selections.some((model) => !model)) {
          throw new Error("部分所选模型已不在兼容列表中，请刷新后重试");
        }
        await manageToolApi.setWorkBuddyManagedModels(
          selections.map((model) => toWorkBuddyModelSelection(model!)),
        );
        setCurrentModels(draftModels);
      } else {
        const selectedProtocol =
          compatibilityResults[0]?.status === "compatible"
            ? (compatibilityResults[0].protocol ?? undefined)
            : manualProtocol || undefined;
        await manageToolApi.setActiveModel(
          tool.id,
          draftModel,
          undefined,
          selectedProtocol,
          allowUnverified,
        );
        setCurrentModel(draftModel);
      }
      toast.success(
        workBuddy ? `已同步 ${draftModels.length} 个模型` : "模型已更新",
      );
      onChanged?.();
      // Close on success — mirrors handleUnbind's收尾 sequence and
      // matches the pattern users expect from a save action. Failures
      // keep the dialog open so the user can fix and retry.
      onOpenChange(false);
    } catch (e) {
      toast.error(`保存失败：${String(e)}`);
    } finally {
      setSaving(false);
    }
  }, [
    tool,
    draftModel,
    currentModel,
    isDirty,
    currentModelLoaded,
    draftModels,
    models,
    compatibilityResults,
    manualProtocol,
    allowUnverified,
    onChanged,
    onOpenChange,
  ]);

  const probeModel = draftModel;

  /**
   * Probe the OfoxAI gateway end-to-end with the currently-drafted model.
   *
   * Pings *draft*, not saved — so the user can vet a model choice before
   * committing it (the bound provider's settings_config is still pointing
   * at whatever was saved last). The backend never rejects on HTTP errors;
   * it folds them into `PingResult.error`, which we render verbatim.
   *
   * Each click runs a fresh request — no caching. Latency varies enough
   * (cold starts, model warm-up) that a stale "✓ 1.2s" is misleading.
   */
  const handlePing = useCallback(async () => {
    if (!tool) return;
    if (!probeModel) {
      toast.error("请先选择一个模型再测试连通性");
      return;
    }
    setPingLoading(true);
    setPingResult(null);
    try {
      const result = await manageToolApi.pingModel(tool.id, probeModel);
      setPingResult(result);
    } catch (e) {
      // Tauri-level failure (invalid app, IPC error, …) — wrap into the same
      // shape so the UI doesn't have to handle two failure paths.
      setPingResult({
        success: false,
        latencyMs: 0,
        statusCode: null,
        error: String(e),
      });
    } finally {
      setPingLoading(false);
    }
  }, [tool, probeModel]);

  const handleUnbind = useCallback(async () => {
    if (!tool) return;
    if (!unbindConfirming) {
      // First click → arm the confirm state and show what will be restored
      // before the second click commits.
      setUnbindConfirming(true);
      try {
        const stillBound = readBoundTools().filter((id) => id !== tool.id);
        setUnbindPreview(await ofoxBindApi.unbindPreview(tool.id, stillBound));
      } catch (e) {
        // A failed preview must not block unbinding; the panel falls back to
        // the generic description.
        console.warn("[ManageToolDialog] unbind preview failed", e);
        setUnbindPreview(null);
      }
      return;
    }
    setUnbindLoading(true);
    try {
      const report = await unbindTool(tool.id);
      const message = t("unbind.success", { tool: tool.label });
      const description = describeUnbindResult(report, t);
      if (report?.legacy) {
        toast.warning(message, { description, duration: 10_000 });
      } else {
        toast.success(message, { description });
      }
      onChanged?.();
      onOpenChange(false);
    } catch (e) {
      toast.error(t("unbind.failed", { error: String(e) }));
      setUnbindConfirming(false);
    } finally {
      setUnbindLoading(false);
    }
  }, [tool, unbindConfirming, onChanged, onOpenChange, t]);

  const hasIncompatible = compatibilityResults.some(
    (result) => result.status === "incompatible",
  );
  const hasInconclusive = compatibilityResults.some(
    (result) => result.status === "inconclusive",
  );
  const compatibilityReady =
    tool?.id === "workbuddy" ||
    (!compatibilityLoading &&
      !hasIncompatible &&
      (!hasInconclusive ||
        (allowUnverified && (tool?.id !== "opencode" || !!manualProtocol))) &&
      (!draftModel ||
        draftModel === currentModel ||
        compatibilityResults[0]?.model === draftModel));
  const catalogModelIds = new Set(models.map((model) => model.id));
  const workBuddyMissingIds = draftModels.filter(
    (id) => !catalogModelIds.has(id),
  );
  const workBuddyCatalogReady =
    !modelsLoading &&
    !workBuddyCatalogError &&
    draftModels.length > 0 &&
    workBuddyMissingIds.length === 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        ref={setDialogContentNode}
        className="max-w-lg gap-0 p-0"
        aria-describedby={undefined}
      >
        {tool && (
          <>
            <DialogHeader className="flex-row items-center gap-3 space-y-0">
              <ToolBadge toolId={tool.id} size={40} rounded="xl" />
              <div className="min-w-0 flex-1 text-left">
                <DialogTitle className="text-[15px] font-semibold text-foreground">
                  {tool.label}
                </DialogTitle>
                <div className="truncate text-[12px] text-muted-foreground">
                  {tool.version
                    ? `${TOOL_META[tool.id]?.launchKind === "desktopApp" ? "桌面应用 · " : ""}v${tool.version}`
                    : "未检测到"}
                </div>
              </div>
              {projectUrl && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => void handleOpenProject()}
                  aria-label={`查看 ${tool.label} ${projectLinkLabel}`}
                  title={`查看 ${tool.label} ${projectLinkLabel}`}
                  className="shrink-0 text-muted-foreground"
                >
                  {projectLinkLabel === "GitHub" ? (
                    <Github className="mr-1.5 h-4 w-4" />
                  ) : (
                    <ExternalLink className="mr-1.5 h-4 w-4" />
                  )}
                  {projectLinkLabel}
                </Button>
              )}
            </DialogHeader>

            <div className="flex-1 overflow-y-auto px-6 py-5 space-y-5">
              {/* ---- Model ---- */}
              <div className="space-y-2">
                <Label htmlFor="manage-model" className="text-[13px]">
                  模型
                </Label>
                {tool.id === "chatgpt" && (
                  <p className="text-[11px] text-muted-foreground">
                    {t("modelCompatibility.chatgptCodexShared")}
                  </p>
                )}
                {protocol && !desktopCodex && tool.id === "workbuddy" ? (
                  <>
                    <WorkBuddyModelPicker
                      id="manage-model"
                      selectedIds={draftModels}
                      onChange={setDraftModels}
                      models={models}
                      loading={modelsLoading}
                      onFetch={fetchModels}
                      portalContainer={dialogContentNode}
                    />
                    <p className="text-[11px] text-muted-foreground">
                      {t("modelCompatibility.workBuddyProtocol")}
                    </p>
                    {workBuddyEndpointStatus && (
                      <div className="space-y-1 rounded-md border border-border-default bg-muted/20 p-3 text-[11px]">
                        <p className="font-medium">
                          {t("modelCompatibility.workBuddyConfiguredEndpoint")}
                        </p>
                        {workBuddyEndpointStatus.configuredUrls.length > 0 ? (
                          workBuddyEndpointStatus.configuredUrls.map((url) => (
                            <code
                              key={url}
                              className="block break-all"
                              title={url}
                            >
                              {url}
                            </code>
                          ))
                        ) : (
                          <p className="text-muted-foreground">—</p>
                        )}
                        <p className="break-all text-muted-foreground">
                          {t("modelCompatibility.workBuddyExpectedEndpoint")}:{" "}
                          {workBuddyEndpointStatus.expectedUrl}
                        </p>
                        {(workBuddyEndpointStatus.externallyModified ||
                          workBuddyEndpointStatus.configuredUrls.some(
                            (url) =>
                              url !== workBuddyEndpointStatus.expectedUrl,
                          )) && (
                          <p className="text-red-600">
                            {t("modelCompatibility.workBuddyEndpointMismatch")}
                          </p>
                        )}
                        <p className="text-muted-foreground">
                          {t("modelCompatibility.workBuddyEndpointCheckScope")}
                        </p>
                      </div>
                    )}
                    {!modelsLoading && workBuddyCatalogError && (
                      <p className="text-[11px] text-red-600">
                        {t("modelCompatibility.workBuddyCatalogFailed")}
                      </p>
                    )}
                    {!modelsLoading &&
                      !workBuddyCatalogError &&
                      workBuddyMissingIds.length > 0 && (
                        <p className="text-[11px] text-red-600">
                          {t("modelCompatibility.workBuddyMissingCatalog")}
                        </p>
                      )}
                  </>
                ) : protocol && !desktopCodex ? (
                  <ModelPicker
                    id="manage-model"
                    value={draftModel}
                    onChange={setDraftModel}
                    models={models}
                    loading={modelsLoading}
                    onFetch={fetchModels}
                    portalContainer={dialogContentNode}
                  />
                ) : (
                  <div className="rounded-md border border-dashed border-border-default px-3 py-2 text-[12px] text-muted-foreground">
                    {desktopCodex
                      ? t("modelCompatibility.desktopCodex")
                      : "该工具暂不支持模型管理"}
                  </div>
                )}
                {isDirty && !desktopCodex && tool.id !== "workbuddy" && (
                  <div className="space-y-2 rounded-md border border-border-default bg-muted/20 p-3 text-[12px]">
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-medium">
                        {compatibilityLoading
                          ? t("modelCompatibility.checking")
                          : t("modelCompatibility.title")}
                      </span>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        disabled={compatibilityLoading}
                        onClick={() => void runCompatibility(true)}
                      >
                        <RefreshCw className="mr-1 h-3.5 w-3.5" />
                        {t("modelCompatibility.retest")}
                      </Button>
                    </div>
                    {compatibilityResults.map((result) => (
                      <p key={result.model} className="text-muted-foreground">
                        {result.model}:{" "}
                        {t(`modelCompatibility.${result.status}`)}
                        {result.protocol ? ` · ${result.protocol}` : ""}
                        {result.reason ? ` · ${result.reason}` : ""}
                      </p>
                    ))}
                    {hasInconclusive && (
                      <div className="space-y-2">
                        {tool.id === "opencode" && (
                          <Select
                            value={manualProtocol}
                            onValueChange={(value) =>
                              setManualProtocol(value as CompatibilityProtocol)
                            }
                          >
                            <SelectTrigger
                              aria-label={t(
                                "modelCompatibility.chooseProtocol",
                              )}
                              className="h-9 text-[12px]"
                            >
                              <SelectValue
                                placeholder={t(
                                  "modelCompatibility.chooseProtocol",
                                )}
                              />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem
                                value="responses"
                                disabled={
                                  !!compatibilityResults[0]?.allowedProtocols
                                    ?.length &&
                                  !compatibilityResults[0].allowedProtocols.includes(
                                    "responses",
                                  )
                                }
                              >
                                Responses
                              </SelectItem>
                              <SelectItem
                                value="chatCompletions"
                                disabled={
                                  !!compatibilityResults[0]?.allowedProtocols
                                    ?.length &&
                                  !compatibilityResults[0].allowedProtocols.includes(
                                    "chatCompletions",
                                  )
                                }
                              >
                                Chat Completions
                              </SelectItem>
                            </SelectContent>
                          </Select>
                        )}
                        <label className="flex items-center gap-2">
                          <input
                            type="checkbox"
                            checked={allowUnverified}
                            onChange={(event) =>
                              setAllowUnverified(event.target.checked)
                            }
                          />
                          {t("modelCompatibility.continueUnverified")}
                        </label>
                      </div>
                    )}
                  </div>
                )}
              </div>

              {/* ---- Config file ---- */}
              <div className="space-y-2">
                <Label className="text-[13px]">配置文件</Label>
                <div className="flex items-center gap-2">
                  <code
                    className="flex-1 truncate rounded-md border border-border-default bg-muted/30 px-3 py-2 text-[12px] text-foreground"
                    title={filePath ?? filePathError ?? ""}
                  >
                    {filePath ?? filePathError ?? "加载中…"}
                  </code>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleOpenFolder}
                  >
                    <Folder className="mr-1 h-3.5 w-3.5" />在 Finder 打开
                  </Button>
                </div>
              </div>

              {/* WorkBuddy checks one chosen stream on demand. Other tools
                  keep their separate, optional 1-token connectivity ping. */}
              {tool.id === "workbuddy" ? (
                <div className="space-y-2">
                  <Label htmlFor="workbuddy-test-model" className="text-[13px]">
                    {t("modelCompatibility.title")}
                  </Label>
                  <div className="flex gap-2">
                    <Select
                      value={workBuddyTestModel}
                      onValueChange={(value) => {
                        workBuddyTestRequest.current += 1;
                        setWorkBuddyTestModel(value);
                        setWorkBuddyTestResult(null);
                        setWorkBuddyTestLoading(false);
                      }}
                      disabled={draftModels.length === 0}
                    >
                      <SelectTrigger
                        id="workbuddy-test-model"
                        aria-label={t("modelCompatibility.workBuddyTestModel")}
                        className="min-w-0 flex-1 text-[12px]"
                      >
                        <SelectValue placeholder="—" />
                      </SelectTrigger>
                      <SelectContent className="max-h-60">
                        {draftModels.map((id) => (
                          <SelectItem
                            key={id}
                            value={id}
                            className="text-[12px]"
                          >
                            {id}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <Button
                      type="button"
                      size="sm"
                      onClick={() => void handleWorkBuddyTest()}
                      disabled={!workBuddyTestModel || workBuddyTestLoading}
                    >
                      {workBuddyTestLoading && (
                        <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                      )}
                      {workBuddyTestLoading
                        ? t("modelCompatibility.checking")
                        : t("modelCompatibility.workBuddyTest")}
                    </Button>
                  </div>
                  <p className="text-[11px] text-muted-foreground">
                    {t("modelCompatibility.workBuddySaveHint")}
                  </p>
                  {workBuddyTestResult && (
                    <div
                      className={cn(
                        "rounded-md border px-3 py-2 text-[12px]",
                        workBuddyTestResult.status === "compatible"
                          ? "border-emerald-500/30 text-emerald-700"
                          : workBuddyTestResult.status === "incompatible"
                            ? "border-red-500/30 text-red-700"
                            : "border-border-default text-muted-foreground",
                      )}
                    >
                      {workBuddyTestResult.model}:{" "}
                      {workBuddyTestResult.status === "incompatible"
                        ? t("modelCompatibility.workBuddyIncompatible")
                        : t(`modelCompatibility.${workBuddyTestResult.status}`)}
                      {workBuddyTestResult.reason &&
                        ` · ${workBuddyTestResult.reason}`}
                    </div>
                  )}
                </div>
              ) : (
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-[13px]">连通性</Label>
                    <Button
                      type="button"
                      size="sm"
                      onClick={handlePing}
                      disabled={pingLoading || !protocol || !probeModel}
                      className="bg-orange-500 text-white shadow-sm shadow-orange-200/60 hover:bg-orange-600 disabled:bg-orange-500/60 disabled:text-white dark:shadow-orange-900/20"
                    >
                      {pingLoading ? (
                        <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Zap className="mr-1.5 h-3.5 w-3.5" />
                      )}
                      {pingLoading ? "测试中…" : "测试连通性"}
                    </Button>
                  </div>
                  <PingStatusBox
                    loading={pingLoading}
                    result={pingResult}
                    model={probeModel}
                  />
                </div>
              )}
            </div>

            {unbindConfirming && <UnbindPreviewPanel preview={unbindPreview} />}

            <DialogFooter className="!items-center sm:!justify-between">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleUnbind}
                disabled={unbindLoading}
                className={cn(
                  unbindConfirming &&
                    "border-red-500 text-red-600 hover:bg-red-50 hover:text-red-700",
                )}
              >
                {unbindLoading && (
                  <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                )}
                {unbindConfirming ? t("unbind.confirm") : t("unbind.button")}
              </Button>
              <div className="flex gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => onOpenChange(false)}
                  disabled={saving}
                >
                  取消
                </Button>
                <Button
                  type="button"
                  size="sm"
                  onClick={handleSave}
                  disabled={
                    (!isDirty && !currentModelLoaded) ||
                    saving ||
                    !protocol ||
                    !compatibilityReady ||
                    desktopCodex ||
                    (tool.id === "workbuddy" && !workBuddyCatalogReady)
                  }
                >
                  {saving && (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  )}
                  {tool.id === "opencode" &&
                  draftModel === currentModel &&
                  isDirty
                    ? t("modelCompatibility.applyProtocol")
                    : "保存"}
                </Button>
              </div>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// WorkBuddyModelPicker — searchable multi-select with curated/all shortcuts
// ---------------------------------------------------------------------------

// cmdk becomes noticeably slow when every catalog entry is mounted at once.
// Search the full catalog, but render it in small batches as the user scrolls.
const MODEL_PICKER_BATCH_SIZE = 80;

function filterPickerModels(
  models: FetchedModel[],
  query: string,
): FetchedModel[] {
  const search = query.trim().toLocaleLowerCase();
  if (!search) return models;
  return models.filter((model) =>
    `${model.id} ${model.name ?? ""} ${model.ownedBy ?? ""}`
      .toLocaleLowerCase()
      .includes(search),
  );
}

function groupPickerModels(
  models: FetchedModel[],
): Record<string, FetchedModel[]> {
  const grouped: Record<string, FetchedModel[]> = {};
  for (const model of models) {
    const slash = model.id.indexOf("/");
    const vendor =
      slash > 0 ? model.id.slice(0, slash) : model.ownedBy || "Other";
    (grouped[vendor] ||= []).push(model);
  }
  return grouped;
}

function usePickerResults(models: FetchedModel[]) {
  const [search, setSearch] = useState("");
  const [visibleCount, setVisibleCount] = useState(MODEL_PICKER_BATCH_SIZE);
  const filtered = useMemo(
    () => filterPickerModels(models, search),
    [models, search],
  );
  const visible = useMemo(
    () => filtered.slice(0, visibleCount),
    [filtered, visibleCount],
  );
  const grouped = useMemo(() => groupPickerModels(visible), [visible]);
  const vendors = useMemo(() => Object.keys(grouped).sort(), [grouped]);

  const updateSearch = useCallback((value: string) => {
    setSearch(value);
    setVisibleCount(MODEL_PICKER_BATCH_SIZE);
  }, []);
  const loadMore = useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      const list = event.currentTarget;
      if (
        visibleCount < filtered.length &&
        list.scrollTop + list.clientHeight >= list.scrollHeight - 48
      ) {
        setVisibleCount((count) => count + MODEL_PICKER_BATCH_SIZE);
      }
    },
    [filtered.length, visibleCount],
  );

  return { search, updateSearch, filtered, grouped, vendors, loadMore };
}

interface WorkBuddyModelPickerProps {
  id: string;
  selectedIds: string[];
  onChange: (ids: string[]) => void;
  models: FetchedModel[];
  loading: boolean;
  onFetch: (forceRefresh?: boolean) => void;
  portalContainer?: HTMLElement | null;
}

function WorkBuddyModelPicker({
  id,
  selectedIds,
  onChange,
  models,
  loading,
  onFetch,
  portalContainer,
}: WorkBuddyModelPickerProps) {
  const [open, setOpen] = useState(false);
  const selected = useMemo(() => new Set(selectedIds), [selectedIds]);
  const allSelected =
    models.length > 0 && models.every((model) => selected.has(model.id));
  const { search, updateSearch, filtered, grouped, vendors, loadMore } =
    usePickerResults(models);

  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next) {
        updateSearch("");
        if (models.length === 0 && !loading) onFetch();
      }
    },
    [loading, models.length, onFetch, updateSearch],
  );

  const toggle = useCallback(
    (id: string) => {
      if (selected.has(id)) {
        onChange(selectedIds.filter((selectedId) => selectedId !== id));
      } else {
        onChange([...selectedIds, id]);
      }
    },
    [onChange, selected, selectedIds],
  );

  return (
    <div className="space-y-2">
      <div className="flex gap-1">
        <Popover open={open} onOpenChange={handleOpenChange}>
          <PopoverTrigger asChild>
            <Button
              id={id}
              type="button"
              variant="outline"
              role="combobox"
              aria-expanded={open}
              className="flex-1 justify-between font-normal"
            >
              <span
                className={cn(
                  "truncate",
                  selectedIds.length === 0 && "text-muted-foreground",
                )}
              >
                {selectedIds.length > 0
                  ? `已选择 ${selectedIds.length} 个兼容模型`
                  : "请选择至少一个兼容模型"}
              </span>
              <ChevronsUpDown className="ml-2 h-3.5 w-3.5 shrink-0 opacity-50" />
            </Button>
          </PopoverTrigger>
          <PopoverContent
            className="p-0"
            align="start"
            style={{ width: "var(--radix-popover-trigger-width)" }}
            container={portalContainer ?? undefined}
          >
            <Command shouldFilter={false}>
              <CommandInput
                placeholder="搜索模型或供应商…"
                className="h-9"
                value={search}
                onValueChange={updateSearch}
              />
              <CommandList className="max-h-72" onScroll={loadMore}>
                {loading ? (
                  <div className="flex items-center justify-center py-6 text-[12px] text-muted-foreground">
                    <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                    加载中…
                  </div>
                ) : (
                  <>
                    {filtered.length === 0 && (
                      <CommandEmpty>未找到兼容模型</CommandEmpty>
                    )}
                    {vendors.map((vendor) => (
                      <CommandGroup key={vendor} heading={vendor}>
                        {grouped[vendor].map((model) => (
                          <CommandItem
                            key={model.id}
                            value={`${model.id} ${model.name ?? ""} ${vendor}`}
                            onSelect={() => toggle(model.id)}
                          >
                            <span
                              className={cn(
                                "mr-2 flex h-4 w-4 shrink-0 items-center justify-center rounded border",
                                selected.has(model.id)
                                  ? "border-orange-500 bg-orange-500 text-white"
                                  : "border-border-default",
                              )}
                            >
                              {selected.has(model.id) && (
                                <Check className="h-3 w-3" />
                              )}
                            </span>
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-[12px] font-medium">
                                {model.name?.trim() || model.id}
                              </span>
                              <span className="block truncate text-[10px] text-muted-foreground">
                                {model.id}
                              </span>
                            </span>
                            {(model.supportedParameters ?? []).includes(
                              "reasoning",
                            ) && (
                              <span className="ml-2 rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] text-violet-600">
                                推理
                              </span>
                            )}
                            {(model.inputModalities ?? []).includes(
                              "image",
                            ) && (
                              <span className="ml-1 rounded bg-sky-500/10 px-1.5 py-0.5 text-[9px] text-sky-600">
                                视觉
                              </span>
                            )}
                          </CommandItem>
                        ))}
                      </CommandGroup>
                    ))}
                  </>
                )}
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
        <Button
          type="button"
          variant="outline"
          size="icon"
          onClick={() => onFetch(true)}
          disabled={loading}
          title="刷新模型列表"
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <RefreshCw className="h-3.5 w-3.5" />
          )}
        </Button>
      </div>
      <div className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
        <span>共 {models.length} 个兼容模型，共用一个 Ofox Key</span>
        <span className="flex shrink-0 gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-[11px]"
            disabled={loading || models.length === 0}
            onClick={() =>
              onChange(
                pickWorkBuddyCuratedModels(models).map((model) => model.id),
              )
            }
          >
            精选模型
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-[11px]"
            disabled={loading || models.length === 0 || allSelected}
            onClick={() => onChange(models.map((model) => model.id))}
          >
            全选兼容
          </Button>
        </span>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// PingStatusBox — connectivity result card
// ---------------------------------------------------------------------------
//
// Three visual states reflecting the probe lifecycle:
//   - idle    (loading=false, result=null) → "尚未测试" muted hint
//   - loading                              → spinner + "正在请求 OfoxAI 网关…"
//   - done    (result != null)             → success/failure dot + summary
//
// We deliberately render the *model name* alongside the status so a user who
// has clicked test, then edited the dropdown without re-testing, can see the
// stale result is for an older model and re-run.
interface PingStatusBoxProps {
  loading: boolean;
  result: PingResult | null;
  model: string;
}

/** Shown between the first and second "解除绑定" click: what will be restored. */
function UnbindPreviewPanel({ preview }: { preview: UnbindReport | null }) {
  const { t } = useTranslation();
  const changedKeys = preview
    ? [...preview.restoredKeys, ...preview.removedKeys]
    : [];
  return (
    <div
      role="note"
      className="mx-6 mb-3 space-y-1.5 rounded-md border border-red-200 bg-red-50/60 px-3 py-2 text-[12px] text-foreground dark:border-red-500/30 dark:bg-red-500/10"
    >
      {preview && preview.sharedKeptBy.length > 0 ? (
        <p>
          {t("unbind.sharedNotice", {
            others: toolLabels(preview.sharedKeptBy),
          })}
        </p>
      ) : preview?.alreadyUnbound ? (
        <p>{t("unbind.nothingToRestore")}</p>
      ) : (
        <>
          <p className="font-medium">{t("unbind.willRestoreTitle")}</p>
          {preview?.legacy && (
            <p className="text-amber-700 dark:text-amber-300">
              {t("unbind.legacyNotice")}
            </p>
          )}
          {changedKeys.length > 0 ? (
            <ul className="space-y-0.5 font-mono text-[11px] text-muted-foreground">
              {changedKeys.map((key) => (
                <li key={key}>{key}</li>
              ))}
            </ul>
          ) : (
            <p>{t("unbind.generic")}</p>
          )}
          {!preview?.legacy && <p>{t("unbind.forceRestoreNotice")}</p>}
          <p className="text-muted-foreground">{t("unbind.keepNotice")}</p>
        </>
      )}
    </div>
  );
}

function PingStatusBox({ loading, result, model }: PingStatusBoxProps) {
  if (loading) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-border-default bg-muted/30 px-3 py-2 text-[12px] text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        <span>正在请求 OfoxAI 网关…</span>
      </div>
    );
  }
  if (!result) {
    return (
      <div className="rounded-md border border-dashed border-border-default px-3 py-2 text-[12px] text-muted-foreground">
        尚未测试。点击右上方「测试连通性」可向所选模型发送一次 1-token
        验证请求。
      </div>
    );
  }
  if (result.success) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-[12px] text-emerald-700 dark:text-emerald-400">
        <CheckCircle2 className="h-4 w-4 shrink-0" />
        <span className="flex-1 truncate">
          连通成功（{result.latencyMs} ms）
          {model && (
            <span className="ml-1 text-muted-foreground">· {model}</span>
          )}
        </span>
      </div>
    );
  }
  return (
    <div className="flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-[12px] text-red-700 dark:text-red-400">
      <XCircle className="mt-0.5 h-4 w-4 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="font-medium">连通失败</div>
        <div className="mt-0.5 break-words text-[11.5px] text-red-700/80 dark:text-red-400/80">
          {result.error || "未知错误"}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ModelPicker — local searchable dropdown
// ---------------------------------------------------------------------------
//
// We don't reuse `ModelSelectFromApi` here because that component is wired to
// the i18n stack and the larger provider-form lifecycle. The manage dialog
// only needs a simple controlled picker, so a local component keeps the
// dependency graph clean and the empty/loading states tuned to *this* UX.

interface ModelPickerProps {
  id: string;
  value: string;
  onChange: (v: string) => void;
  models: FetchedModel[];
  loading: boolean;
  onFetch: (forceRefresh?: boolean) => void;
  /**
   * Portal target for the popover. Defaults to `document.body` when omitted.
   * Pass the dialog's content node when this picker is rendered inside a
   * Radix Dialog, so the popover lands inside the dialog's scroll-lock
   * shard and the search/option list stays scrollable.
   */
  portalContainer?: HTMLElement | null;
}

function ModelPicker({
  id,
  value,
  onChange,
  models,
  loading,
  onFetch,
  portalContainer,
}: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const { search, updateSearch, filtered, grouped, vendors, loadMore } =
    usePickerResults(models);

  // Trigger an initial fetch the first time the user opens the dropdown.
  // Subsequent opens reuse the cached list; the explicit refresh button
  // re-fetches on demand.
  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next) {
        updateSearch("");
        if (models.length === 0 && !loading) onFetch();
      }
    },
    [models.length, loading, onFetch, updateSearch],
  );

  return (
    <div className="flex gap-1">
      <Popover open={open} onOpenChange={handleOpenChange}>
        <PopoverTrigger asChild>
          <Button
            id={id}
            type="button"
            variant="outline"
            role="combobox"
            aria-expanded={open}
            className="flex-1 justify-between font-normal"
          >
            <span className={cn("truncate", !value && "text-muted-foreground")}>
              {value || "未设置（使用 OfoxAI 默认路由）"}
            </span>
            <ChevronsUpDown className="ml-2 h-3.5 w-3.5 shrink-0 opacity-50" />
          </Button>
        </PopoverTrigger>
        <PopoverContent
          className="p-0"
          align="start"
          // Match the trigger width so the list never visually mismatches
          // its anchor (Radix exposes this via CSS var).
          style={{ width: "var(--radix-popover-trigger-width)" }}
          // Portal into the dialog's content node so this popover lives
          // inside Radix Dialog's scroll-lock shard — otherwise wheel
          // events on the model list get swallowed and the dropdown looks
          // "stuck" (see `ModelPickerProps.portalContainer` above).
          container={portalContainer ?? undefined}
        >
          <Command shouldFilter={false}>
            <CommandInput
              placeholder="搜索模型…"
              className="h-9"
              value={search}
              onValueChange={updateSearch}
            />
            <CommandList onScroll={loadMore}>
              {loading ? (
                <div className="flex items-center justify-center py-6 text-[12px] text-muted-foreground">
                  <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                  加载中…
                </div>
              ) : (
                <>
                  {filtered.length === 0 && (
                    <CommandEmpty>未找到模型</CommandEmpty>
                  )}
                  {vendors.map((vendor) => (
                    <CommandGroup key={vendor} heading={vendor}>
                      {grouped[vendor].map((m) => (
                        <CommandItem
                          key={m.id}
                          value={m.id}
                          onSelect={() => {
                            onChange(m.id);
                            setOpen(false);
                          }}
                        >
                          <Check
                            className={cn(
                              "mr-2 h-3.5 w-3.5",
                              value === m.id ? "opacity-100" : "opacity-0",
                            )}
                          />
                          <span className="truncate">{m.id}</span>
                        </CommandItem>
                      ))}
                    </CommandGroup>
                  ))}
                </>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
      <Button
        type="button"
        variant="outline"
        size="icon"
        onClick={() => onFetch(true)}
        disabled={loading}
        title="刷新模型列表"
      >
        {loading ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
        ) : (
          <RefreshCw className="h-3.5 w-3.5" />
        )}
      </Button>
    </div>
  );
}
