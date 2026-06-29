import { useEffect, useState, useCallback, useRef } from "react";
import {
  Settings,
  RefreshCw,
  Loader2,
  ArrowUpCircle,
  BarChart3,
  SlidersHorizontal,
  Wrench,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { settingsApi } from "@/lib/api";
import { proxyApi } from "@/lib/api/proxy";
import { ofoxGetUserInfo, isOfoxBillingManager, type OfoxUserInfo } from "@/lib/api/ofoxAuth";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import {
  ofoxAnalyticsUrl,
  ofoxAvatarUrl,
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
// OfoxApexSwitch 不再在 ConsolePage 顶栏渲染——已迁入 OfoxSettingsDialog 的
// "区域" SectionCard。useOfoxApex() hook 仍保留：ofoxAnalyticsUrl(apex, ...)
// 还要它来拼"数据统计"按钮的目标 URL。
import { UserAvatar } from "@/components/UserAvatar";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useUpdate } from "@/contexts/UpdateContext";
import { manageToolApi } from "@/lib/api/manageTool";
import ofoxLogo from "@/assets/icons/ofox-logo.png";

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
  const { t } = useTranslation();
  // 应用自更新：底部栏提示按钮 + 首次发现弹窗。数据源 = UpdateContext
  // （自动检查走 R2 latest.json）。
  const {
    hasUpdate,
    isDismissed,
    updateInfo,
    shouldPrompt: shouldPromptUpdate,
    markPrompted,
    dismissUpdate,
  } = useUpdate();
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

  // tool.id → 该工具绑定时 ofox 签发的 API key id。来源 `ofox_list_api_keys`
  // 命令（读 settings.json 的 ofoxApiKeys，`ApiKeyMeta.tool` 是小写工具名、
  // `keyId` 是服务端 id）。用于"数据统计"按钮拼 analytics URL；没拿到 id 的
  // 工具不展示该按钮（按钮要求 key id 作为查询参数）。
  const [apiKeyIdByTool, setApiKeyIdByTool] = useState<
    Record<string, string>
  >({});

  // tool.id → 工具行二级信息要显示的 API key 标签。显示优先级：
  // user 起的 alias > 服务端返回的 keyStart 前缀（如 "sk-of-Ab12"）>
  // keyId 末 6 位 fallback。从同一份 ApiKeyMeta 拿到，所以和 keyId 一起填。
  const [apiKeyLabelByTool, setApiKeyLabelByTool] = useState<
    Record<string, string>
  >({});

  // tool.id → 当前 active model id（来自 manageToolApi.getActiveModel）。
  // 空字符串表示"未设置"——工具行二级信息会显示"未设置 model"提示用户
  // 进"管理"挑一个。loadData 时批量拉，bind 后 ofox-prefs-updated 触发
  // 重拉以反映 ensureDefaultModel 写入的默认值。
  const [modelByTool, setModelByTool] = useState<Record<string, string>>({});

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
    const [toolResults, takeoverStatus, apiKeyMetas] = await Promise.all([
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
      // 本地 settings.json 里的 ofox API key 元数据——拿每个工具的 keyId 给
      // "数据统计"按钮拼 URL，顺带拿 name / alias 给工具行二级信息行显示。
      // 失败（如未绑定任何 key）静默回退空数组，按钮自然不显示，不阻塞列表
      // 渲染。
      invoke<
        Array<{
          tool: string;
          keyId: string;
          name?: string | null;
          alias?: string | null;
          keyStart?: string | null;
        }>
      >("ofox_list_api_keys").catch(
        () =>
          [] as Array<{
            tool: string;
            keyId: string;
            name?: string | null;
            alias?: string | null;
            keyStart?: string | null;
          }>,
      ),
    ]);

    const keyIdMap: Record<string, string> = {};
    const keyLabelMap: Record<string, string> = {};
    for (const meta of apiKeyMetas) {
      if (meta.keyId) keyIdMap[meta.tool] = meta.keyId;
      // 显示优先级：用户在 desktop 端起的 alias > bind 时落本地的 name
      // （`<tool> on <host>`，更易识别"哪台机器哪个工具"）> keyStart 前缀
      // 兼容老数据 > keyId 末 6 位兜底。任一非空都行。
      const aliasTrim = meta.alias?.trim();
      const nameTrim = meta.name?.trim();
      const label =
        (aliasTrim && aliasTrim) ||
        (nameTrim && nameTrim) ||
        meta.keyStart ||
        (meta.keyId ? `…${meta.keyId.slice(-6)}` : "");
      if (label) keyLabelMap[meta.tool] = label;
    }
    setApiKeyIdByTool(keyIdMap);
    setApiKeyLabelByTool(keyLabelMap);

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

    // 工具行二级信息——批量读各 tool 的 active model。每个 invoke ≈ DB
    // 单查询，6 个工具的总开销可忽略。失败的 tool 当 "" 处理（二级行显示
    // "未设置 model"，引导用户去"管理"挑一个）。Promise.all 不阻塞列表
    // 渲染：上面 setTools/setLoading 已让基础行先亮起来。
    void (async () => {
      const entries = await Promise.all(
        ordered.map(async (id) => {
          try {
            return [id, (await manageToolApi.getActiveModel(id)).trim()] as const;
          } catch {
            return [id, ""] as const;
          }
        }),
      );
      setModelByTool(Object.fromEntries(entries));
    })();

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

  // 监听 ofox-prefs-updated —— bindTools.ts 在每次 bind 完一组工具后会
  // emit 一次（ensureDefaultModel 可能已异步写了默认 model）；unbindTool 也
  // 经由 mirrorBoundToolsToSettings 异步 emit 一次。节流 500ms 防 burst。
  //
  // 关键陷阱：**listener 必须用 ref 持有最新 loadData**，不能把 loadData 直接
  // 写进 useEffect 依赖。原因——emit 是 Tauri IPC fire-and-forget，发出与回
  // 调到达 listener 之间有时延；这段时延里 React 已经完成 boundTools 变化的
  // reconcile，旧 useEffect 已经 cleanup（clearTimeout 看到的是空 pending），
  // 新 listener 还没来得及 setup。emit 回调命中 still-alive 的旧 listener，
  // 起了一个 500ms 后跑**旧 closure 里旧 loadData** 的 timer，没人能 clear。
  // 结果就是 unbind 后 React state 已经 `[]`，但 500ms 后旧 loadData
  // 把 `tools` setTools 回 `["codex"]`——UI 残留幽灵条目，无法消除。
  //
  // useRef 保证 listener 总是调到最新的 loadData，useEffect 只跑一次绑定，
  // 不再依赖 loadData，没有 cleanup race。
  const loadDataRef = useRef(loadData);
  useEffect(() => {
    loadDataRef.current = loadData;
  }, [loadData]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let pending: ReturnType<typeof setTimeout> | null = null;
    (async () => {
      try {
        const off = await listen("ofox-prefs-updated", () => {
          if (pending) clearTimeout(pending);
          pending = setTimeout(() => {
            pending = null;
            void loadDataRef.current();
          }, 500);
        });
        unlisten = off;
      } catch (e) {
        console.error("[ConsolePage] listen ofox-prefs-updated failed", e);
      }
    })();
    return () => {
      if (pending) clearTimeout(pending);
      if (unlisten) unlisten();
    };
  }, []);

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
          className="flex h-full items-center justify-center gap-1.5"
          data-tauri-drag-region="true"
        >
          <img
            src={ofoxLogo}
            alt=""
            aria-hidden
            className="h-4 w-4"
            data-tauri-drag-region="true"
          />
          <span
            className="text-[13px] font-medium text-muted-foreground"
            data-tauri-drag-region="true"
          >
            Ofox Desktop
          </span>
        </div>
        {/* Apex (region) switcher 已迁入"设置"弹窗的"区域"卡——顶栏只保留
            logo + 标题，更干净；apex 切换是低频操作，藏一层更合适。 */}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-5 pb-4">
        {/* Profile + Balance Card */}
        <div className="mb-4 rounded-xl border border-border bg-background/80 px-5 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <UserAvatar
                avatarUrl={ofoxAvatarUrl(user?.avatar_url)}
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
            // 的视觉跳变，尤其是用户已经选好滚动位置时。
            <div className="relative">
              {tools.map((tool) => {
                const keyLabel = apiKeyLabelByTool[tool.id];
                const model = modelByTool[tool.id];
                const hasSecondary = !!(keyLabel || model !== undefined);
                return (
                  <div
                    key={tool.id}
                    className="flex items-center gap-3 border-b border-border px-4 py-3 last:border-b-0"
                  >
                    <ToolBadge toolId={tool.id} size={36} rounded="xl" />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2 text-[13px] font-medium text-foreground">
                        <span>{tool.label}</span>
                        {tool.version && (
                          <span className="text-[11px] font-normal text-muted-foreground">
                            v{tool.version}
                          </span>
                        )}
                      </div>
                      {hasSecondary && (
                        // 二级信息行——展示当前绑定的 API key 标签 + active model。
                        // 任一为空时用 "—" / "未设置 model" 占位，让两段宽度
                        // 稳定，避免行高跳动。`truncate` 防止长 model id 把
                        // 右侧按钮挤变形。
                        <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                          <span>{keyLabel ?? "—"}</span>
                          <span className="mx-1.5 opacity-50">·</span>
                          <span>{model || "未设置 model"}</span>
                        </div>
                      )}
                    </div>
                    {/* Action column — "数据统计"（按是否拿到 keyId 决定渲染）
                        + 管理/修复。"延迟测试"按钮已下线——日常排障不需要，
                        重要的连通性测试仍在"管理"弹窗里。 */}
                    <div className="flex items-center justify-end gap-1.5">
                      {apiKeyIdByTool[tool.id] && (
                        <button
                          onClick={() =>
                            settingsApi
                              .openExternal(
                                ofoxAnalyticsUrl(
                                  apex,
                                  apiKeyIdByTool[tool.id],
                                ),
                              )
                              .catch((e) => {
                                console.error(
                                  "[ConsolePage] open analytics url failed",
                                  e,
                                );
                                toast.error("打开数据统计失败");
                              })
                          }
                          title="在浏览器中查看该工具 API key 的用量数据统计"
                          className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[12px] font-medium text-muted-foreground hover:bg-accent"
                        >
                          <BarChart3 className="h-3.5 w-3.5" />
                          数据统计
                        </button>
                      )}
                      {tool.status === "error" ? (
                        <button className="inline-flex items-center gap-1.5 rounded-lg bg-orange-500 px-2.5 py-1 text-[12px] font-medium text-white hover:bg-orange-600">
                          <Wrench className="h-3.5 w-3.5" />
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
                          className="inline-flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1 text-[12px] font-medium text-muted-foreground hover:bg-accent"
                        >
                          <SlidersHorizontal className="h-3.5 w-3.5" />
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
        <div className="flex items-center gap-4">
          <button
            onClick={() => setSettingsDialogOpen(true)}
            className="flex items-center gap-1.5 text-[12px] text-muted-foreground hover:text-foreground"
          >
            <Settings className="h-3.5 w-3.5" />
            设置
          </button>
          {hasUpdate && !isDismissed && (
            <div className="flex items-center gap-1">
              <button
                onClick={() => {
                  const target = updateInfo?.downloadUrl;
                  if (target) {
                    settingsApi.openExternal(target).catch((e) => {
                      console.error("[ConsolePage] open download url failed", e);
                      toast.error(t("settings.openReleaseNotesFailed"));
                    });
                  }
                }}
                title={t("settings.updateAvailable", {
                  version: updateInfo?.availableVersion ?? "",
                })}
                className="flex items-center gap-1.5 text-[12px] font-medium text-emerald-600 hover:text-emerald-700 dark:text-emerald-400 dark:hover:text-emerald-300"
              >
                <ArrowUpCircle className="h-3.5 w-3.5" />
                {t("update.bannerTitle", { defaultValue: "发现新版本" })}
                {updateInfo?.availableVersion
                  ? ` v${updateInfo.availableVersion}`
                  : ""}
              </button>
              <button
                onClick={() => dismissUpdate()}
                title={t("update.skipThisVersion", {
                  defaultValue: "跳过此版本",
                })}
                className="rounded px-1 text-[11px] text-muted-foreground hover:text-foreground"
              >
                {t("update.skipThisVersion", { defaultValue: "跳过此版本" })}
              </button>
            </div>
          )}
        </div>
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

      {/* 首次发现新版本的提示对话框。底部栏的提示按钮常驻（hasUpdate &&
          !isDismissed），这里只在本会话首次发现时弹一次，弹过即由
          markPrompted 关闭，避免每次定时复查都打扰。 */}
      <ConfirmDialog
        isOpen={shouldPromptUpdate}
        variant="info"
        title={t("update.promptTitle", { defaultValue: "发现新版本" })}
        message={t("update.promptMessage", {
          version: updateInfo?.availableVersion ?? "",
          defaultValue: "Ofox Desktop {{version}} 已发布，前往下载页获取更新。",
        })}
        confirmText={t("settings.goDownload", { defaultValue: "前往下载" })}
        cancelText={t("update.later", { defaultValue: "稍后" })}
        onConfirm={() => {
          markPrompted();
          const target = updateInfo?.downloadUrl;
          if (target) {
            settingsApi.openExternal(target).catch((e) => {
              console.error("[ConsolePage] open download url failed", e);
              toast.error(t("settings.openReleaseNotesFailed"));
            });
          }
        }}
        onCancel={() => {
          // "稍后"：仅关闭弹窗，底部栏提示按钮保留。
          markPrompted();
        }}
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

