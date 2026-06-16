import { useEffect, useState, useCallback } from "react";
import { Settings, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { settingsApi } from "@/lib/api";
import { proxyApi } from "@/lib/api/proxy";
import { usageApi } from "@/lib/api/usage";
import { ofoxGetUserInfo, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import {
  TOOL_META,
  TOOL_ORDER,
  PROXY_SUPPORTED_TOOLS,
} from "@/config/toolMeta";
// NOTE: ConsolePage previously rendered a read-only "已锁定" badge for tools
// in `boundTools ∩ PROXY_SUPPORTED_TOOLS`, sourced from
// `useLockedTakeoverTools`. The badge was retired once the manage-tool dialog
// landed: every bound tool now exposes the same "管理" entry (model edit +
// 解除绑定), and 解除绑定 is the explicit way to turn takeover off. The hook
// itself is still imported by ProxyToggle / ProxyPanel to hide the on/off
// toggle row for those tools.
import AddToolsDialog from "./AddToolsDialog";
import ManageToolDialog, { type ManageToolTarget } from "./ManageToolDialog";
import OfoxSettingsDialog from "./OfoxSettingsDialog";
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
  /** Whether the proxy can intercept this tool. Drives the right-side cell:
   *  unsupported tools render "不支持统计" instead of a token count. */
  proxySupported: boolean;
  /** Right-side cell value when `proxySupported` is true. Pre-formatted (e.g.
   *  "1.2M") or null when the fetch returned nothing. Ignored otherwise. */
  monthTokens: string | null;
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

/**
 * Per-tool month-to-date token total for the right-side cell of "绑定的工具".
 *
 * Sums all four token buckets (input + output + cache_creation + cache_read).
 * Cache reads are included because they still count toward the user's monthly
 * usage on OFox's billing side — omitting them would understate burn for
 * cache-heavy workloads. Returns "—" for zero so empty months read cleanly.
 */
function formatMonthTokens(s: UsageSummary): string {
  const total =
    s.totalInputTokens +
    s.totalOutputTokens +
    s.totalCacheCreationTokens +
    s.totalCacheReadTokens;
  if (total === 0) return "—";
  if (total >= 1_000_000) return `${(total / 1_000_000).toFixed(1)}M`;
  if (total >= 1_000) return `${(total / 1_000).toFixed(1)}K`;
  return String(total);
}

interface ConsolePageProps {
  boundTools: string[];
  /** Called after the user adds new tools through "+ 添加" so MainApp can
   *  re-read localStorage and re-fire its takeover reconciliation sweep. */
  onBoundToolsChanged?: () => void;
}

export default function ConsolePage({
  boundTools,
  onBoundToolsChanged,
}: ConsolePageProps) {
  const [tools, setTools] = useState<BoundTool[]>([]);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  // null = manage dialog is closed. Holds the snapshot of the tool row that
  // was clicked, so the dialog stays consistent even if `tools` reloads
  // mid-edit (e.g. monthly token refetch).
  const [manageTool, setManageTool] = useState<ManageToolTarget | null>(null);
  const [user, setUser] = useState<OfoxUserInfo | null>(null);
  const [loading, setLoading] = useState(true);
  /** Standalone spinner for the manual balance refresh button. */
  const [balanceRefreshing, setBalanceRefreshing] = useState(false);

  /**
   * Refetch *only* the user/wallet snapshot. Decoupled from `loadData` so the
   * 🔄 button next to the balance doesn't also re-scan tools/usage stats.
   *
   * Surfaces an explicit toast on completion so the user knows the network
   * round-trip actually happened — without it the only feedback is the
   * spinner stopping, which is too easy to miss when the number didn't move.
   */
  const refreshBalance = useCallback(async () => {
    setBalanceRefreshing(true);
    try {
      const fresh = await ofoxGetUserInfo();
      setUser(fresh);
      const value = fresh?.balance?.balance;
      if (typeof value === "number") {
        toast.success(`余额已更新：$${value.toFixed(2)}`);
      } else {
        // Auth was fine but the balance endpoint returned no number.
        toast.warning("余额已刷新，但暂无可用数据");
      }
    } catch (e) {
      console.error("[ConsolePage] balance refresh failed", e);
      toast.error("余额刷新失败，请稍后重试");
    } finally {
      setBalanceRefreshing(false);
    }
  }, []);

  // Usage stats
  const [todayCost, setTodayCost] = useState("$0.00");
  const [todayRequests, setTodayRequests] = useState("0");
  const [todayTokens, setTodayTokens] = useState("0");
  const [monthCost, setMonthCost] = useState("$0.00");

  const loadData = useCallback(async () => {
    setLoading(true);

    // Two-stage load — the bound-tools list is the user's primary focus and
    // must not wait on the OFox profile/balance round-trip (which can be
    // multi-second under flaky network) or on the unused-but-fetched stat
    // cards. Stage 1 covers everything backed by local SQL (tool detection,
    // takeover status, per-tool month tokens) and flips loading→false.
    // Stage 2 covers OFox profile/balance and the top-card summaries; it
    // runs in the background and updates the header when ready. A failure
    // in stage 2 leaves the list intact and the header in its placeholder
    // state — never spins forever.

    // ===== Stage 1: local-only, blocks list render =====
    const [toolResults, takeoverStatus] = await Promise.all([
      // includeLatest=false: skip the npm/GitHub fetch — we only need the
      // local "is it installed?" check here. That fetch was the main reason
      // this list took ~5s to render.
      invoke<ToolInfo[]>("get_tool_versions", {
        tools: null,
        wslShellByTool: null,
        includeLatest: false,
      }).catch(() => [] as ToolInfo[]),
      proxyApi
        .getProxyTakeoverStatus()
        .catch(() => ({}) as Record<string, boolean>),
    ]);

    const detectedMap = new Map<string, ToolInfo>();
    for (const r of toolResults) {
      detectedMap.set(r.name, r);
    }

    const takeoverMap = takeoverStatus as Record<string, boolean>;

    // Per-tool month-to-date token total — shown in the right-side cell.
    // Only meaningful for tools whose requests flow through cc-switch's proxy
    // (the local `proxy_request_logs` table is keyed by `app_type` and only
    // populated for intercepted apps). For tools the proxy can't intercept
    // (e.g. OpenCode — see `services/proxy.rs:286-287` hard-coding false),
    // skip the fetch and render an explicit "不支持统计" later so we don't
    // misrepresent a 0-row aggregation as "no usage".
    const toolMonthTokens = await Promise.all(
      boundTools.map(async (id) => {
        if (!PROXY_SUPPORTED_TOOLS.includes(id)) {
          return [id, null] as [string, string | null];
        }
        try {
          const s = await getMonthSummary(id);
          return [id, s ? formatMonthTokens(s) : null] as [
            string,
            string | null,
          ];
        } catch {
          return [id, null] as [string, string | null];
        }
      }),
    );
    const monthTokensMap = new Map(toolMonthTokens);

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
        // "已接管" instead of "运行中": the proxy intercepts the tool's
        // requests, but that doesn't imply the CLI itself is actively
        // running right now — a user installing Gemini CLI v0.42.0 but not
        // using it shouldn't see a "running" badge just because we hooked it.
        statusText = "已接管";
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
        proxySupported,
        monthTokens: monthTokensMap.get(id) ?? null,
      };
    });

    setTools(list);
    setLoading(false);

    // ===== Stage 2: remote / non-blocking =====
    // Fire-and-forget — these update the header / stat cards once the
    // network round-trips finish. Errors are swallowed at each call site so
    // the list (already on-screen) doesn't get pulled back into a loading
    // state by a stale `await` further down.
    void (async () => {
      const [todaySummary, monthSummary, userInfo] = await Promise.all([
        getTodaySummary(),
        getMonthSummary(),
        ofoxGetUserInfo().catch(() => null),
      ]);

      if (userInfo) setUser(userInfo);

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
    })();
  }, [boundTools]);

  useEffect(() => {
    // `loadData` fires both the local stage and the background stage-2
    // updates. We don't have a cancel handle into the latter (it's a
    // fire-and-forget IIFE inside `loadData` to keep loading→false on the
    // critical path), so on unmount the setters in stage 2 can fire after
    // React has dropped this component. That's harmless with React 18's
    // auto-batching but does log a dev-only warning; the cost of plumbing
    // an AbortController through every fetch isn't justified for what is
    // a couple of state updates. Leave as-is intentionally.
    loadData();
  }, [loadData]);

  const errorCount = tools.filter((t) => t.status === "error").length;

  /**
   * Stats row.
   *
   * None of these four metrics are surfaced by the OAuth OpenAPI yet:
   *   - today's cost / requests / tokens have no endpoint at all
   *   - `/openapi/orgs/me/spending-limits` looks tempting for "本月" but
   *     it tracks *configured limits* (returns 403 for members, empty for
   *     orgs without a monthly limit). Using its `used_usd` would mislead
   *     users who haven't configured a monthly cap.
   *
   * Until a real usage endpoint lands, all four cards advertise "即将推出"
   * with a muted style so they're clearly placeholders, not zeros.
   */
  void todayCost;
  void todayRequests;
  void todayTokens;
  void monthCost;

  const stats: Array<{ label: string; value: string; placeholder: boolean }> = [
    { label: "今日消耗", value: "即将推出", placeholder: true },
    { label: "今日请求", value: "即将推出", placeholder: true },
    { label: "TOKENS", value: "即将推出", placeholder: true },
    { label: "本月", value: "即将推出", placeholder: true },
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
                <div className="flex items-center justify-end gap-1.5 text-[11px] text-muted-foreground">
                  <span>OFOX 账户余额</span>
                  <button
                    onClick={refreshBalance}
                    disabled={balanceRefreshing}
                    title="刷新余额"
                    className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                  >
                    <RefreshCw
                      className={`h-3 w-3 ${
                        balanceRefreshing ? "animate-spin" : ""
                      }`}
                    />
                  </button>
                </div>
                <div className="text-2xl font-bold text-foreground">
                  {formatBalance(user?.balance?.balance)}
                </div>
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
              <span
                className={
                  stat.placeholder
                    ? "text-sm font-medium text-muted-foreground/70"
                    : "text-lg font-bold text-foreground"
                }
              >
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
                onClick={() => setAddDialogOpen(true)}
                className="rounded-md border border-border px-3 py-1 text-[12px] text-foreground hover:bg-accent"
              >
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
            <>
              {/* Column header — mirrors the row's flex layout so labels sit
                  above their respective cells. The icon column gets a blank
                  placeholder of the same `w-9` width as the abbr square; the
                  status text column gets the "工具/状态" combo label. */}
              <div className="flex items-center gap-3 border-b border-border bg-muted/30 px-4 py-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
                <div className="h-4 w-9 shrink-0" aria-hidden />
                <div className="min-w-0 flex-1">工具 / 状态</div>
                <div
                  className="w-[88px] text-right"
                  title="按自然月统计，每月 1 号 00:00 本地时间重置"
                >
                  本月 Token
                </div>
                <div className="w-[68px] text-center">操作</div>
              </div>
              {tools.map((tool) => {
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
                    {tool.proxySupported ? (
                      <div
                        className="w-[88px] text-right text-[14px] font-medium text-foreground"
                        title="本月 Token 用量"
                      >
                        {tool.monthTokens ?? "—"}
                      </div>
                    ) : (
                      // Label, not a number — render muted/smaller so it doesn't
                      // compete visually with real token totals on other rows.
                      <div
                        className="w-[88px] text-right text-[11px] text-muted-foreground"
                        title="该工具未走 cc-switch 代理，无法统计 Token 用量"
                      >
                        不支持统计
                      </div>
                    )}
                    {/* Action column — fixed width (w-[68px]) matching the
                      header so labels and controls stay aligned regardless
                      of which variant renders. */}
                    {tool.status === "error" ? (
                      <button className="w-[68px] rounded-lg bg-orange-500 py-1 text-center text-[12px] font-medium text-white hover:bg-orange-600">
                        修复
                      </button>
                    ) : (
                      <button
                        onClick={() =>
                          setManageTool({
                            id: tool.id,
                            abbr: tool.abbr,
                            label: tool.label,
                            color: tool.color,
                            version: tool.version,
                            statusText: tool.statusText,
                          })
                        }
                        className="w-[68px] rounded-lg py-1 text-center text-[12px] font-medium text-muted-foreground hover:bg-accent"
                      >
                        管理
                      </button>
                    )}
                  </div>
                );
              })}
            </>
          )}
        </div>
      </div>

      {/* Bottom Bar */}
      <div className="flex shrink-0 items-center justify-between border-t border-border px-5 py-2.5">
        <button
          onClick={() => setSettingsDialogOpen(true)}
          className="flex items-center gap-1.5 text-[12px] text-muted-foreground hover:text-foreground"
        >
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

      <AddToolsDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        alreadyBound={boundTools}
        onAdded={() => {
          // localStorage is already updated by bindTools(); notify the
          // parent so MainApp's `boundTools` state re-reads and the
          // reconciliation useEffect re-fires for the newly-bound tools.
          // ConsolePage itself doesn't need a manual reload: when
          // MainApp updates `boundTools`, our `loadData` reruns via the
          // `[boundTools]` dep already on the useEffect at line 211.
          onBoundToolsChanged?.();
        }}
      />

      <ManageToolDialog
        tool={manageTool}
        onOpenChange={(open) => {
          if (!open) setManageTool(null);
        }}
        onChanged={() => {
          // Single trigger: bubble up to MainApp so it re-reads the
          // bound-tools localStorage. The new boundTools prop flows
          // back into our `loadData` (its useCallback deps include
          // boundTools), so the row list re-renders without us
          // having to call loadData() here.
          //
          // Calling loadData() inline used to "double up" the refresh,
          // but the manual call captured a stale `boundTools` closure
          // — for unbind, the in-flight stale loadData would race the
          // prop-driven one and could win, leaving the just-unbound
          // tool stuck in the list until the user bounced through
          // 管理→解绑 a second time. The single prop-driven path is
          // race-free.
          onBoundToolsChanged?.();
        }}
      />

      <OfoxSettingsDialog
        open={settingsDialogOpen}
        onOpenChange={setSettingsDialogOpen}
      />
    </div>
  );
}

// ==================== Helpers ====================

/**
 * Render the wallet figure. `null`/`undefined` collapses to `—` so the
 * layout doesn't shift between "loading" and "no balance returned".
 */
function formatBalance(value: number | null | undefined): string {
  if (typeof value !== "number" || Number.isNaN(value)) return "—";
  return `$${value.toFixed(2)}`;
}

async function getTodaySummary(appType?: string): Promise<UsageSummary | null> {
  try {
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    return await usageApi.getUsageSummary(today.getTime(), undefined, appType);
  } catch {
    return null;
  }
}

async function getMonthSummary(appType?: string): Promise<UsageSummary | null> {
  try {
    const month = new Date();
    month.setDate(1);
    month.setHours(0, 0, 0, 0);
    return await usageApi.getUsageSummary(month.getTime(), undefined, appType);
  } catch {
    return null;
  }
}
