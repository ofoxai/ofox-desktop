import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { emit } from "@tauri-apps/api/event";
import {
  Loader2,
  RefreshCw,
  Check,
  ChevronsUpDown,
  Folder,
  Zap,
  CheckCircle2,
  XCircle,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
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
  TOOL_PROTOCOL,
  type PingResult,
} from "@/lib/api/manageTool";
import {
  fetchOfoxModels,
  filterOfoxModelsByProtocol,
  type FetchedModel,
} from "@/lib/api/model-fetch";
import { unbindTool } from "@/lib/bindTools";
import type { AppId } from "@/lib/api/types";

/** Subset of `ConsolePage`'s `BoundTool` that the dialog actually needs.
 *  Kept structural so we can call it with the full BoundTool object. */
export interface ManageToolTarget {
  id: string;
  abbr: string;
  label: string;
  color: string;
  version: string | null;
  statusText: string;
}

interface ManageToolDialogProps {
  /** Non-null = dialog is open and managing this tool. Null closes. */
  tool: ManageToolTarget | null;
  onOpenChange: (open: boolean) => void;
  /** Fired after a successful save or successful unbind so ConsolePage can
   *  refresh its list (status pill, monthly tokens, presence after unbind). */
  onChanged?: () => void;
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
  const open = !!tool;
  const protocol = tool ? TOOL_PROTOCOL[tool.id] : undefined;

  // Anchors the model-picker's Popover portal inside the dialog so its
  // CommandList stays scrollable. Radix Dialog wraps its content in
  // `react-remove-scroll` with the content node as the only "scroll shard";
  // any popover portaled to body falls outside the shard and has its wheel
  // events swallowed. Pointing the popover's `container` at this ref puts it
  // back inside the shard.
  const dialogContentRef = useRef<HTMLDivElement>(null);

  // --- per-tool state ------------------------------------------------------
  // Reset every time we switch tools so the dialog never shows stale data
  // from a previous open.
  const [filePath, setFilePath] = useState<string | null>(null);
  const [filePathError, setFilePathError] = useState<string | null>(null);
  const [currentModel, setCurrentModel] = useState<string>("");
  const [draftModel, setDraftModel] = useState<string>("");
  const [models, setModels] = useState<FetchedModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [unbindConfirming, setUnbindConfirming] = useState(false);
  const [unbindLoading, setUnbindLoading] = useState(false);
  // Connectivity probe — null means "未测试". The backend always resolves
  // with a PingResult, so we never put an exception here.
  const [pingResult, setPingResult] = useState<PingResult | null>(null);
  const [pingLoading, setPingLoading] = useState(false);

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
    setModels([]);
    setModelsLoading(false);
    setSaving(false);
    setUnbindConfirming(false);
    setUnbindLoading(false);
    setPingResult(null);
    setPingLoading(false);

