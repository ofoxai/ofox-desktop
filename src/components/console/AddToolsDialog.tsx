import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { INSTALLABLE_TOOLS, TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { bindTools } from "@/lib/bindTools";
import { useToolInstall } from "@/hooks/useToolInstall";
import {
  ToolDiscoveryCard,
  type ToolStatus,
} from "@/components/onboarding/ToolDiscoveryCard";

interface ToolInfo {
  name: string;
  version: string | null;
  error: string | null;
  installationKind: "desktopApp" | "cli";
}

interface ToolEntry {
  id: string;
  label: string;
  status: ToolStatus;
  version: string | null;
  autoSelectTick: number;
}

interface AddToolsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 已绑定工具——在面板里以 bound 态展示（橙边 + 绿勾，不可点）。 */
  alreadyBound: string[];
  /** 新增绑定成功后回调。返回的是 bindTools succeeded 列表（完整新绑定集）。 */
  onAdded: (newBoundList: string[]) => void;
}

/** stagger 翻 selected 的相邻间隔（ms）——和 onboarding 保持一致。 */
const STAGGER_INTERVAL_MS = 200;

/**
 * "添加工具"对话框，复用 onboarding ToolDiscoveryCard 的 5 态状态机。
 *
 * 与 onboarding 的差异：
 *   - 已绑工具不过滤，而是以 `bound` 态展示在网格里——给用户"我目前的接入
 *     状态全貌"的视觉，新装/可绑的工具自然就站在尚未绑定的位置上
 *   - bound 态卡片不可点；只读
 *   - 确认按钮统计的是 selected 数（即"本次新绑的"），不含 bound
 *
 * 状态流：
 *   scanning（首屏 6 张占位，让对话框打开瞬间不空）
 *     → bound        // 已在 alreadyBound 集合里
 *     → missing      // 没装
 *     → unselected   // 装了 + 没绑——stagger 候选
 *     → selected     // stagger 翻进来（200ms / 跳过非候选）
 */
