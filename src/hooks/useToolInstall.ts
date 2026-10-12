import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import i18n from "i18next";
import {
  checkToolNodeRequirement,
  type NodeRequirement,
} from "@/lib/api/nodeRequirement";

/**
 * 调 Rust `install_tool` command，并订阅 `install-tool-log` /
 * `install-tool-done` 事件，让 UI 卡片能 (a) 进入 installing 态 (b) 装完
 * 自动回调让父组件重扫工具版本。
 *
 * 详细 npm 日志在后台 osascript 弹出的 Terminal 窗口里给用户看；这里捕获到的只是
 * init.py 主进程的状态行。其中 `app/progress.py` 打的结构化 JSON 行会被解析成
 * `progress` 暴露给 UI，其余文本行仍打到 console——未来若要做内嵌日志抽屉，
 * 扩一个 `logs` state 即可。
 */

interface InstallLog {
  tool: string;
  stream: "stdout" | "stderr";
  line: string;
}

interface InstallDone {
  tool: string;
  /** 0 = 成功；非 0 = 失败（init.py 退出码契约见 init.py 顶部注释）。 */
  code: number;
}

/**
 * 安装器 `app/progress.py` 打出来的结构化进度行的标识。
 * 改这个值要同步改 `scripts/installer/app/progress.py` 的 `PROGRESS_TYPE`。
 */
const PROGRESS_TYPE = "ofox-install-progress";

export interface InstallProgress {
  /** 当前第几步，从 1 开始。 */
  step: number;
  /** 总步数。单工具安装通常是 4 步：nvm → Node → 镜像 → 目标工具。 */
  total: number;
  /** 步骤名，直接来自 steps.py，可展示给用户。 */
  name: string;
  phase: "start" | "waiting" | "done" | "skipped" | "failed";
  /** 该步已等待秒数，仅 waiting 阶段有。 */
  elapsed?: number;
  /** 该步的超时预算秒数，仅 waiting 阶段有。 */
  timeout?: number;
}

/**
 * 从一行安装器输出里提取进度，不是进度行就返回 null。
 *
 * 安装器 stdout 是人类可读文本和进度 JSON 混在一起的，所以这里必须对任何输入
 * 都安全：普通日志、半截 JSON、别的 type 的 JSON 都只能返回 null，不能抛错——
 * 一旦抛错会中断 listen 回调，后续事件全部丢失。
 */
function parseProgressLine(line: string): InstallProgress | null {
  const trimmed = line.trim();
  // 绝大多数行不是 JSON，先做一次廉价判断，省掉 try/catch 开销。
  if (!trimmed.startsWith("{")) return null;

  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(trimmed) as Record<string, unknown>;
  } catch {
    return null;
  }

  if (parsed.type !== PROGRESS_TYPE) return null;
  if (typeof parsed.step !== "number" || typeof parsed.total !== "number") {
    return null;
  }

  // 显式挑字段而不是展开剩余属性：既剥掉了 type，也保证多出来的字段不会漏进
  // state（将来安装器加字段时，前端不会静默接收未知内容）。
  const progress: InstallProgress = {
    step: parsed.step,
    total: parsed.total,
    name: String(parsed.name ?? ""),
    phase: parsed.phase as InstallProgress["phase"],
  };
  if (typeof parsed.elapsed === "number") progress.elapsed = parsed.elapsed;
  if (typeof parsed.timeout === "number") progress.timeout = parsed.timeout;

  return progress;
}

const ANSI_ESCAPE = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, "g");

/**
 * 从安装器输出里认出失败原因，认不出返回 null。对应 `app/display.py`：
 * 预检失败打 `错误: …`，步骤失败先打 `✕ <步骤> 失败` 再打 `提示: …`。
 * 同一次安装里以最后一条为准——步骤失败时 `提示` 比 `✕` 更具体。
 */
function failureReason(line: string): string | null {
  const text = line.replace(ANSI_ESCAPE, "").trim();
  const labeled = /^(?:错误|提示)[:：]\s*(.+)$/.exec(text);
  if (labeled) return labeled[1].trim();
  if (text.startsWith("✕")) return text.slice(1).trim();
  return null;
}

export interface UseToolInstall {
  /** 当前正在装的工具 id 集合——同一时刻可能多个并发。 */
  installing: Set<string>;
  /** 触发安装。`skipEnv=true` 时跳过 nvm/Node 公共环境，加速重复调用。 */
  install: (toolId: string, skipEnv?: boolean) => Promise<void>;
  /**
   * 最近一次安装失败的信息，`null` 表示当前无错误。
   *
   * 两种来源：
   *   - **invoke 本身** 抛错（找不到 init.sh、非 macOS/arm64、脚本起不来）；
   *   - 安装脚本跑起来了但以非零码退出（如预检发现磁盘不足）——message 取
   *     脚本最后打印的失败原因，认不出时给退出码。
   * 两种情况下卡片都会瞬间从 installing 退回 missing，没有这条错误用户会以为
   * 按钮没反应。
   */
  error: { toolId: string; message: string } | null;
  /** 手动清除 error（用户关掉提示条时调）。 */
  clearError: () => void;
  /**
   * 按工具 id 分组的最新安装进度，装完即清除。
   *
   * 注意这里只反映 init.py 主进程推进到了第几步——真正的 npm 下载百分比拿不到，
   * 因为那部分跑在 osascript 弹出的独立 Terminal 里，与主进程没有管道关系。
   */
  progress: Record<string, InstallProgress>;
  /**
   * 工具要求更新的 Node.js：先停下来让用户确认（`NodeUpgradeDialog`），不悄悄升级
   * ——Node 是整台电脑共用的。没有 Node 时不问，安装器会自动装。
   */
  nodePrompt: NodePrompt | null;
  /** 用户同意升级 Node.js，接着安装。 */
  confirmNodeUpgrade: () => Promise<void>;
  dismissNodePrompt: () => void;
}

