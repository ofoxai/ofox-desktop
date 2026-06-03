import { useEffect, useState, useCallback } from "react";
import { Settings } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { settingsApi } from "@/lib/api";
import { proxyApi } from "@/lib/api/proxy";
import { usageApi } from "@/lib/api/usage";
import { ofoxGetUserInfo, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import { TOOL_META, TOOL_ORDER, PROXY_SUPPORTED_TOOLS } from "@/config/toolMeta";
import type { UsageSummary } from "@/types/usage";

interface ToolInfo {
  name: string;
  version: string | null;
  error: string | null;
}

type ToolStatus = "active" | "idle" | "error";

interface BoundTool {
  id: string;
  abbr: string;
  label: string;
  color: string;
  version: string | null;
  status: ToolStatus;
  statusText: string;
  cost: string | null;
}

const STATUS_STYLES: Record<ToolStatus, { dot: string }> = {
  active: { dot: "bg-green-500" },
  idle: { dot: "bg-gray-400" },
  error: { dot: "bg-red-500" },
};

function formatCost(cost: string | undefined): string {
  if (!cost) return "$0.00";
  const n = parseFloat(cost);
  if (isNaN(n) || n === 0) return "$0.00";
  return `$${n.toFixed(2)}`;
}

function formatTokens(input: number, output: number): string {
  const total = input + output;
  if (total >= 1_000_000) return `${(total / 1_000_000).toFixed(1)}M`;
  if (total >= 1_000) return `${(total / 1_000).toFixed(1)}K`;
  return String(total);
}

interface ConsolePageProps {
  boundTools: string[];
}

export default function ConsolePage({ boundTools }: ConsolePageProps) {
  const [tools, setTools] = useState<BoundTool[]>([]);
  const [user, setUser] = useState<OfoxUserInfo | null>(null);
  const [loading, setLoading] = useState(true);

  // Usage stats
  const [todayCost, setTodayCost] = useState("$0.00");
  const [todayRequests, setTodayRequests] = useState("0");
  const [todayTokens, setTodayTokens] = useState("0");
  const [monthCost, setMonthCost] = useState("$0.00");

  const loadData = useCallback(async () => {
    setLoading(true);

    // Parallel: tool detection + proxy status + usage stats + user info
    const [toolResults, takeoverStatus, todaySummary, monthSummary, userInfo] =
      await Promise.all([
        invoke<ToolInfo[]>("get_tool_versions", {
          tools: null,
          wslShellByTool: null,
        }).catch(() => [] as ToolInfo[]),
        proxyApi.getProxyTakeoverStatus().catch(() => ({}) as Record<string, boolean>),
        getTodaySummary(),
        getMonthSummary(),
        ofoxGetUserInfo().catch(() => null),
      ]);

    setUser(userInfo);

    // Update stats
    if (todaySummary) {
      setTodayCost(formatCost(todaySummary.totalCost));
      setTodayRequests(String(todaySummary.totalRequests));
      setTodayTokens(
        formatTokens(
          todaySummary.totalInputTokens,
          todaySummary.totalOutputTokens,
        ),
      );
    }
    if (monthSummary) {
      setMonthCost(formatCost(monthSummary.totalCost));
    }

    // Build tool list
    const detectedMap = new Map<string, ToolInfo>();
    for (const r of toolResults) {
      detectedMap.set(r.name, r);
    }

    const takeoverMap = takeoverStatus as Record<string, boolean>;

    // Per-tool today cost
    const toolCosts = await Promise.all(
      boundTools.map(async (id) => {
        try {
          const s = await getTodaySummary(id);
          return [id, s ? formatCost(s.totalCost) : null] as [string, string | null];
        } catch {
          return [id, null] as [string, string | null];
        }
      }),
    );
    const costMap = new Map(toolCosts);

    const ordered = TOOL_ORDER.filter((id) => boundTools.includes(id));
    // Also include any bound tools not in TOOL_ORDER
    for (const id of boundTools) {
      if (!ordered.includes(id)) ordered.push(id);
    }

    const list: BoundTool[] = ordered.map((id) => {
      const meta = TOOL_META[id] ?? {
        abbr: id.substring(0, 2).toUpperCase(),
        label: id,
        color: "bg-gray-500",
      };
      const info = detectedMap.get(id);
      const detected = !!info && !!info.version && !info.error;
      const proxied = !!takeoverMap[id];
      const proxySupported = PROXY_SUPPORTED_TOOLS.includes(id);

      let status: ToolStatus;
      let statusText: string;

      if (proxied && detected) {
        status = "active";
        statusText = "运行中";
      } else if (proxied && !detected) {
        status = "error";
        statusText = "未检测到";
      } else if (!proxySupported && detected) {
        status = "idle";
        statusText = "暂不支持代理统计";
      } else if (detected) {
        status = "idle";
        statusText = "未开启代理";
      } else {
        status = "error";
        statusText = "未检测到";
      }

      return {
        id,
        abbr: meta.abbr,
        label: meta.label,
        color: meta.color,
        version: info?.version ?? null,
        status,
        statusText,
        cost: costMap.get(id) ?? null,
      };
    });

    setTools(list);
    setLoading(false);
  }, [boundTools]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const errorCount = tools.filter((t) => t.status === "error").length;

  const stats = [
    { label: "今日消耗", value: todayCost },
    { label: "今日请求", value: todayRequests },
    { label: "TOKENS", value: todayTokens },
    { label: "本月", value: monthCost },
  ];

  return (
    <div className="flex h-screen w-full flex-col bg-gradient-to-br from-orange-50/50 via-white to-orange-50/30 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      {/* Title bar drag region */}
      <div className="h-7 shrink-0" data-tauri-drag-region="true">
        <div className="flex h-full items-center justify-center">
          <span className="text-[13px] font-medium text-muted-foreground">
            Ofox
          </span>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-5 pb-4">
        {/* Profile + Balance Card */}
        <div className="mb-4 rounded-xl border border-border bg-background/80 px-5 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-full bg-purple-200 text-lg font-bold text-purple-700">
                {user?.name?.charAt(0) ?? "U"}
              </div>
              <div>
                <div className="text-[14px] font-semibold text-foreground">
                  {user?.name ?? "用户"}
                </div>
                <div className="text-[12px] text-muted-foreground">
                  {user?.email ?? ""}
                </div>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="text-right">
                <div className="text-[11px] text-muted-foreground">
                  OFOX 账户余额
                </div>
                <div className="text-2xl font-bold text-foreground">—</div>
              </div>
              <button
                onClick={() =>
                  settingsApi.openExternal("https://app.ofox.ai/billing")
                }
                className="rounded-lg bg-orange-500 px-4 py-1.5 text-[13px] font-medium text-white hover:bg-orange-600"
              >
                充值
              </button>
            </div>
          </div>
        </div>

        {/* Stats Row */}
        <div className="mb-4 grid grid-cols-4 gap-3">
          {stats.map((stat) => (
            <div
              key={stat.label}
              className="flex flex-col items-start rounded-xl border border-border bg-background/80 px-4 py-3"
            >
              <span className="text-[11px] text-muted-foreground">
                {stat.label}
              </span>
              <span className="text-lg font-bold text-foreground">
                {stat.value}
              </span>
            </div>
          ))}
        </div>

        {/* Tool List */}
        <div className="rounded-xl border border-border bg-background/80">
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <div className="flex items-center gap-2">
              <span className="text-[14px] font-semibold text-foreground">
                绑定的工具
              </span>
              <span className="rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                {tools.length}
              </span>
              {errorCount > 0 && (
                <span className="rounded-md bg-red-50 px-2 py-0.5 text-[11px] font-medium text-red-500 dark:bg-red-950/50">
                  {errorCount} 个需修复
                </span>
              )}
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={loadData}
                className="rounded-md p-1.5 text-muted-foreground hover:bg-accent"
                title="刷新"
              >
                <Settings className="h-4 w-4" />
              </button>
              <button className="rounded-md border border-border px-3 py-1 text-[12px] text-foreground hover:bg-accent">
                + 添加
              </button>
            </div>
          </div>

          {loading ? (
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              正在加载...
            </div>
          ) : tools.length === 0 ? (
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              暂无绑定的工具
            </div>
          ) : (
            tools.map((tool) => {
              const style = STATUS_STYLES[tool.status];
              return (
                <div
                  key={tool.id}
                  className="flex items-center gap-3 border-b border-border px-4 py-3 last:border-b-0"
                >
                  <div
                    className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[11px] font-bold text-white ${tool.color}`}
                  >
                    {tool.abbr}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] font-medium text-foreground">
                      {tool.label}
                    </div>
                    <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                      <span
                        className={`h-1.5 w-1.5 rounded-full ${style.dot}`}
                      />
                      {tool.statusText}
                      {tool.version && ` · v${tool.version}`}
                    </div>
                  </div>
                  <div className="text-[14px] font-medium text-foreground">
                    {tool.cost && tool.cost !== "$0.00" ? tool.cost : "—"}
                  </div>
                  <button
                    className={`rounded-lg px-3 py-1 text-[12px] font-medium ${
                      tool.status === "error"
                        ? "bg-orange-500 text-white hover:bg-orange-600"
                        : "text-muted-foreground hover:bg-accent"
                    }`}
                  >
                    {tool.status === "error" ? "修复" : "管理"}
                  </button>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* Bottom Bar */}
      <div className="flex shrink-0 items-center justify-between border-t border-border px-5 py-2.5">
        <button className="flex items-center gap-1.5 text-[12px] text-muted-foreground hover:text-foreground">
          <Settings className="h-3.5 w-3.5" />
          设置
        </button>
        <div className="flex items-center gap-4">
          <button
            onClick={() =>
              settingsApi.openExternal("https://app.ofox.ai/dashboard")
            }
            className="text-[12px] text-orange-500 hover:text-orange-600 hover:underline"
          >
            查看详细用量 ↗
          </button>
          <button
            onClick={() => settingsApi.openExternal("https://ofox.ai")}
            className="text-[12px] text-orange-500 hover:text-orange-600 hover:underline"
          >
            Ofox 网站 ↗
          </button>
        </div>
      </div>
    </div>
  );
}

// ==================== Helpers ====================

async function getTodaySummary(
  appType?: string,
): Promise<UsageSummary | null> {
  try {
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    return await usageApi.getUsageSummary(
      today.getTime(),
      undefined,
      appType,
    );
  } catch {
    return null;
  }
}

async function getMonthSummary(): Promise<UsageSummary | null> {
  try {
    const month = new Date();
    month.setDate(1);
    month.setHours(0, 0, 0, 0);
    return await usageApi.getUsageSummary(month.getTime());
  } catch {
    return null;
  }
}