export default function AddToolsDialog({
  open,
  onOpenChange,
  alreadyBound,
  onAdded,
}: AddToolsDialogProps) {
  const alreadyBoundSet = useMemo(() => new Set(alreadyBound), [alreadyBound]);

  // 初始 6 张 scanning——对话框打开瞬间不显示空白网格。
  const [entries, setEntries] = useState<ToolEntry[]>(() =>
    TOOL_ORDER.map((id) => ({
      id,
      label: TOOL_META[id].label,
      status: "scanning" as ToolStatus,
      version: null,
      autoSelectTick: 0,
    })),
  );
  const [scanDone, setScanDone] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  const detect = useCallback(async () => {
    setScanDone(false);
    // 把所有 timer 重置一次——detect 可能在用户关-开对话框时重跑。
    timersRef.current.forEach(clearTimeout);
    timersRef.current = [];

    // 重新置 scanning（关-开对话框场景下 entries 还停在上一次的 selected/bound）
    setEntries(
      TOOL_ORDER.map((id) => ({
        id,
        label: TOOL_META[id].label,
        status: "scanning" as ToolStatus,
        version: null,
        autoSelectTick: 0,
      })),
    );

    try {
      const results = await invoke<ToolInfo[]>("get_tool_versions", {
        tools: null,
        wslShellByTool: null,
        includeLatest: false,
      });
      const byName = new Map(results.map((r) => [r.name, r]));

      // Step 1：scanning → bound / missing / unselected
      const settled: ToolEntry[] = TOOL_ORDER.map((id) => {
        const info = byName.get(id);
        const detected = !!info && !!info.version && !info.error;
        let status: ToolStatus;
        if (alreadyBoundSet.has(id)) {
          // 已绑工具即便本机检测不到（极少见，比如卸载了 CLI 但配置还在）
          // 也保留 bound 态——用户对"是否已绑"的认知靠 ofox 端，不依赖
          // 本地 CLI 是否仍可用。
          status = "bound";
        } else if (detected) {
          status = "unselected";
        } else {
          status = "missing";
        }
        return {
          id,
          label: TOOL_META[id].label,
          status,
          version: info?.version ?? null,
          autoSelectTick: 0,
        };
      });
      setEntries(settled);

      // Step 2：stagger 把 unselected 逐个翻 selected。bound/missing 不占 tick。
      const candidateIds = settled
        .filter((e) => e.status === "unselected")
        .map((e) => e.id);
      candidateIds.forEach((id, i) => {
        const t = setTimeout(() => {
          setEntries((prev) =>
            prev.map((e) =>
              e.id === id
                ? {
                    ...e,
                    status: "selected",
                    autoSelectTick: e.autoSelectTick + 1,
                  }
                : e,
            ),
          );
        }, i * STAGGER_INTERVAL_MS);
        timersRef.current.push(t);
      });

      // 最后一张卡翻 selected 的瞬间 scanDone=true。无候选时立即就绪。
      const doneT = setTimeout(
        () => setScanDone(true),
        Math.max(0, candidateIds.length - 1) * STAGGER_INTERVAL_MS,
      );
      timersRef.current.push(doneT);
    } catch (e) {
      console.error("[AddToolsDialog] detect failed", e);
      // 失败兜底：保留 bound 显示已绑、其它全 missing，让用户至少看到现状。
      setEntries(
        TOOL_ORDER.map((id) => ({
          id,
          label: TOOL_META[id].label,
          status: alreadyBoundSet.has(id) ? "bound" : "missing",
          version: null,
          autoSelectTick: 0,
        })),
      );
      setScanDone(true);
    }
  }, [alreadyBoundSet]);

  useEffect(() => {
    if (open) {
      void detect();
    }
    return () => {
      timersRef.current.forEach(clearTimeout);
      timersRef.current = [];
    };
  }, [open, detect]);

  // 装完一个 missing 工具就重扫，让卡片自动从 missing 翻 unselected/selected。
  // 注意：在 stagger 期间装完也安全——detect() 会重置 entries 和 scanDone，
  // 用户体验是"哦原来这个工具装好了，自动选中给我了"。
  const {
    installing,
    install,
    error: installError,
    progress: installProgress,
  } = useToolInstall((toolId, code) => {
    if (code === 0 && open) {
      void detect();
    } else if (code !== 0) {
      console.warn(`[AddToolsDialog] install_tool(${toolId}) exit=${code}`);
    }
  });

  const handleToggle = (id: string) => {
    // scanDone 之前完全不响应——stagger 进行中点击会被下一 tick 覆盖。
    if (!scanDone || submitting) return;
    setEntries((prev) =>
      prev.map((e) => {
        if (e.id !== id) return e;
        if (e.status === "selected") return { ...e, status: "unselected" };
        if (e.status === "unselected") return { ...e, status: "selected" };
        return e; // bound / missing / scanning 不动
      }),
    );
  };

  const selected = entries.filter((e) => e.status === "selected");
  // 候选数 = 未绑且装了的工具——文案"检测到 N 个可绑定"应反映这个口径，
  // 不包含已绑的 bound 卡。
  const candidateCount = entries.filter(
    (e) => e.status === "selected" || e.status === "unselected",
  ).length;
  const isScanning = entries.some((e) => e.status === "scanning");

  const handleConfirm = async () => {
    if (selected.length === 0) return;
    setSubmitting(true);
    try {
      // bindTools 按"完整列表"语义重写 localStorage，所以要带上已绑工具。
      // 它对已绑做幂等二次 bind——失败会 toast 警告。
      const newlySelectedIds = selected.map((e) => e.id);
      const merged = Array.from(
        new Set([...alreadyBound, ...newlySelectedIds]),
      );
      const succeeded = await bindTools(merged);
      // 真实成功的"新增"——避免在用户选了 2 个、只成 1 个时谎报。
      const newlySucceeded = succeeded.filter((id) =>
        newlySelectedIds.includes(id),
      );
      if (newlySucceeded.length > 0) {
        toast.success(`已绑定 ${newlySucceeded.length} 个工具`);
      }
      onAdded(succeeded);
      onOpenChange(false);
    } catch (e) {
      console.error("[AddToolsDialog] bind failed", e);
      toast.error("绑定失败，请稍后重试");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>添加工具</DialogTitle>
          <DialogDescription>
            {isScanning
              ? "正在扫描已安装的 AI 工具…"
              : candidateCount === 0
                ? "没有可添加的工具——所有已识别的工具都已绑定。"
                : `检测到 ${candidateCount} 个可绑定的工具，勾选后确认即可接入。`}
          </DialogDescription>
        </DialogHeader>

        {/* 顶部 mt-2 给绿勾留出溢出空间——勾压在卡片顶边（-top-2），
            网格容器没有额外间距时会被对话框 padding 裁掉一半。 */}
        <div className="mt-2 grid grid-cols-3 gap-3 px-6 py-4">
          {entries.map((e) => {
            // installing 覆盖 entries 里的 status —— 同 ToolDiscoveryPage。
            // bound 工具理论上不会进 install 流程（按钮也只在 missing 渲染），
            // 但 effective 取代会自然处理。
            const effective: ToolStatus = installing.has(e.id)
              ? "installing"
              : e.status;
            return (
              <ToolDiscoveryCard
                key={e.id}
                toolId={e.id}
                label={e.label}
                version={e.version}
                status={effective}
                autoSelectTick={e.autoSelectTick}
                progress={installProgress[e.id]}
                onClick={() => handleToggle(e.id)}
                onInstall={
                  INSTALLABLE_TOOLS.includes(e.id)
                    ? () => install(e.id)
                    : undefined
                }
              />
            );
          })}
        </div>

        {installError && (
          // 安装脚本没起来时唯一的用户可见反馈——同 ToolDiscoveryPage。
          <div
            role="alert"
            className="mx-6 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300"
          >
            安装 {TOOL_META[installError.toolId]?.label ?? installError.toolId}{" "}
            失败：{installError.message}
          </div>
        )}

        <DialogFooter>
          <button
            onClick={() => onOpenChange(false)}
            disabled={submitting}
            className="rounded-md px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            取消
          </button>
          <button
            onClick={handleConfirm}
            disabled={selected.length === 0 || submitting || !scanDone}
            className="rounded-md bg-gradient-to-r from-orange-400 to-orange-500 px-4 py-2 text-sm font-medium text-white shadow-sm hover:from-orange-500 hover:to-orange-600 disabled:opacity-50"
          >
            {submitting
              ? "正在绑定…"
              : !scanDone
                ? "扫描中…"
                : `确认（${selected.length}）`}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
