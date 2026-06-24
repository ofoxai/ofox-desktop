import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { Settings, RefreshCw, Loader2, HelpCircle } from "lucide-react";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";
import { proxyApi } from "@/lib/api/proxy";
import { usageApi } from "@/lib/api/usage";
import { ofoxGetUserInfo, isOfoxBillingManager, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import {
  ofoxDashboardUrl,
  ofoxMarketingUrl,
  ofoxWalletUrl,
  type OfoxApex,
} from "@/lib/ofoxUrls";
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
import { OfoxApexSwitch } from "@/components/OfoxApexSwitch";
import { UserAvatar } from "@/components/UserAvatar";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useToolHealth } from "@/hooks/useToolHealth";
import type { ToolHealthSnapshot } from "@/lib/api/toolHealth";
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
  /** Today's usage triple. Pre-formatted strings, ready to render. `null`
   *  means the fetch failed or returned nothing — fall back to "—" rendering.
   *  Unsupported tools (proxySupported=false) get `null` here too and render
   *  "不支持统计" in the dedicated UI branch. */
  todayUsage: {
    requests: string;
    tokens: string;
    cost: string;
  } | null;
  /** Raw numeric snapshot of today + month, used by the top-of-page summary
   *  cards. Kept separate from the pre-formatted `todayUsage` so the cards
   *  can sum across tools without re-parsing strings. `null` for unsupported
   *  or failed fetches — the card aggregator then skips this tool to avoid
   *  silently understating totals as zero. */
  rawStats: {
    todayRequests: number;
    todayTokens: number;
    todayCostUsd: number;
    monthCostUsd: number;
  } | null;
}

const STATUS_STYLES: Record<ToolStatus, { dot: string }> = {
  active: { dot: "bg-green-500" },
  idle: { dot: "bg-gray-400" },
  error: { dot: "bg-red-500" },
};

/**
 * True when the user object came back from the backend with at least one
 * identity field populated. False for `null` AND for the all-null
 * placeholder the backend returns when `/openapi/me` was unreachable
 * during the login flow (see `ofox_auth.rs::poll_for_token`).
 *
 * We treat email or name presence as the signal — `org_id` alone wouldn't
 * give the header anything useful to render.
 */
