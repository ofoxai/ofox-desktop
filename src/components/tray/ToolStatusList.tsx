import { useRef, useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { manageToolApi } from "@/lib/api/manageTool";
import { usageApi } from "@/lib/api/usage";
import {
  TOOL_META,
  TOOL_ORDER,
  PROXY_SUPPORTED_TOOLS,
} from "@/config/toolMeta";

/**
 * 单个工具的"活跃度"——与主窗口 ConsolePage 的状态点完全同语义：
 *
 *   active = 已接管 + 检测到二进制     (绿点)
 *   error  = 已接管但未检测到 / 未检测到 (红点)
 *   idle   = 不支持代理 / 未开启代理   (灰点)
 *
 * 由 TrayPopoverApp 集中算好后透传给 StatsRow / ToolStatusList，
 * 保证 popover 内部多处状态显示一致。
 */
export type ToolActiveness = "active" | "error" | "idle";
export type ToolActivenessMap = Record<string, ToolActiveness>;

interface ToolStatusListProps {
  boundTools: string[];
  activeness: ToolActivenessMap;
}

interface ToolRowData {
  id: string;
  abbr: string;
  label: string;
  color: string;
  /** 当前激活的 Ofox 模型；拉不到时为 null（保持 UI 不抖）。 */
  model: string | null;
  /** 今日花费 USD；不在 PROXY_SUPPORTED_TOOLS 时为 null。 */
  todayCostUsd: number | null;
  /** 今日是否有过请求（用于在金额=0 时显示"未使用"而非 $0.00）。 */
  todayHasUsage: boolean;
}

/** activeness -> 状态点颜色。 */
const STATUS_DOT_BG: Record<ToolActiveness, string> = {
  active: "bg-green-500",
  error: "bg-red-500",
  idle: "bg-gray-400",
};

/** activeness -> hover title 文案，与主页 statusText 同口径方便用户对照。 */
const STATUS_TITLE: Record<ToolActiveness, string> = {
  active: "已接管",
  error: "未检测到",
  idle: "未开启代理 / 不支持代理统计",
};

function todayStartSec(): number {
  const t = new Date();
  t.setHours(0, 0, 0, 0);
  return Math.floor(t.getTime() / 1000);
}

function formatUsd(value: number): string {
  return `$${value.toFixed(2)}`;
}

/** 解析 totalCost 字段（后端返回的是字符串小数）。 */
function parseTotalCost(s: string | undefined): number {
  if (!s) return 0;
  const n = Number(s);
  return Number.isFinite(n) ? n : 0;
}

export default function ToolStatusList({
  boundTools,
  activeness,
}: ToolStatusListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollRatio, setScrollRatio] = useState(0);
  const [thumbRatio, setThumbRatio] = useState(1);
  const [showTrack, setShowTrack] = useState(false);
  const [rows, setRows] = useState<ToolRowData[]>([]);

  const update = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    const overflow = scrollHeight - clientHeight;
    setShowTrack(overflow > 0);
    setThumbRatio(Math.min(clientHeight / scrollHeight, 1));
    setScrollRatio(overflow > 0 ? scrollTop / overflow : 0);
  }, []);

  useEffect(() => {
    update();
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [update]);

  // 拉每个工具的当前模型 + 今日花费。在 mount + window focus 时各拉一次。
  useEffect(() => {
    let cancelled = false;

    const sortedIds = boundTools
      .filter((id) => TOOL_META[id])
      .sort((a, b) => TOOL_ORDER.indexOf(a) - TOOL_ORDER.indexOf(b));

    if (sortedIds.length === 0) {
      setRows([]);
      return;
    }

    const loadAll = async () => {
      const results = await Promise.all(
        sortedIds.map(async (id) => {
          const meta = TOOL_META[id];
          // 模型名：拉不到不影响其它字段
          let model: string | null = null;
          try {
            const m = await manageToolApi.getActiveModel(id);
            model = typeof m === "string" && m.trim() ? m : null;
          } catch {
            model = null;
          }

          // 今日花费：只查代理支持的工具，避免对 opencode 等返回 0 误导
          let todayCostUsd: number | null = null;
          let todayHasUsage = false;
          if (PROXY_SUPPORTED_TOOLS.includes(id)) {
            try {
              const s = await usageApi.getUsageSummary(
                todayStartSec(),
                undefined,
                id,
              );
              todayCostUsd = parseTotalCost(s.totalCost);
              todayHasUsage = s.totalRequests > 0;
            } catch {
              todayCostUsd = null;
            }
          }

          const row: ToolRowData = {
            id,
            abbr: meta.abbr,
            label: meta.label,
            color: meta.color,
            model,
            todayCostUsd,
            todayHasUsage,
          };
          return row;
        }),
      );
      if (!cancelled) setRows(results);
    };

    void loadAll();

    const onFocus = () => void loadAll();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
    };
    // 依赖 boundTools 数组本身变化（参考引用）— mount 一次即可，
    // boundTools 由父组件控制刷新节奏。
  }, [boundTools]);

  const handleEmptyClick = useCallback(async () => {
    try {
      await invoke("show_main_window");
      await getCurrentWindow().hide();
    } catch (e) {
      console.error("[ToolStatusList] open main failed", e);
    }
  }, []);

  return (
    <div className="flex min-h-0 flex-1 flex-col border-t border-border">
      {/* 固定标题 */}
      <div className="shrink-0 px-3.5 pt-2 pb-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        工具状态
      </div>

      {/* 可滚动列表 + 自定义滚动条 */}
      <div className="relative min-h-0 flex-1">
        <div
          ref={scrollRef}
          onScroll={update}
          className="h-full overflow-y-auto px-2 pb-1"
        >
          {rows.length === 0 ? (
            <button
              onClick={handleEmptyClick}
              className="mx-1.5 my-2 flex w-[calc(100%-12px)] flex-col items-center justify-center gap-0.5 rounded-md border border-dashed border-border/70 px-3 py-4 text-center transition-colors hover:bg-accent/50"
            >
              <div className="text-[12px] font-medium text-foreground">
                暂未绑定工具
              </div>
              <div className="text-[10px] text-muted-foreground">
                点此前往主窗口完成绑定
              </div>
            </button>
          ) : (
            rows.map((row) => {
              const state = activeness[row.id] ?? "idle";
              const dotColor = STATUS_DOT_BG[state];

              const subtitleParts: string[] = [];
              if (row.model) subtitleParts.push(row.model);
              if (PROXY_SUPPORTED_TOOLS.includes(row.id)) {
                if (row.todayCostUsd === null) {
                  subtitleParts.push("今日数据加载中…");
                } else if (row.todayHasUsage) {
                  subtitleParts.push(`${formatUsd(row.todayCostUsd)} 今日`);
                } else {
                  subtitleParts.push("今日未使用");
                }
              } else {
                subtitleParts.push("不支持统计");
              }
              const subtitle =
                subtitleParts.length > 0 ? subtitleParts.join(" · ") : "—";

              return (
                <div
                  key={row.id}
                  className="flex items-center gap-2.5 rounded-md px-1.5 py-1.5 hover:bg-accent/50"
                >
                  <div
                    className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-md ${row.color} text-[10px] font-bold text-white`}
                  >
                    {row.abbr}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="text-[12px] font-medium leading-tight text-foreground">
                      {row.label}
                    </div>
                    <div className="truncate text-[10px] text-muted-foreground">
                      {subtitle}
                    </div>
                  </div>
                  <span
                    className={`h-2 w-2 shrink-0 rounded-full ${dotColor}`}
                    title={STATUS_TITLE[state]}
                  />
                </div>
              );
            })
          )}
        </div>

        {/* 自定义滚动进度条 */}
        {showTrack && (
          <div className="absolute right-0.5 top-0 bottom-0 flex w-1.5 items-stretch py-1">
            <div className="relative w-full rounded-full bg-black/5 dark:bg-white/10">
              <div
                className="absolute left-0 right-0 rounded-full bg-orange-400/60 transition-all duration-100"
                style={{
                  height: `${Math.max(thumbRatio * 100, 20)}%`,
                  top: `${scrollRatio * (100 - Math.max(thumbRatio * 100, 20))}%`,
                }}
              />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
