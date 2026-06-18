import { useEffect, useState } from "react";

import { usageApi } from "@/lib/api/usage";
import type { UsageSummary } from "@/types/usage";
import type { ToolActivenessMap } from "./ToolStatusList";
import { PROXY_SUPPORTED_TOOLS } from "@/config/toolMeta";

interface StatsRowProps {
  boundTools: string[];
  activeness: ToolActivenessMap;
}

interface AggregatedToday {
  totalRequests: number;
  totalTokens: number;
}

/** Unix-seconds since today 00:00 local. usage API expects seconds. */
function todayStartSec(): number {
  const t = new Date();
  t.setHours(0, 0, 0, 0);
  return Math.floor(t.getTime() / 1000);
}

/** 千分位。 */
function formatThousand(n: number): string {
  return n.toLocaleString("en-US");
}

/** 紧凑写法：1.2M / 22.2K。与主窗口卡片视觉一致。 */
function formatCompact(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0";
  if (n >= 1_000_000) {
    return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  }
  if (n >= 1_000) {
    return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}K`;
  }
  return formatThousand(n);
}

function sumTokens(s: UsageSummary): number {
  return (
    s.totalInputTokens +
    s.totalOutputTokens +
    s.totalCacheCreationTokens +
    s.totalCacheReadTokens
  );
}

export default function StatsRow({ boundTools, activeness }: StatsRowProps) {
  const [aggregated, setAggregated] = useState<AggregatedToday | null>(null);

  useEffect(() => {
    let cancelled = false;

    // 与主窗口"今日总请求 / 今日总 TOKENS"卡片完全一致：
    // 只对 boundTools ∩ PROXY_SUPPORTED_TOOLS 调一次 per-tool 的
    // getUsageSummary，再求和。这样未绑定的历史 / 不支持代理统计的工具
    // 都不会被算进来，跟主页的"绑定的工具"列表合计对得上。
    const supported = boundTools.filter((id) =>
      PROXY_SUPPORTED_TOOLS.includes(id),
    );

    const load = async () => {
      // 没有任何 supported 绑定 → 直接 0/0/0，不发请求
      if (supported.length === 0) {
        if (!cancelled) {
          setAggregated({ totalRequests: 0, totalTokens: 0 });
        }
        return;
      }

      try {
        const start = todayStartSec();
        const summaries = await Promise.all(
          supported.map((id) =>
            usageApi
              .getUsageSummary(start, undefined, id)
              .catch(() => null as UsageSummary | null),
          ),
        );
        let totalRequests = 0;
        let totalTokens = 0;
        for (const s of summaries) {
          if (!s) continue;
          totalRequests += s.totalRequests;
          totalTokens += sumTokens(s);
        }
        if (!cancelled) setAggregated({ totalRequests, totalTokens });
      } catch (e) {
        // 整体失败保留上次值
        console.error("[StatsRow] aggregate failed", e);
      }
    };

    void load();

    // Popover 是失焦即隐藏的窗口，每次"打开"都触发 focus，等价
    // 每次打开重拉一次。
    const onFocus = () => void load();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
    };
  }, [boundTools]);

  const total = boundTools.length;
  // "连接工具 X/Y"：X = 已接管且检测到二进制（activeness === "active"）的数量，
  // 与主窗口的"已接管 ●（绿点）"完全同语义。
  const ok = boundTools.filter((id) => activeness[id] === "active").length;

  const stats = [
    {
      label: "今日请求",
      value: aggregated ? formatThousand(aggregated.totalRequests) : "—",
    },
    {
      label: "TOKENS",
      value: aggregated ? formatCompact(aggregated.totalTokens) : "—",
    },
    {
      label: "连接工具",
      value: total === 0 ? "0/0" : `${ok}/${total}`,
    },
  ];

  return (
    <div className="flex border-t border-border">
      {stats.map((stat, i) => (
        <div
          key={stat.label}
          className={`flex flex-1 flex-col items-center py-2 ${
            i < stats.length - 1 ? "border-r border-border" : ""
          }`}
        >
          <div className="text-[10px] text-muted-foreground">{stat.label}</div>
          <div className="text-[15px] font-bold text-foreground">
            {stat.value}
          </div>
        </div>
      ))}
    </div>
  );
}
