import { useRef, useState, useEffect, useCallback } from "react";

const MOCK_TOOLS = [
  {
    abbr: "CC",
    color: "bg-orange-700",
    name: "Claude Code",
    subtitle: "Claude Sonnet 4.6 \u00b7 $1.24 今日",
    status: "active" as const,
  },
  {
    abbr: "Cu",
    color: "bg-orange-600",
    name: "Cursor",
    subtitle: "Claude Sonnet 4.6 \u00b7 $1.86 今日",
    status: "active" as const,
  },
  {
    abbr: "OC",
    color: "bg-orange-500",
    name: "OpenCode",
    subtitle: "GPT-5.2 \u00b7 $0.72 今日",
    status: "active" as const,
  },
  {
    abbr: "Co",
    color: "bg-orange-600",
    name: "Continue",
    subtitle: "Claude Sonnet 4.6 \u00b7 未使用",
    status: "idle" as const,
  },
  {
    abbr: "Ai",
    color: "bg-orange-500",
    name: "Aider",
    subtitle: "认证失败（HTTP 401）\u00b7 需要重新绑定",
    status: "error" as const,
  },
];

const STATUS_DOT: Record<string, string> = {
  active: "bg-green-500",
  idle: "bg-gray-400",
  error: "bg-red-500",
};

export default function ToolStatusList() {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollRatio, setScrollRatio] = useState(0);
  const [thumbRatio, setThumbRatio] = useState(1);
  const [showTrack, setShowTrack] = useState(false);

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
          {MOCK_TOOLS.map((tool) => (
            <div
              key={tool.name}
              className="flex items-center gap-2.5 rounded-md px-1.5 py-1.5 hover:bg-accent/50"
            >
              <div
                className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-md ${tool.color} text-[10px] font-bold text-white`}
              >
                {tool.abbr}
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-[12px] font-medium leading-tight text-foreground">
                  {tool.name}
                </div>
                <div className="truncate text-[10px] text-muted-foreground">
                  {tool.subtitle}
                </div>
              </div>
              <span
                className={`h-2 w-2 shrink-0 rounded-full ${STATUS_DOT[tool.status]}`}
              />
            </div>
          ))}
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
