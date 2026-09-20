import { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { INSTALLABLE_TOOLS, TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { useToolInstall } from "@/hooks/useToolInstall";
import { ToolDiscoveryCard, type ToolStatus } from "./ToolDiscoveryCard";

interface ToolInfo {
  name: string;
  version: string | null;
  error: string | null;
}

interface ToolEntry {
  id: string;
  label: string;
  status: ToolStatus;
  version: string | null;
  autoSelectTick: number;
}

interface ToolDiscoveryPageProps {
  /**
   * Optional. When omitted, the "返回" button is hidden — useful when this
   * page is the entry point of onboarding (already-logged-in user with no
   * bound tools), where there's nothing meaningful to go back to.
   */
  onBack?: () => void;
  onBind: (selectedTools: string[]) => void;
}

/** Stagger 翻 selected 的相邻间隔（ms）。跳过 missing 不占 tick。 */
const STAGGER_INTERVAL_MS = 200;

export default function ToolDiscoveryPage({
  onBack,
  onBind,
}: ToolDiscoveryPageProps) {
  // 初始就给 6 张 scanning 卡片占位——之前是 tools=[]，扫描期间整面什么也
  // 看不到，用户不知道有哪些可选工具。改成首屏即就位，扫描完成后从
  // scanning → missing/unselected → selected 渐进推进。
  const [entries, setEntries] = useState<ToolEntry[]>(() =>
    TOOL_ORDER.map((id) => ({
      id,
      label: TOOL_META[id].label,
      status: "scanning" as ToolStatus,
      version: null,
      autoSelectTick: 0,
    })),
  );
  // scanDone：stagger 全部跑完才翻 true，确认按钮才允许点。覆盖两种边界：
  //   1. 扫描出错（catch 分支）—— 直接 true，让用户能继续 onboarding 即便
  //      6 张都是 missing
  //   2. 没有任何工具被检测到 —— detectedIds 为空，stagger 不会启动；
  //      doneT 仍按 `Math.max(0, len-1)*INTERVAL` 调度（0ms 即触发）
  const [scanDone, setScanDone] = useState(false);
  // 保存所有 stagger / scanDone 定时器，组件卸载或 detectTools 重跑时统一
  // clearTimeout——否则 unmount 后 setEntries 会触发 setState-after-unmount
  // 警告（React 18 dev only，但读 console 时干扰排障）。
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  const detectTools = useCallback(async () => {
    try {
      // Onboarding 只需要本地版本——不查 npm/GitHub 最新版（那是 console
      // 里的"有新版本"提示用的），跳过远程 fetch 让 invoke 更快返回。
      const results = await invoke<ToolInfo[]>("get_tool_versions", {
        tools: null,
        wslShellByTool: null,
        includeLatest: false,
      });
      const byName = new Map(results.map((r) => [r.name, r]));

      // Step 1：把 scanning 一次性翻成 missing（没装）或 unselected（装了）。
      // 此时整页"扫描中..."标题变成"已发现 N 个"，每卡的版本号 / 灰显状态
      // 落定，但 detected 的卡还都是 unselected——下一步才逐个翻 selected。
      const settled: ToolEntry[] = TOOL_ORDER.map((id) => {
        const info = byName.get(id);
        const detected = !!info && !!info.version && !info.error;
        return {
          id,
          label: TOOL_META[id].label,
          status: detected ? ("unselected" as ToolStatus) : "missing",
          version: info?.version ?? null,
          autoSelectTick: 0,
        };
      });
      setEntries(settled);

      // Step 2：按 TOOL_ORDER 跳过 missing 逐个 200ms 翻 selected。跳过
      // missing 是为了视觉节奏 —— 比如 claude(装)/codex(没装)/opencode(装)
      // 时 opencode 在 claude 之后 200ms，而不是 400ms（让没装的工具占
      // 一个 tick 空白）。
      const detectedIds = settled
        .filter((e) => e.status === "unselected")
        .map((e) => e.id);
      detectedIds.forEach((id, i) => {
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

      // scanDone 在最后一张卡翻 selected **的瞬间**翻 true，让"开始绑定"按钮
      // 跟着最后一张卡的高亮一起出现。Math.max(0, len-1) 处理 0 工具被检测
      // 到的边界（立刻 ready）。
      const doneT = setTimeout(
        () => setScanDone(true),
        Math.max(0, detectedIds.length - 1) * STAGGER_INTERVAL_MS,
      );
      timersRef.current.push(doneT);
    } catch (e) {
      console.error("Tool detection failed:", e);
      // 全部置 missing 让用户至少能"返回"或硬选 —— 不阻塞 onboarding。
      setEntries(
        TOOL_ORDER.map((id) => ({
          id,
          label: TOOL_META[id].label,
          status: "missing",
          version: null,
          autoSelectTick: 0,
        })),
      );
      setScanDone(true);
    }
  }, []);

  useEffect(() => {
    detectTools();
    return () => {
      timersRef.current.forEach(clearTimeout);
      timersRef.current = [];
    };
  }, [detectTools]);

  // 工具自动安装——missing 卡片右下角"安装"按钮触发。装完成功就重扫，让
  // 卡片从 missing 状态自动翻到 unselected/selected；失败则保持 missing
  // 态让用户可以再点一次（init.py 的断点续装会跳过已完成的子步骤）。
  const {
    installing,
    install,
    error: installError,
    progress: installProgress,
  } = useToolInstall((toolId, code) => {
    if (code === 0) {
      void detectTools();
    } else {
      console.warn(`[ToolDiscoveryPage] install_tool(${toolId}) exit=${code}`);
    }
  });

  const handleToggle = (id: string) => {
    // 扫描中 / stagger 还没跑完都不允许手动切。前者是显式产品规则
    // ("等所有扫描结束才能流转")，后者是为了不让用户在自动选中动画中
    // 抢一次 click 造成状态混乱（比如点中的卡正好下一个 tick 要翻
    // selected，会被覆盖回去）。
    if (!scanDone) return;
    setEntries((prev) =>
      prev.map((e) => {
        if (e.id !== id) return e;
        if (e.status === "selected") return { ...e, status: "unselected" };
        if (e.status === "unselected") return { ...e, status: "selected" };
        return e; // missing / scanning 不动
      }),
    );
  };

  const selected = entries.filter((e) => e.status === "selected");
  // detectedCount 包括 selected 和 unselected——只要不是 scanning/missing
  // 就算"扫描发现的工具"。文案"已发现 N 个" 不依赖用户最终选了几个。
  const detectedCount = entries.filter(
    (e) => e.status !== "scanning" && e.status !== "missing",
  ).length;
  const isScanning = entries.some((e) => e.status === "scanning");

  return (
    <div className="flex h-screen w-full items-center justify-center bg-gradient-to-br from-orange-50/80 via-white to-orange-50/40 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      <div className="flex w-full max-w-2xl flex-col items-center px-8">
        <h1 className="mb-2 text-3xl font-bold text-foreground">
          {isScanning
            ? "正在扫描 AI 工具…"
            : `已发现 ${detectedCount} 个 AI 工具`}
        </h1>
        <p className="mb-8 text-center text-sm text-muted-foreground">
          选择要接入 Ofox 的工具，我们将生成独立 API Key 并备份原配置
        </p>

        <div className="mb-8 grid w-full grid-cols-3 gap-4">
          {entries.map((e) => {
            // installing 覆盖 entries 里的 status——entries 是检测结果的镜像，
            // 不知道"我正在装它"；installing 集合才是当前操作的真相。
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
          // 安装脚本压根没起来时（找不到 init.sh / 非 arm64）唯一的用户可见
          // 反馈——没有这条，点"安装"会表现为完全没反应。
          <div
            role="alert"
            className="mb-4 w-full rounded-lg border border-red-200 bg-red-50 px-4 py-2.5 text-xs text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300"
          >
            安装 {TOOL_META[installError.toolId]?.label ?? installError.toolId}{" "}
            失败：{installError.message}
          </div>
        )}

        <div className="flex w-full max-w-sm items-center justify-center gap-4">
          {onBack && (
            <button
              onClick={onBack}
              className="px-6 py-2.5 text-sm text-muted-foreground hover:text-foreground"
            >
              返回
            </button>
          )}
          <button
            onClick={() => onBind(selected.map((e) => e.id))}
            disabled={!scanDone || selected.length === 0}
            className="flex-1 rounded-xl bg-gradient-to-r from-orange-400 to-orange-500 px-6 py-3 text-base font-semibold text-white shadow-md shadow-orange-200 transition-all hover:from-orange-500 hover:to-orange-600 disabled:opacity-50 dark:shadow-orange-900/20"
          >
            {!scanDone ? "扫描中…" : `开始绑定（${selected.length}）→`}
          </button>
        </div>
      </div>
    </div>
  );
}
