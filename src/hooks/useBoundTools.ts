import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { announceKeyReconcile } from "@/lib/accountKeys";
import { ofoxReconcileToolKeys } from "@/lib/api/ofoxAuth";
import { ofoxBindApi, type ToolBindingStatus } from "@/lib/api/ofoxBind";
import {
  getInstallationStatus,
  type ToolInstallationInfo,
} from "@/lib/api/toolUpdates";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import type { BoundToolRowData } from "@/components/console/BoundToolRow";
import { invalidateModelState } from "@/hooks/useModelSelection";
import { useOfoxApex } from "@/hooks/useOfoxApex";
import { checkToolUpdates } from "@/hooks/useToolUpdates";
import { isMeaningfulUser, type OfoxAccount } from "@/hooks/useOfoxAccount";

export type BoundTool = BoundToolRowData;

/** 切回窗口时最多每 30 秒重新检测一次工具。 */
const FOCUS_REFRESH_INTERVAL_MS = 30_000;

/** "quiet"：后台刷新，列表照常显示和操作，不叠「刷新中…」遮罩。 */
type LoadMode = "shown" | "quiet";

interface ApiKeyMeta {
  tool: string;
  keyId: string;
  name?: string | null;
  alias?: string | null;
  keyStart?: string | null;
}

const UNKNOWN_BINDING: ToolBindingStatus = {
  status: "unknown",
  message: null,
  missingFiles: [],
  modifiedFields: [],
  envOverrides: [],
};

export interface BoundTools {
  tools: BoundTool[];
  /** First load or a shown reload is running. */
  loading: boolean;
  /** The manual refresh (`refreshList`) is running. */
  listRefreshing: boolean;
  /** tool.id → the OFox API key id it was bound with, for the analytics URL. */
  apiKeyIdByTool: Record<string, string>;
  /** tool.id → the API key label shown on its row. */
  apiKeyLabelByTool: Record<string, string>;
  /** Re-detect the tools and recheck their updates, with a toast. */
  refreshList: () => Promise<void>;
  /** Re-detect the tools with the overlay shown. Stable across renders. */
  reload: () => Promise<void>;
}

/**
 * The bound tools as detected on this machine: installation, binding status
 * and API key labels. Re-detects on bind/unbind (`ofox-prefs-updated`), after
 * tool updates and when the window regains focus (at most every 30s).
 *
 * After each scan it loads the OFox user through `account`, moves bound tools
 * onto the current account's keys, and retries an empty user.
 */