export interface NodePrompt {
  toolId: string;
  requirement: NodeRequirement;
}

/** 检查本身失败（离线、超时）就照常安装，不拦用户。 */
async function nodeRequirementFor(
  toolId: string,
): Promise<NodeRequirement | null> {
  try {
    return await checkToolNodeRequirement(toolId);
  } catch (e) {
    console.warn(`[useToolInstall] Node.js 检查失败，照常安装 ${toolId}`, e);
    return null;
  }
}

export function useToolInstall(
  onDone: (toolId: string, code: number) => void,
): UseToolInstall {
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  const [progress, setProgress] = useState<Record<string, InstallProgress>>({});
  const [error, setError] = useState<{
    toolId: string;
    message: string;
  } | null>(null);
  const [nodePrompt, setNodePrompt] = useState<
    (NodePrompt & { skipEnv: boolean }) | null
  >(null);

  // 把回调存进 ref，让 useEffect 的依赖列表保持空——否则 onDone 每次重渲染
  // 都会新建函数引用，导致 listen 不断重订阅（旧 listener 会泄漏，事件可能
  // 触发多次）。这套 ref 模式跟 ConsolePage 里 loadDataRef 同款。
  const onDoneRef = useRef(onDone);
  // 每个工具本次安装里最后一条失败原因；安装结束（或重新开始）时清掉。
  const reasonsRef = useRef<Record<string, string>>({});
  useEffect(() => {
    onDoneRef.current = onDone;
  }, [onDone]);

  useEffect(() => {
    let un1: UnlistenFn | undefined;
    let un2: UnlistenFn | undefined;

    (async () => {
      un1 = await listen<InstallLog>("install-tool-log", (e) => {
        const { tool, stream, line } = e.payload;

        const parsed = parseProgressLine(line);
        if (parsed) {
          setProgress((prev) => ({ ...prev, [tool]: parsed }));
          return;
        }

        const reason = failureReason(line);
        if (reason) reasonsRef.current[tool] = reason;

        // 非进度行仍走 console，便于 dev 时排查路径/参数问题。
        // 真实 npm 日志在 osascript 弹的 Terminal 里，不通过这条事件流。
        console.log(`[install:${tool}:${stream}] ${line}`);
      });

      un2 = await listen<InstallDone>("install-tool-done", (e) => {
        const { tool, code } = e.payload;
        const reason = reasonsRef.current[tool];
        delete reasonsRef.current[tool];
        if (code !== 0) {
          setError({
            toolId: tool,
            message: reason ?? i18n.t("toolInstall.exitCode", { code }),
          });
        }
        setInstalling((prev) => {
          if (!prev.has(tool)) return prev;
          const next = new Set(prev);
          next.delete(tool);
          return next;
        });
        // 清掉进度，否则下次装同一个工具时，卡片会先闪一下上次的残留步数。
        setProgress((prev) => {
          if (!(tool in prev)) return prev;
          const next = { ...prev };
          delete next[tool];
          return next;
        });
        onDoneRef.current(tool, code);
      });
    })();

    return () => {
      un1?.();
      un2?.();
    };
  }, []);

  const install = useCallback(
    async (toolId: string, skipEnv: boolean = false, upgradeNode = false) => {
      // 幂等：同一工具已在装就直接 return，避免并发起两条 Terminal 窗口。
      let alreadyInstalling = false;
      setInstalling((prev) => {
        if (prev.has(toolId)) {
          alreadyInstalling = true;
          return prev;
        }
        const next = new Set(prev);
        next.add(toolId);
        return next;
      });
      if (alreadyInstalling) return;

      // 重试前先清掉上一次的错误和残留原因，否则旧提示会一直挂着。
      setError(null);
      delete reasonsRef.current[toolId];

      if (!upgradeNode) {
        const requirement = await nodeRequirementFor(toolId);
        if (requirement?.status === "tooOld") {
          setNodePrompt({ toolId, requirement, skipEnv });
          setInstalling((prev) => {
            if (!prev.has(toolId)) return prev;
            const next = new Set(prev);
            next.delete(toolId);
            return next;
          });
          return;
        }
      }

      try {
        await invoke<number>("install_tool", { toolId, skipEnv, upgradeNode });
        // 真实完成态由 install-tool-done 事件兜底——这里 invoke resolve 时
        // 子进程已经退出，但事件 listener 也会被触发，去重交给上面的 has 检查。
      } catch (e) {
        console.error(`[useToolInstall] install_tool(${toolId}) 调用失败`, e);
        // Rust 端 `Result<_, String>` 的 Err 到这里就是那个 String 本身。
        setError({ toolId, message: typeof e === "string" ? e : String(e) });
        // invoke 失败（脚本启不起来 / 不支持平台），手动从 installing 摘掉。
        setInstalling((prev) => {
          if (!prev.has(toolId)) return prev;
          const next = new Set(prev);
          next.delete(toolId);
          return next;
        });
      }
    },
    [],
  );

  const clearError = useCallback(() => setError(null), []);

  const confirmNodeUpgrade = useCallback(async () => {
    if (!nodePrompt) return;
    setNodePrompt(null);
    await install(nodePrompt.toolId, nodePrompt.skipEnv, true);
  }, [install, nodePrompt]);
  const dismissNodePrompt = useCallback(() => setNodePrompt(null), []);

  return {
    installing,
    install,
    error,
    clearError,
    progress,
    nodePrompt: nodePrompt && {
      toolId: nodePrompt.toolId,
      requirement: nodePrompt.requirement,
    },
    confirmNodeUpgrade,
    dismissNodePrompt,
  };
}
