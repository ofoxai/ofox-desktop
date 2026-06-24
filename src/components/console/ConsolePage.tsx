import { useEffect, useState, useCallback } from "react";
import { Settings, RefreshCw, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";
import { proxyApi } from "@/lib/api/proxy";
import { ofoxGetUserInfo, isOfoxBillingManager, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import {
  ofoxDashboardUrl,
  ofoxMarketingUrl,
  ofoxWalletUrl,
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
import { manageToolApi, type PingResult } from "@/lib/api/manageTool";

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
}

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
  /** Spinner for the "绑定的工具" 刷新 button — re-runs `loadData` to refresh
   *  detection / takeover status / tool health. */
  const [listRefreshing, setListRefreshing] = useState(false);

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

  // 延迟测试 —— 改为按需触发，不再后台轮询。row 上的 "延迟测试" 按钮点一下
  // 就读取该工具的 active model，单独发一次 ping，结果只显示在这一行旁。
  // key = tool.id，value = 该 tool 的 ping 状态：
  //   - undefined → 从未测试过，不展示 pill
  //   - { loading: true } → 进行中
  //   - { result: PingResult } → 已完成（success / fail / 401 / …）
  //   - { error: string } → 客户端/Tauri 错误（极少见）
  // 主动让 `tools` 数据变化时**保留**已有测试结果，让用户不会因为后台 reload
  // 把延迟结果"擦掉"。只有点"刷新"或"管理"里改了 model 才该重测。
  type PingState =
    | { loading: true }
    | { result: PingResult }
    | { error: string };
  const [pingStateByTool, setPingStateByTool] = useState<
    Record<string, PingState | undefined>
  >({});

  const runPingForTool = useCallback(async (toolId: string) => {
    setPingStateByTool((m) => ({ ...m, [toolId]: { loading: true } }));
    try {
      const model = await manageToolApi.getActiveModel(toolId);
      if (!model.trim()) {
        setPingStateByTool((m) => ({
          ...m,
          [toolId]: { error: '未配置模型，请在"管理"中选择' },
        }));
        return;
      }
      const result = await manageToolApi.pingModel(toolId, model);
      setPingStateByTool((m) => ({ ...m, [toolId]: { result } }));
    } catch (e) {
      setPingStateByTool((m) => ({
        ...m,
        [toolId]: { error: String(e) },
      }));
    }
  }, []);

  const loadData = useCallback(async () => {
    setLoading(true);

    // Two-stage load — the bound-tools list is the user's primary focus and
    // must not wait on the OFox profile/balance round-trip (which can be
    // multi-second under flaky network). Stage 1 covers everything backed by
    // local SQL (tool detection, takeover status) and flips loading→false.
    // Stage 2 covers OFox profile/balance; it runs in the background and
    // updates the header when ready. A failure in stage 2 leaves the list
    // intact and the header in its placeholder state — never spins forever.

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

      // Status 只用于决定状态点颜色 + "修复" vs "管理" 按钮分支：
      //   - error  → 没检测到工具（提示用户去装/修）
      //   - active → 检测到且 proxy 接管 OK
      //   - idle   → 检测到、未走代理（OpenCode/OpenClaw/Hermes 这种 ofox 直写
      //              的工具会落在这里——非错误状态，所以不展示"未开启代理"
      //              等中文描述，避免误导用户以为有问题）
      let status: ToolStatus;
      if (!detected) {
        status = "error";
      } else if (proxied) {
        status = "active";
      } else {
        status = "idle";
      }
      // proxySupported 仅参与上面的判定语义，本身不再 surface 到 UI
      void proxySupported;

      return {
        id,
        abbr: meta.abbr,
        label: meta.label,
        color: meta.color,
        version: info?.version ?? null,
        status,
      };
    });

    setTools(list);
    setLoading(false);

    // ===== Stage 2: remote / non-blocking =====
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
   * Re-run `loadData()` to refresh tool detection / takeover status / health.
   * Uses its own spinner so existing rows stay visible mid-refresh instead of
   * bouncing to the loading skeleton.
   */
  const refreshList = useCallback(async () => {
    setListRefreshing(true);
    try {
      await loadData();
      toast.success("已刷新");
    } catch (e) {
      console.error("[ConsolePage] list refresh failed", e);
      toast.error(`刷新失败：${String(e)}`);
    } finally {
      setListRefreshing(false);
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
                onClick={refreshList}
                disabled={listRefreshing}
                title="刷新工具检测与状态"
                className="flex items-center gap-1.5 rounded-md border border-border px-3 py-1 text-[12px] text-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${listRefreshing ? "animate-spin" : ""}`}
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

          {loading && tools.length === 0 ? (
            // 首次加载——列表本身还没渲染过，给个占位条避免空白
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              正在加载...
            </div>
          ) : tools.length === 0 ? (
            <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
              暂无绑定的工具
            </div>
          ) : (
            // 列表始终保持渲染，刷新中叠遮罩——避免刷新时整段消失再回来
            // 的视觉跳变，尤其是用户已经选好滚动位置 / 看着 ping 结果时。
            <div className="relative">
              {tools.map((tool) => {
                const ping = pingStateByTool[tool.id];
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
                      <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
                        <span>{tool.label}</span>
                        {tool.version && (
                          <span className="text-[11px] font-normal text-muted-foreground">
                            v{tool.version}
                          </span>
                        )}
                        <PingResultPill state={ping} />
                      </div>
                    </div>
                    {/* Action column — wider (w-[160px]) so it can host both
                        the new "延迟测试" button and the existing 管理/修复
                        button side-by-side without crowding. */}
                    <div className="flex w-[160px] items-center justify-end gap-1.5">
                      <button
                        onClick={() => runPingForTool(tool.id)}
                        disabled={ping && "loading" in ping}
                        title="单独发一次 ping，测当前 active model 的延迟"
                        className="rounded-lg border border-border px-2.5 py-1 text-[12px] font-medium text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        {ping && "loading" in ping ? "测试中…" : "延迟测试"}
                      </button>
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
                            })
                          }
                          className="w-[68px] rounded-lg py-1 text-center text-[12px] font-medium text-muted-foreground hover:bg-accent"
                        >
                          管理
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
              {(loading || listRefreshing) && (
                <div
                  aria-busy="true"
                  className="pointer-events-none absolute inset-0 flex items-center justify-center bg-background/60 backdrop-blur-[1px]"
                >
                  <span className="inline-flex items-center gap-2 rounded-md bg-background/80 px-3 py-1.5 text-[12px] text-muted-foreground shadow-sm">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    刷新中…
                  </span>
                </div>
              )}
            </div>
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

// ==================== Helpers ====================

/**
 * Render the wallet figure. `null`/`undefined` collapses to `—` so the
 * layout doesn't shift between "loading" and "no balance returned".
 */
function formatBalance(value: number | null | undefined): string {
  if (typeof value !== "number" || Number.isNaN(value)) return "—";
  return `$${value.toFixed(2)}`;
}

// ─── Ping result pill ─────────────────────────────────────────────────────
//
// Renders ONLY after the user clicks "延迟测试"。三态：
//   loading → 灰色 "测试中…"
//   ok      → 绿色 "延迟 X ms"
//   fail    → 红色 "连接失败"（hover 看完整原因）
//   error   → 灰色 短句（如"未配置模型"），用于客户端/前置错误，不是请求层失败

type PingPillState =
  | { loading: true }
  | { result: PingResult }
  | { error: string };

function PingResultPill({ state }: { state: PingPillState | undefined }) {
  if (!state) return null;

  if ("loading" in state) {
    return (
      <span className="inline-flex items-center gap-1 text-[11px] font-normal text-muted-foreground">
        <Loader2 className="h-2.5 w-2.5 animate-spin" />
        <span>测试中…</span>
      </span>
    );
  }

  if ("error" in state) {
    return (
      <span
        className="inline-flex items-center gap-1 text-[11px] font-normal text-muted-foreground"
        title={state.error}
      >
        <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-muted-foreground/60" />
        <span>{state.error}</span>
      </span>
    );
  }

  const r = state.result;
  if (r.success) {
    return (
      <span
        className="inline-flex items-center gap-1 text-[11px] font-normal"
        title={`HTTP ${r.statusCode ?? 200}`}
      >
        <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
        <span className="text-foreground">延迟 {r.latencyMs}ms</span>
      </span>
    );
  }

  const tooltip = [
    r.statusCode ? `HTTP ${r.statusCode}` : null,
    r.error ?? "请求失败",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <span
      className="inline-flex items-center gap-1 text-[11px] font-normal text-rose-500"
      title={tooltip}
    >
      <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-rose-500" />
      <span>连接失败</span>
    </span>
  );
}