export function useBoundTools(
  boundTools: string[],
  account: Pick<OfoxAccount, "loadUser" | "retryUser">,
): BoundTools {
  const { t } = useTranslation();
  const { apex } = useOfoxApex();
  const { loadUser, retryUser } = account;
  const [tools, setTools] = useState<BoundTool[]>([]);
  const [loading, setLoading] = useState(true);
  /** Spinner for the manual refresh — re-runs `loadData` to refresh
   *  detection / takeover status / tool health. */
  const [listRefreshing, setListRefreshing] = useState(false);
  /** 上一次开始检测的时间：切回窗口时据此节流，见 FOCUS_REFRESH_INTERVAL_MS。 */
  const lastLoadStartedAt = useRef(0);

  // tool.id → 该工具绑定时 ofox 签发的 API key id。来源 `ofox_list_api_keys`
  // 命令（读 settings.json 的 ofoxApiKeys，`ApiKeyMeta.tool` 是小写工具名、
  // `keyId` 是服务端 id）。用于"数据统计"按钮拼 analytics URL；没拿到 id 的
  // 工具不展示该按钮（按钮要求 key id 作为查询参数）。
  const [apiKeyIdByTool, setApiKeyIdByTool] = useState<Record<string, string>>(
    {},
  );

  // tool.id → 工具行二级信息要显示的 API key 标签。显示优先级：
  // user 起的 alias > 服务端返回的 keyStart 前缀（如 "sk-of-Ab12"）>
  // keyId 末 6 位 fallback。从同一份 ApiKeyMeta 拿到，所以和 keyId 一起填。
  const [apiKeyLabelByTool, setApiKeyLabelByTool] = useState<
    Record<string, string>
  >({});

  const loadGeneration = useRef(0);
  const boundToolsRef = useRef(boundTools);
  boundToolsRef.current = boundTools;

  useEffect(
    () => () => {
      loadGeneration.current += 1;
    },
    [],
  );

  const loadData = useCallback(
    async (mode?: LoadMode) => {
      const generation = ++loadGeneration.current;
      lastLoadStartedAt.current = Date.now();
      // 首次加载和手动「刷新」显示遮罩；切回窗口等后台刷新检测完再替换。
      if (mode !== "quiet") setLoading(true);

      // Two-stage load — the bound-tools list is the user's primary focus and
      // must not wait on the OFox profile/balance round-trip (which can be
      // multi-second under flaky network). Stage 1 covers everything backed by
      // local SQL (tool detection, takeover status) and flips loading→false.
      // Stage 2 covers OFox profile/balance; it runs in the background and
      // updates the header when ready. A failure in stage 2 leaves the list
      // intact and the header in its placeholder state — never spins forever.

      // ===== Stage 1: local-only, blocks list render =====
      const [toolResults, bindingEntries, apiKeyMetas] = await Promise.all([
        // includeLatest=false: skip the npm/GitHub fetch — we only need the
        // local "is it installed?" check here. That fetch was the main reason
        // this list took ~5s to render.
        // 只检测已绑定的工具：每个工具都要起一次 `--version`，Windows 上经
        // cmd 和 Node 启动的 CLI 一个就要一两秒。
        invoke<ToolInstallationInfo[]>("get_tool_versions", {
          tools: boundTools,
          wslShellByTool: null,
          includeLatest: false,
        }).catch(() => [] as ToolInstallationInfo[]),
        Promise.all(
          boundTools.map(async (id) => {
            const status = await ofoxBindApi
              .status(id)
              .catch((): ToolBindingStatus => UNKNOWN_BINDING);
            return [id, status] as const;
          }),
        ),
        // 本地 settings.json 里的 ofox API key 元数据——拿每个工具的 keyId 给
        // "数据统计"按钮拼 URL，顺带拿 name / alias 给工具行二级信息行显示。
        // 失败（如未绑定任何 key）静默回退空数组，按钮自然不显示，不阻塞列表
        // 渲染。
        invoke<ApiKeyMeta[]>("ofox_list_api_keys").catch(
          () => [] as ApiKeyMeta[],
        ),
      ]);

      // A refresh started before an unbind must not resurrect its old row.
      if (
        generation !== loadGeneration.current ||
        boundToolsRef.current !== boundTools
      )
        return;
      const bindingByTool = new Map(bindingEntries);

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

      const detectedMap = new Map<string, ToolInstallationInfo>();
      for (const r of toolResults) {
        detectedMap.set(r.name, r);
      }

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
        return {
          id,
          abbr: meta.abbr,
          label: meta.label,
          color: meta.color,
          version: info?.version ?? null,
          installationKind: info?.installationKind,
          installationStatus: getInstallationStatus(info),
          installationError: info?.error ?? null,
          binding: bindingByTool.get(id) ?? UNKNOWN_BINDING,
        };
      });

      setTools(list);
      setLoading(false);

      // ===== Stage 2: remote / non-blocking =====
      void (async () => {
        const userInfo = await loadUser();

        // 账号信息刚刷新：已绑定的工具如果还在用别的账号的 key（换了账号，或
        // 退出登录时停用了），换成当前账号的。没有要换的时后端不访问网络。
        const keyReport = await ofoxReconcileToolKeys().catch((e) => {
          console.error("[useBoundTools] reconcile tool keys failed", e);
          return null;
        });
        if (keyReport) {
          announceKeyReconcile(keyReport, t, userInfo?.email);
          if (keyReport.switched.length > 0 || keyReport.failed.length > 0)
            void invalidateModelState();
          // 新 key 的标签和统计链接要重新读。
          if (keyReport.switched.length > 0) void loadDataRef.current();
        }

        // Silently retry a placeholder user a few times with backoff before
        // giving up. If the backend was just having a temporary blip (which
        // is the common case), the user never sees the empty state at all.
        if (!isMeaningfulUser(userInfo)) {
          void retryUser();
        }
      })();
    },
    // `apex` re-runs the scan after a region switch.
    [boundTools, apex, t, loadUser, retryUser],
  );

  // 监听 ofox-prefs-updated 等事件的 listener 必须用 ref 持有最新 loadData，
  // 不能把 loadData 直接写进 useEffect 依赖。原因——emit 是 Tauri IPC
  // fire-and-forget，发出与回调到达 listener 之间有时延；这段时延里 React
  // 已经完成 boundTools 变化的 reconcile，旧 useEffect 已经 cleanup
  // （clearTimeout 看到的是空 pending），新 listener 还没来得及 setup。emit
  // 回调命中 still-alive 的旧 listener，起了一个 500ms 后跑**旧 closure 里旧
  // loadData** 的 timer，没人能 clear。结果就是 unbind 后 React state 已经
  // `[]`，但 500ms 后旧 loadData 把 `tools` setTools 回 `["codex"]`——UI 残留
  // 幽灵条目，无法消除。
  //
  // useRef 保证 listener 总是调到最新的 loadData，useEffect 只跑一次绑定，
  // 不再依赖 loadData，没有 cleanup race。
  const loadDataRef = useRef(loadData);
  useEffect(() => {
    loadDataRef.current = loadData;
  }, [loadData]);

  const reload = useCallback(() => loadDataRef.current(), []);

  /**
   * Re-run `loadData()` to refresh tool detection / takeover status / health.
   * Uses its own spinner so existing rows stay visible mid-refresh instead of
   * bouncing to the loading skeleton.
   */
  const refreshList = useCallback(async () => {
    setListRefreshing(true);
    try {
      await Promise.all([loadData(), checkToolUpdates()]);
      toast.success(t("console.listRefreshed"));
    } catch (e) {
      console.error("[useBoundTools] list refresh failed", e);
      toast.error(t("console.listRefreshFailed", { error: String(e) }));
    } finally {
      setListRefreshing(false);
    }
  }, [loadData, t]);

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

  useEffect(() => {
    const refreshQuietly = () => {
      void loadDataRef.current("quiet");
    };
    // 切回窗口是为了看到在外面装好或升级的工具；频繁切换时不必每次都重新检测。
    const refreshOnFocus = () => {
      if (Date.now() - lastLoadStartedAt.current < FOCUS_REFRESH_INTERVAL_MS)
        return;
      refreshQuietly();
    };
    window.addEventListener("tool-updates-complete", refreshQuietly);
    window.addEventListener("focus", refreshOnFocus);
    return () => {
      window.removeEventListener("tool-updates-complete", refreshQuietly);
      window.removeEventListener("focus", refreshOnFocus);
    };
  }, []);

  // ofox-prefs-updated —— bindTools.ts 在每次 bind 完一组工具后会 emit 一次
  // （ensureDefaultModel 可能已异步写了默认 model）；unbindTool 也经由
  // mirrorBoundToolsToSettings 异步 emit 一次。节流 500ms 防 burst。
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
        console.error("[useBoundTools] listen ofox-prefs-updated failed", e);
      }
    })();
    return () => {
      if (pending) clearTimeout(pending);
      if (unlisten) unlisten();
    };
  }, []);

  return {
    tools,
    loading,
    listRefreshing,
    apiKeyIdByTool,
    apiKeyLabelByTool,
    refreshList,
    reload,
  };
}