    (async () => {
      try {
        const path = await manageToolApi.getConfigFilePath(tool.id);
        if (!cancelled) setFilePath(path);
      } catch (e) {
        if (!cancelled) setFilePathError(String(e));
      }
      try {
        const m = await manageToolApi.getActiveModel(tool.id);
        if (!cancelled) {
          setCurrentModel(m);
          setDraftModel(m);
        }
      } catch (e) {
        // Soft-fail: an empty model just means "OfoxAI default routing",
        // which is a valid state. Don't block the dialog over it.
        console.warn(`[ManageToolDialog] getActiveModel(${tool.id}) failed`, e);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [tool]);

  // Lazy model fetch — only runs when the user clicks the dropdown trigger
  // (or the refresh button). We never preload on open, since hitting the
  // network for a dialog the user might just be peeking at is wasteful.
  const fetchModels = useCallback(async () => {
    if (!protocol) return;
    setModelsLoading(true);
    try {
      const all = await fetchOfoxModels(protocol);
      setModels(filterOfoxModelsByProtocol(all, protocol));
    } catch (e) {
      console.error("[ManageToolDialog] fetchOfoxModels failed", e);
      toast.error(`获取模型列表失败：${String(e)}`);
    } finally {
      setModelsLoading(false);
    }
  }, [protocol]);

  const handleOpenFolder = useCallback(async () => {
    if (!tool) return;
    try {
      await settingsApi.openConfigFolder(tool.id as AppId);
    } catch (e) {
      toast.error(`打开配置目录失败：${String(e)}`);
    }
  }, [tool]);

  const handleSave = useCallback(async () => {
    if (!tool) return;
    if (draftModel === currentModel) return;
    setSaving(true);
    try {
      await manageToolApi.setActiveModel(tool.id, draftModel);
      setCurrentModel(draftModel);
      // 唤醒后端 tool_health 循环立刻重跑——上一轮跑的时候 model 还是空，
      // cache 里存的是 Plan::Skip{reason:"未配置模型"}。不 emit 的话用户得
      // 等 1h/6h/24h 才会看到行尾 pill 从"未配置模型"变成"延迟 X ms"，
      // 体感像没保存成功。
      await emit("ofox-prefs-updated");
      toast.success("模型已更新");
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
  }, [tool, draftModel, currentModel, onChanged, onOpenChange]);

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
    if (!draftModel) {
      toast.error("请先选择一个模型再测试连通性");
      return;
    }
    setPingLoading(true);
    setPingResult(null);
    try {
      const result = await manageToolApi.pingModel(tool.id, draftModel);
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
  }, [tool, draftModel]);

  const handleUnbind = useCallback(async () => {
    if (!tool) return;
    if (!unbindConfirming) {
      // First click → arm the confirm state. The button label flips to make
      // the destructive intent obvious before the second click commits.
      setUnbindConfirming(true);
      return;
    }
    setUnbindLoading(true);
    try {
      await unbindTool(tool.id);
      toast.success(`${tool.label} 已解除绑定`);
      onChanged?.();
      onOpenChange(false);
    } catch (e) {
      toast.error(`解除绑定失败：${String(e)}`);
      setUnbindConfirming(false);
    } finally {
      setUnbindLoading(false);
    }
  }, [tool, unbindConfirming, onChanged, onOpenChange]);

  const isDirty = draftModel !== currentModel;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent ref={dialogContentRef} className="max-w-lg gap-0 p-0">
        {tool && (
          <>
            <DialogHeader className="flex-row items-center gap-3 space-y-0">
              <div
                className={cn(
                  "flex h-10 w-10 shrink-0 items-center justify-center rounded-xl text-[12px] font-bold text-white",
                  tool.color,
                )}
              >
                {tool.abbr}
              </div>
              <div className="min-w-0 flex-1 text-left">
                <div className="text-[15px] font-semibold text-foreground">
                  {tool.label}
                </div>
                <div className="truncate text-[12px] text-muted-foreground">
                  {tool.statusText}
                  {tool.version ? ` · v${tool.version}` : " · 未检测到"}
                </div>
              </div>
            </DialogHeader>

            <div className="flex-1 overflow-y-auto px-6 py-5 space-y-5">
              {/* ---- Model ---- */}
              <div className="space-y-2">
                <Label htmlFor="manage-model" className="text-[13px]">
                  模型
                </Label>
                {protocol ? (
                  <ModelPicker
                    id="manage-model"
                    value={draftModel}
                    onChange={setDraftModel}
                    models={models}
                    loading={modelsLoading}
                    onFetch={fetchModels}
                    portalContainer={dialogContentRef.current}
                  />
                ) : (
                  <div className="rounded-md border border-dashed border-border-default px-3 py-2 text-[12px] text-muted-foreground">
                    该工具暂不支持模型管理
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

              {/* ---- Connectivity ----
                   Ping *draft* model (not saved) so the user can vet their
                   choice before committing. Empty state until the first run;
                   afterwards the dot reflects the most recent attempt. */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label className="text-[13px]">连通性</Label>
                  <Button
                    type="button"
                    size="sm"
                    onClick={handlePing}
                    disabled={pingLoading || !protocol || !draftModel}
                    // Brand-orange to match the other primary CTAs in the app
                    // (充值 / 修复 / 打开控制台). The ping action is the focal
                    // verb of this section, so it gets the same weight rather
                    // than the muted outline used for secondary controls.
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
                  model={draftModel}
                />
              </div>
            </div>

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
                {unbindConfirming ? "确认解除绑定？" : "解除绑定"}
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
                  disabled={!isDirty || saving || !protocol}
                >
                  {saving && (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  )}
                  保存
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
  onFetch: () => void;
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

  // Trigger an initial fetch the first time the user opens the dropdown.
  // Subsequent opens reuse the cached list; the explicit refresh button
  // re-fetches on demand.
  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next && models.length === 0 && !loading) {
        onFetch();
      }
    },
    [models.length, loading, onFetch],
  );

  // Group by vendor (everything before the first "/", falling back to ownedBy
  // or "Other"). Matches what ModelSelectFromApi does so the UX is familiar.
  const grouped = useMemo(() => {
    const map: Record<string, FetchedModel[]> = {};
    for (const m of models) {
      const slash = m.id.indexOf("/");
      const vendor = slash > 0 ? m.id.slice(0, slash) : m.ownedBy || "Other";
      (map[vendor] ||= []).push(m);
    }
    return map;
  }, [models]);
  const vendors = useMemo(() => Object.keys(grouped).sort(), [grouped]);

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
          <Command>
            <CommandInput placeholder="搜索模型…" className="h-9" />
            <CommandList>
              {loading ? (
                <div className="flex items-center justify-center py-6 text-[12px] text-muted-foreground">
                  <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                  加载中…
                </div>
              ) : (
                <>
                  <CommandEmpty>未找到模型</CommandEmpty>
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
        onClick={onFetch}
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