function isMeaningfulUser(u: OfoxUserInfo | null | undefined): boolean {
  if (!u) return false;
  return Boolean(u.email || u.name);
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

/**
 * Render today's usage triple for the bound-tools list's "今日数据" column.
 *
 * Returned strings are display-ready and locale-formatted. Three reasons
 * this lives outside the row JSX:
 * - The same shape is reused for the manual refresh path, where a fresh
 *   `UsageSummary` lands without a re-render of the row's cell helpers.
 * - Callers (the row map) can keep the column conditional logic minimal:
 *   `null` → "不支持统计" branch; non-null → render `triple` directly.
 * - Cost formatting needs the same "<$0.0001 / 4-digit / 2-digit" cliff
 *   the ManageToolDialog uses so小钱不被四舍五入成 $0.00。
 *
 * Tokens here include all four buckets to match `formatMonthTokens` and
 * the per-tool dialog stat card — billing-equivalent surfaces should
 * count the same things.
 */
function formatTodayUsage(s: UsageSummary): {
  requests: string;
  tokens: string;
  cost: string;
} {
  const tokenTotal =
    s.totalInputTokens +
    s.totalOutputTokens +
    s.totalCacheCreationTokens +
    s.totalCacheReadTokens;
  const tokens =
    tokenTotal === 0
      ? "—"
      : tokenTotal >= 1_000_000
        ? `${(tokenTotal / 1_000_000).toFixed(1)}M`
        : tokenTotal >= 1_000
          ? `${(tokenTotal / 1_000).toFixed(1)}K`
          : String(tokenTotal);

  const n = parseFloat(s.totalCost || "0");
  let cost: string;
  if (Number.isNaN(n) || n === 0) cost = "$0.00";
  else if (n < 0.0001) cost = "<$0.0001";
  else cost = `$${n.toFixed(n < 0.01 ? 4 : 2)}`;

  return {
    requests: String(s.totalRequests),
    tokens,
    cost,
  };
}

/**
 * Format a USD amount for the top-of-page summary cards.
 *
 * Same精度 cliffs as `formatTodayUsage.cost` and `ManageToolDialog.renderStatCell`,
 * since these surfaces all show the same kind of small-dollar usage:
 *   - 0 → "$0.00" (the formatter NEVER returns "—" — that's the placeholder
 *         path's job, kept separate so a tool genuinely with $0 today reads
 *         as "$0.00" not as "no data")
 *   - <0.0001 → "<$0.0001"
 *   - <0.01   → 4-digit
 *   - else    → 2-digit
 */
function formatUsd(amount: number): string {
  if (!Number.isFinite(amount) || amount === 0) return "$0.00";
  if (amount < 0.0001) return "<$0.0001";
  return `$${amount.toFixed(amount < 0.01 ? 4 : 2)}`;
}

/**
 * Format a raw token count for the TOKENS summary card.
 *
 * Aligned with the per-row "本月 Token" cell and the dialog's TOKENS card —
 * `1.2M` / `22.2K` / raw, no zero-suppression (the card always renders
 * something; the placeholder branch handles the "no data" case upstream).
 */
function formatTokenCount(total: number): string {
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
  const { apex } = useOfoxApex();
  const [tools, setTools] = useState<BoundTool[]>([]);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);
  // null = manage dialog is closed. Holds the snapshot of the tool row that
  // was clicked, so the dialog stays consistent even if `tools` reloads
  // mid-edit (e.g. monthly token refetch).
  const [manageTool, setManageTool] = useState<ManageToolTarget | null>(null);
  const [user, setUser] = useState<OfoxUserInfo | null>(null);
  // True while the post-mount retry loop is actively probing because the
  // initial /openapi/me came back empty (backend's `poll_for_token` now
  // returns a placeholder user when the API is unreachable, instead of
  // hard-failing — see `ofox_auth.rs`). The loop tries 3 times with
  // 1s/3s/8s backoff (total ~12s) so the user doesn't sit through the
  // earlier 22s window before seeing a definitive failure state.
  const [userRetrying, setUserRetrying] = useState(false);
  // Which retry attempt we're currently on, for the "重试中 (1/3)" hint.
  // 0 means "not retrying"; 1..3 maps to the three backoff tiers.
  const [userRetryAttempt, setUserRetryAttempt] = useState(0);
  // Set to true after the retry loop exhausts without ever seeing a real
  // user. Surfaces a manual "重新加载" link in the header so the user has
  // an explicit recovery path when the OFox backend was unreachable at
  // login time but came back later.
  const [userLoadFailed, setUserLoadFailed] = useState(false);
  const [loading, setLoading] = useState(true);
  /** Standalone spinner for the manual balance refresh button. */
  const [balanceRefreshing, setBalanceRefreshing] = useState(false);
  /** Standalone spinner for the manual "绑定的工具" usage refresh button. */
  const [usageRefreshing, setUsageRefreshing] = useState(false);

  // 监听 tray popover 发出的"打开偏好设置"事件，把当前 dialog 拉起来。
  // popover 是独立 webview，无法直接调用本组件的 setState，因此走 Tauri
  // event 通信。channel 名与 BottomMenu.tsx 里的 emit 必须一致。
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const off = await listen("ofox-open-settings", () => {
          setSettingsDialogOpen(true);
        });
        if (cancelled) {
          off();
        } else {
          unlisten = off;
        }
      } catch (e) {
        console.error("[ConsolePage] listen ofox-open-settings failed", e);
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

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

  // Usage stats — aggregated client-side from `tools[].rawStats` via the
  // `stats` useMemo below. `tools` is the loadData output, so refreshing
  // it refreshes the top cards automatically; no separate fetch needed.
  // The 4th card ("本月") surfaces month-to-date *cost* (USD) rather than
  // tokens — the per-row "本月 Token" column already shows token sums.

  // Tool health pills — backend probes each bound tool every 1h/6h/24h
  // (configurable in settings) and emits `ofox-tool-health-updated`. We
  // render a colored dot + text in the tool card status row.
  const { snapshot: healthSnapshot } = useToolHealth();

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
          return [id, { tokens: null, costUsd: null }] as [
            string,
            { tokens: string | null; costUsd: number | null },
          ];
        }
        try {
          const s = await getMonthSummary(id);
          if (!s) {
            return [id, { tokens: null, costUsd: null }] as [
              string,
              { tokens: string | null; costUsd: number | null },
            ];
          }
          const cost = parseFloat(s.totalCost || "0");
          return [
            id,
            {
              tokens: formatMonthTokens(s),
              costUsd: Number.isNaN(cost) ? null : cost,
            },
          ] as [
            string,
            { tokens: string | null; costUsd: number | null },
          ];
        } catch {
          return [id, { tokens: null, costUsd: null }] as [
            string,
            { tokens: string | null; costUsd: number | null },
          ];
        }
      }),
    );
    const monthMap = new Map(toolMonthTokens);

    // Per-tool today snapshot — fetched in parallel with the month query.
    // Mirror the same proxy-supported gate: unsupported tools render
    // "不支持统计" in the column and never hit the SQL aggregator. Returning
    // `null` (not a zero-summary) keeps the unsupported-vs-zero distinction
    // at the type level so the renderer can branch unambiguously.
    const toolTodayUsage = await Promise.all(
      boundTools.map(async (id) => {
        if (!PROXY_SUPPORTED_TOOLS.includes(id)) {
          return [id, { display: null, raw: null }] as [
            string,
            {
              display: ReturnType<typeof formatTodayUsage> | null;
              raw: {
                requests: number;
                tokens: number;
                costUsd: number;
              } | null;
            },
          ];
        }
        try {
          const s = await getTodaySummary(id);
          if (!s) {
            return [id, { display: null, raw: null }] as [
              string,
              {
                display: ReturnType<typeof formatTodayUsage> | null;
                raw: {
                  requests: number;
                  tokens: number;
                  costUsd: number;
                } | null;
              },
            ];
          }
          const cost = parseFloat(s.totalCost || "0");
          return [
            id,
            {
              display: formatTodayUsage(s),
              raw: {
                requests: s.totalRequests,
                tokens:
                  s.totalInputTokens +
                  s.totalOutputTokens +
                  s.totalCacheCreationTokens +
                  s.totalCacheReadTokens,
                costUsd: Number.isNaN(cost) ? 0 : cost,
              },
            },
          ] as [
            string,
            {
              display: ReturnType<typeof formatTodayUsage> | null;
              raw: {
                requests: number;
                tokens: number;
                costUsd: number;
              } | null;
            },
          ];
        } catch {
          return [id, { display: null, raw: null }] as [
            string,
            {
              display: ReturnType<typeof formatTodayUsage> | null;
              raw: {
                requests: number;
                tokens: number;
                costUsd: number;
              } | null;
            },
          ];
        }
      }),
    );
    const todayMap = new Map(toolTodayUsage);

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

      const todayEntry = todayMap.get(id);
      const monthEntry = monthMap.get(id);
      const todayRaw = todayEntry?.raw ?? null;
      const monthCost = monthEntry?.costUsd ?? null;
      // rawStats is non-null only when we actually got numbers back. We
      // intentionally keep it nullable rather than zero-fill, so the top-of-
      // page summary cards can distinguish "no data fetched" from "real zero".
      const rawStats =
        proxySupported && (todayRaw !== null || monthCost !== null)
          ? {
              todayRequests: todayRaw?.requests ?? 0,
              todayTokens: todayRaw?.tokens ?? 0,
              todayCostUsd: todayRaw?.costUsd ?? 0,
              monthCostUsd: monthCost ?? 0,
            }
          : null;

      return {
        id,
        abbr: meta.abbr,
        label: meta.label,
        color: meta.color,
        version: info?.version ?? null,
        status,
        statusText,
        proxySupported,
        monthTokens: monthEntry?.tokens ?? null,
        todayUsage: todayEntry?.display ?? null,
        rawStats,
      };
    });

    setTools(list);
    setLoading(false);

    // ===== Stage 2: remote / non-blocking =====
    // Fetch `/openapi/me` after the list paints. The 4 top summary cards
    // are now derived from `tools[].rawStats` so we no longer fetch
    // today/month aggregates here — that data already came in during
    // Stage 1 (per-tool, parallelized) and feeds the cards via useMemo.
    void (async () => {
      const userInfo = await ofoxGetUserInfo().catch(() => null);

      if (userInfo) setUser(userInfo);

      // The backend's `poll_for_token` now returns a placeholder user
      // (all-null fields) when /openapi/me was unreachable at login time
      // — that prevents the LoginPage spinner trap, but it leaves the
      // Console showing "用户" with an empty email until the user takes
      // some action. Detect the placeholder and silently retry a few
      // times with backoff before giving up. If the backend was just
      // having a temporary blip (which is the common case), the user
      // never sees the empty state at all.
      if (!isMeaningfulUser(userInfo)) {
        void retryUserInfoWithBackoff();
      }
    })();
  }, [boundTools]);

  /**
   * Retry `/openapi/me` up to 3 times with 2s/5s/15s backoff. Bails as soon
   * as a meaningful user comes back (anything with email or name set).
   *
   * Sets `userRetrying` while a probe is in flight so the header can show
   * a "正在加载用户信息…" hint instead of the empty-state ghost. After all
   * 3 attempts return placeholders, sets `userLoadFailed` so the user
   * gets an explicit "重新加载" button rather than living with the silent
   * empty card.
   *
   * Safe to call multiple times — caller already guarded with the
   * isMeaningfulUser check, and the backoff is bounded.
   */
  const retryUserInfoWithBackoff = useCallback(async () => {
    setUserLoadFailed(false);
    setUserRetrying(true);
    setUserRetryAttempt(0);
    // 1s/3s/8s — total ~12s. Picked over the original 2s/5s/15s because
    // 22s with no UI progress feels broken; 12s is short enough that the
    // user is still expecting a result and long enough that genuine
    // backend recovery (a few hundred ms after a glitch) lands inside.
    const delays = [1000, 3000, 8000];
    try {
      for (let i = 0; i < delays.length; i++) {
        setUserRetryAttempt(i + 1);
        await new Promise((r) => setTimeout(r, delays[i]));
        const fresh = await ofoxGetUserInfo().catch(() => null);
        if (isMeaningfulUser(fresh)) {
          setUser(fresh);
          return;
        }
      }
      setUserLoadFailed(true);
    } finally {
      setUserRetrying(false);
      setUserRetryAttempt(0);
    }
  }, []);

  /**
   * Force-refresh the bound-tools row stats.
   *
   * Two steps:
   *   1) Sync session usage logs from disk (~/.claude/projects, ~/.codex/sessions,
   *      ~/.gemini/tmp/...) into proxy_request_logs. Normally this runs every
   *      60s in the background (lib.rs:1147), but the user clicking refresh
   *      shouldn't have to wait that long after sending a CLI message.
   *   2) Re-run loadData(): re-fetches takeover status, tool versions, and
   *      per-tool monthly token sums. This is what actually re-paints the row.
   *
   * Decoupled from loadData() because the sync step is the slow path
   * (filesystem walk) and we want a separate spinner so the existing rows
   * stay visible mid-refresh instead of bouncing to the loading skeleton.
   */
  const refreshUsage = useCallback(async () => {
    setUsageRefreshing(true);
    try {
      const result = await usageApi.syncSessionUsage();
      await loadData();
      if (result.imported > 0) {
        toast.success(`统计已刷新 · 同步 ${result.imported} 条新会话`);
      } else {
        toast.success("统计已刷新");
      }
    } catch (e) {
      console.error("[ConsolePage] usage refresh failed", e);
      toast.error(`刷新失败：${String(e)}`);
    } finally {
      setUsageRefreshing(false);
    }
  }, [loadData]);

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
   * Stats row — top-of-page summary cards.
   *
   * Aggregated from the bound-tools list, NOT from a separate API endpoint.
   * Each card sums the matching field across `tools[].rawStats` for proxy-
   * supported tools where the fetch returned data. Tools without rawStats
   * (unsupported, or fetch failed) are skipped — the card never claims `0`
   * when the truth is "we couldn't see this tool".
   *
   * If NONE of the bound tools provided rawStats (e.g. all are unsupported,
   * or first paint hasn't completed), each card falls back to "计算中"
   * placeholder text so we don't show misleading zeros mid-load.
   *
   * Card semantics:
   *   - 今日总消费    : sum(todayCostUsd)        → "$0.0007" / "<$0.0001"
   *   - 今日总请求    : sum(todayRequests)       → "66"
   *   - 今日总 TOKENS : sum(todayTokens)         → "22.2K"  (today, not month —
   *                     the per-row "本月 Token" column is the month surface)
   *   - 本月总消费    : sum(monthCostUsd)        → "$43.25" / "<$0.0001"
   *
   * Scope note: "本月总消费" sums only across **bound** tools (those listed
   * in `boundTools`). A user who chats with claude-code without binding it
   * to cc-switch won't see those costs reflected here — that's intentional;
   * the card mirrors what the table below shows. To see whole-account
   * monthly spend regardless of binding, point users to "查看详细用量 ↗".
   */
  const stats = useMemo(() => {
    const supported = tools.filter((t) => t.rawStats !== null);
    if (supported.length === 0) {
      return [
        { label: "今日总消费", value: "计算中", placeholder: true },
        { label: "今日总请求", value: "计算中", placeholder: true },
        { label: "今日总 TOKENS", value: "计算中", placeholder: true },
        { label: "本月总消费", value: "计算中", placeholder: true },
      ];
    }
    let todayCostSum = 0;
    let todayReqSum = 0;
    let todayTokenSum = 0;
    let monthCostSum = 0;
    for (const t of supported) {
      const r = t.rawStats!;
      todayCostSum += r.todayCostUsd;
      todayReqSum += r.todayRequests;
      todayTokenSum += r.todayTokens;
      monthCostSum += r.monthCostUsd;
    }
    return [
      {
        label: "今日总消费",
        value: formatUsd(todayCostSum),
        placeholder: false,
      },
      {
        label: "今日总请求",
        value: todayReqSum.toLocaleString(),
        placeholder: false,
      },
      {
        label: "今日总 TOKENS",
        value: formatTokenCount(todayTokenSum),
        placeholder: false,
      },
      {
        label: "本月总消费",
        value: formatUsd(monthCostSum),
        placeholder: false,
      },
    ];
  }, [tools]);

  return (
    <div className="flex h-screen w-full flex-col bg-gradient-to-br from-orange-50/50 via-white to-orange-50/30 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      {/* Title bar drag region — full 40px so it covers the macOS traffic-light
          row (titleBarStyle: "Overlay") with comfortable margin on either side.
          The previous 28px (h-7) was tall enough technically but felt fiddly:
          users would aim for the visible "Ofox" text and miss. */}
      <div
        className="relative h-10 shrink-0"
        data-tauri-drag-region="true"
      >
        <div
          className="flex h-full items-center justify-center"
          data-tauri-drag-region="true"
        >
          <span className="text-[13px] font-medium text-muted-foreground">
            Ofox
          </span>
        </div>
        {/* Apex (region) switcher — 浮在右上角，与下方内容区 (px-5) 右
            对齐；垂直方向居中 (`top-1.5` = (40-28)/2 = 6px)，上下边距相等。
            `data-tauri-drag-region={false}` 阻止 select 的点击被 window-drag
            吞掉。已登录态切换会触发 ConfirmDialog → 重新登录流程。 */}
        <div
          className="absolute right-5 top-1.5 z-10"
          data-tauri-drag-region="false"
        >
          <OfoxApexSwitch triggerClassName="h-7 w-[108px] text-[12px]" />
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-5 pb-4">
        {/* Profile + Balance Card */}
        <div className="mb-4 rounded-xl border border-border bg-background/80 px-5 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <UserAvatar
                avatarUrl={user?.avatar_url}
                name={user?.name}
                email={user?.email}
                className="h-10 w-10"
                fallbackTextClassName="text-lg"
              />
              <div>
                <div className="text-[14px] font-semibold text-foreground">
                  {user?.name ?? (userRetrying ? "加载中…" : "用户")}
                </div>
                <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
                  {user?.email ? (
                    <span className="truncate">{user.email}</span>
                  ) : userRetrying ? (
                    <span className="inline-flex items-center gap-1">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      正在加载用户信息
                      {userRetryAttempt > 0
                        ? `（${userRetryAttempt}/3）`
                        : "…"}
                    </span>
                  ) : userLoadFailed ? (
                    <button
                      type="button"
                      onClick={retryUserInfoWithBackoff}
                      className="inline-flex items-center gap-1 text-orange-500 hover:text-orange-600 hover:underline"
                      title="无法加载用户信息，点击重试"
                    >
                      <RefreshCw className="h-3 w-3" />
                      无法加载用户信息，点击重试
                    </button>
                  ) : null}
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
              {isOfoxBillingManager(user) && (
                <button
                  onClick={() =>
                    settingsApi.openExternal(ofoxWalletUrl(apex))
                  }
                  className="rounded-lg bg-orange-500 px-4 py-1.5 text-[13px] font-medium text-white hover:bg-orange-600"
                >
                  充值
                </button>
              )}
            </div>
          </div>
        </div>

        {/* Stats Row — 4 张卡聚合自 `tools[].rawStats`。每张卡的 label
            旁有一个 ❓，hover 整个 label+❓ 区域时弹出统一的「供参考」
            说明：数据来源、误差来源、跳转 Ofox 官方后台。文案在 6 处
            （4 张卡 + 表格 2 个列头）共用 `<StatHelpHover>` 复用。 */}
        <div className="mb-4 grid grid-cols-4 gap-3">
          {stats.map((stat) => (
            <div
              key={stat.label}
              className="flex flex-col items-start rounded-xl border border-border bg-background/80 px-4 py-3"
            >
              <StatHelpHover apex={apex}>
                <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground">
                  {stat.label}
                  <HelpCircle className="h-3 w-3 opacity-60" />
                </span>
              </StatHelpHover>
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
                onClick={refreshUsage}
                disabled={usageRefreshing}
                title="同步本地会话日志并刷新统计"
                className="flex items-center gap-1.5 rounded-md border border-border px-3 py-1 text-[12px] text-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${usageRefreshing ? "animate-spin" : ""}`}
                />
                刷新
              </button>
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
                <div className="w-[210px] text-right">
                  <StatHelpHover apex={apex} align="end">
                    <span className="inline-flex items-center gap-1">
                      今日数据
                      <HelpCircle className="h-3 w-3 opacity-60" />
                    </span>
                  </StatHelpHover>
                </div>
                <div className="w-[88px] text-right">
                  <StatHelpHover apex={apex} align="end">
                    <span className="inline-flex items-center gap-1">
                      本月 Token
                      <HelpCircle className="h-3 w-3 opacity-60" />
                    </span>
                  </StatHelpHover>
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
                        <ToolHealthPill snapshot={healthSnapshot[tool.id]} />
                      </div>
                    </div>
                    {/* "今日数据" column — three-line stack: 请求 / Token / 金额.
                      Mirrors the right-side "本月 Token" column's supported/
                      unsupported branch so OpenCode-style tools that bypass
                      the proxy show the same "不支持统计" label here, instead
                      of three rows of zeros that would misrepresent reality. */}
                    {tool.proxySupported ? (
                      <div
                        className="w-[210px] text-right text-[11px] leading-[1.5] text-muted-foreground"
                        title="今日 0:00 起的累计数据"
                      >
                        {tool.todayUsage ? (
                          <>
                            <div>
                              请求{" "}
                              <span className="font-medium text-foreground">
                                {tool.todayUsage.requests}
                              </span>{" "}
                              次
                            </div>
                            <div>
                              消耗 token{" "}
                              <span className="font-medium text-foreground">
                                {tool.todayUsage.tokens}
                              </span>
                            </div>
                            <div>
                              消耗金额{" "}
                              <span className="font-medium text-foreground">
                                {tool.todayUsage.cost}
                              </span>
                            </div>
                          </>
                        ) : (
                          <span>—</span>
                        )}
                      </div>
                    ) : (
                      <div
                        className="w-[210px] text-right text-[11px] text-muted-foreground"
                        title="该工具未走 cc-switch 代理，无法统计今日数据"
                      >
                        不支持统计
                      </div>
                    )}
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
              settingsApi.openExternal(ofoxDashboardUrl(apex))
            }
            className="text-[12px] text-orange-500 hover:text-orange-600 hover:underline"
          >
            查看详细用量 ↗
          </button>
          <button
            onClick={() => settingsApi.openExternal(ofoxMarketingUrl(apex))}
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

// ─── Stat help hover ─────────────────────────────────────────────────────
//
// Hover-triggered Popover that wraps a stat label (e.g. "今日总消费 ❓").
// The 4 top cards and the 2 table headers (今日数据 / 本月 Token) all share
// the same explanation text — extracting this avoids 6 copies of the same
// JSX and keeps the messaging consistent.
//
// Why Popover (not Tooltip): the help text contains a clickable
// "Ofox 官方后台 ↗" link, and Radix Tooltip doesn't allow the cursor to
// move into the floating panel without dismissing it. Popover-with-hover
// supports that flow — we hold `open` open while the cursor is over either
// the trigger or the content via a small idle timer.

const STAT_HELP_OPEN_DELAY_MS = 80;
const STAT_HELP_CLOSE_DELAY_MS = 120;

function StatHelpHover({
  apex,
  children,
  align = "start",
}: {
  apex: OfoxApex;
  children: React.ReactNode;
  align?: "start" | "center" | "end";
}) {
  const [open, setOpen] = useState(false);
  // Two pending timers — one for "open after a short hover" so brushing
  // past the label doesn't flicker the popover, and one for "close after
  // leaving" so moving from trigger → content stays open. Both reset on
  // re-entry.
  const openTimer = useRef<number | null>(null);
  const closeTimer = useRef<number | null>(null);

  const cancelTimers = useCallback(() => {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, []);

  const handleEnter = useCallback(() => {
    cancelTimers();
    openTimer.current = window.setTimeout(() => {
      setOpen(true);
    }, STAT_HELP_OPEN_DELAY_MS);
  }, [cancelTimers]);

  const handleLeave = useCallback(() => {
    cancelTimers();
    closeTimer.current = window.setTimeout(() => {
      setOpen(false);
    }, STAT_HELP_CLOSE_DELAY_MS);
  }, [cancelTimers]);

  useEffect(() => () => cancelTimers(), [cancelTimers]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        asChild
        onMouseEnter={handleEnter}
        onMouseLeave={handleLeave}
      >
        <span className="inline-flex cursor-help">{children}</span>
      </PopoverTrigger>
      <PopoverContent
        align={align}
        side="bottom"
        sideOffset={6}
        onMouseEnter={handleEnter}
        onMouseLeave={handleLeave}
        // Tighter than the default popover: smaller width, lighter padding,
        // smaller text. The previous 320px-wide variant felt like a modal
        // sheet; this reads as an inline annotation.
        className="w-[280px] p-3 text-[11.5px] leading-relaxed shadow-lg"
      >
        <div className="space-y-1.5 text-muted-foreground">
          <p className="font-medium text-foreground">本页统计仅供参考</p>
          <p>
            数据来自本机捕获的 cc-switch 代理流量，以及对
            Claude / Codex / Gemini 本地会话日志的离线解析。
          </p>
          <p className="pt-0.5 font-medium text-foreground">可能的误差来源</p>
          <ul className="list-disc space-y-0.5 pl-4">
            <li>未经本工具代理的请求不在统计范围内。</li>
            <li>SSE 流式响应在部分路径下未被记录，可能导致偏低。</li>
            <li>金额按本地缓存的模型单价估算，与实际计费可能有差异。</li>
          </ul>
          <p className="pt-1">
            实际消费请以
            <button
              type="button"
              onClick={() => settingsApi.openExternal(ofoxDashboardUrl(apex))}
              className="mx-1 inline text-orange-500 hover:text-orange-600 hover:underline"
            >
              Ofox 官方后台 ↗
            </button>
            为准。
          </p>
        </div>
      </PopoverContent>
    </Popover>
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
    // usage API 期望 Unix epoch *seconds*, not milliseconds —
    // matches src/lib/usageRange.ts:21 and the SQL where-clause in
    // src-tauri/src/services/usage_stats.rs which compares against
    // proxy_request_logs.created_at (stored in seconds). Passing
    // getTime() directly silently filters out every row.
    return await usageApi.getUsageSummary(
      Math.floor(today.getTime() / 1000),
      undefined,
      appType,
    );
  } catch {
    return null;
  }
}

async function getMonthSummary(appType?: string): Promise<UsageSummary | null> {
  try {
    const month = new Date();
    month.setDate(1);
    month.setHours(0, 0, 0, 0);
    // See getTodaySummary above — usage API contract is seconds.
    return await usageApi.getUsageSummary(
      Math.floor(month.getTime() / 1000),
      undefined,
      appType,
    );
  } catch {
    return null;
  }
}

// ─── Tool-health pill ────────────────────────────────────────────────────
//
// Renders nothing until the backend has probed at least once. Colors:
//   ok      → green  + "延迟 X · Y分钟前"
//   fail    → red    + "连接失败"
//   skipped → gray   + "未配置模型"
// All three show full context on hover via `title`.

function ToolHealthPill({
  snapshot,
}: {
  snapshot: ToolHealthSnapshot | undefined;
}) {
  if (!snapshot) return null;

  const ageMin = Math.max(
    0,
    Math.floor((Date.now() - snapshot.checkedAt) / 60000),
  );
  const ageText = ageMin === 0 ? "刚刚" : `${ageMin} 分钟前`;

  if (snapshot.status === "ok") {
    return (
      <span
        className="ml-1 inline-flex items-center gap-1"
        title={`健康检查通过 (HTTP ${snapshot.statusCode ?? 200})`}
      >
        <span
          aria-hidden
          className="h-1.5 w-1.5 rounded-full bg-emerald-500"
        />
        <span>
          延迟 {snapshot.latencyMs ?? 0}ms · {ageText}
        </span>
      </span>
    );
  }

  if (snapshot.status === "fail") {
    const tooltip = [
      snapshot.statusCode ? `HTTP ${snapshot.statusCode}` : null,
      snapshot.error ?? "请求失败",
    ]
      .filter(Boolean)
      .join(" · ");
    return (
      <span
        className="ml-1 inline-flex items-center gap-1 text-rose-500"
        title={tooltip}
      >
        <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-rose-500" />
        <span>连接失败</span>
      </span>
    );
  }

  // skipped
  return (
    <span
      className="ml-1 inline-flex items-center gap-1"
      title={snapshot.error ?? '请在"管理"中选择模型后再启用健康检查'}
    >
      <span
        aria-hidden
        className="h-1.5 w-1.5 rounded-full bg-muted-foreground/60"
      />
      <span>{snapshot.error ?? "未配置模型"}</span>
    </span>
  );
}

