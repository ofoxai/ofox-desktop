import { useRef, useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { manageToolApi } from "@/lib/api/manageTool";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { ToolBadge } from "@/components/tools/ToolBadge";

interface ToolStatusListProps {
  boundTools: string[];
}

interface ToolRowData {
  id: string;
  label: string;
  /** 当前激活的 Ofox 模型；拉不到时为 null（保持 UI 不抖）。 */
  model: string | null;
}

export default function ToolStatusList({ boundTools }: ToolStatusListProps) {
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

          const row: ToolRowData = {
            id,
            label: meta.label,
            model,
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
              const subtitle = row.model ?? "未配置模型";

              // ToolBadge size=40 对齐 "工具名 + 模型" 两行文字
              // (12px + 10px + leading + 间距 ≈ 36-40px) 的上下边。
              return (
                <div
                  key={row.id}
                  className="flex items-center gap-2.5 rounded-md px-1.5 py-1.5 hover:bg-accent/50"
                >
                  <ToolBadge toolId={row.id} size={40} />
                  <div className="min-w-0 flex-1">
                    <div className="text-[12px] font-medium leading-tight text-foreground">
                      {row.label}
                    </div>
                    <div className="truncate text-[10px] text-muted-foreground">
                      {subtitle}
                    </div>
                  </div>
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
