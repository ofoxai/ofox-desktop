import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * 调 Rust `install_tool` command，并订阅 `install-tool-log` /
 * `install-tool-done` 事件，让 UI 卡片能 (a) 进入 installing 态 (b) 装完
 * 自动回调让父组件重扫工具版本。
 *
 * 详细 npm 日志在后台 osascript 弹出的 Terminal 窗口里给用户看；这里捕获到
 * 的只是 init.py 主进程的状态行，目前打到 console，未来若要做一个内嵌日志
 * 抽屉，只需扩 `logs` state 即可。
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

export interface UseToolInstall {
  /** 当前正在装的工具 id 集合——同一时刻可能多个并发。 */
  installing: Set<string>;
  /** 触发安装。`skipEnv=true` 时跳过 nvm/Node 公共环境，加速重复调用。 */
  install: (toolId: string, skipEnv?: boolean) => Promise<void>;
  /**
   * 最近一次 `install_tool` invoke 失败的信息，`null` 表示当前无错误。
   *
   * 只覆盖 **invoke 本身** 抛出的错（找不到 init.sh、非 macOS/arm64、脚本
   * 起不来）——这类失败下子进程根本没跑起来，不会有 `install-tool-done`
   * 事件，卡片瞬间从 installing 退回 missing，UI 上看不出任何异常。把错误
   * 暴露出来让调用方能给用户一条可见反馈，而不是只打进 console。
   */
  error: { toolId: string; message: string } | null;
  /** 手动清除 error（用户关掉提示条时调）。 */
  clearError: () => void;
}

export function useToolInstall(
  onDone: (toolId: string, code: number) => void,
): UseToolInstall {
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  const [error, setError] = useState<{
    toolId: string;
    message: string;
  } | null>(null);

  // 把回调存进 ref，让 useEffect 的依赖列表保持空——否则 onDone 每次重渲染
  // 都会新建函数引用，导致 listen 不断重订阅（旧 listener 会泄漏，事件可能
  // 触发多次）。这套 ref 模式跟 ConsolePage 里 loadDataRef 同款。
  const onDoneRef = useRef(onDone);
  useEffect(() => {
    onDoneRef.current = onDone;
  }, [onDone]);

  useEffect(() => {
    let un1: UnlistenFn | undefined;
    let un2: UnlistenFn | undefined;

    (async () => {
      un1 = await listen<InstallLog>("install-tool-log", (e) => {
        // 临时方案：详细日志走 console，便于 dev 时排查路径/参数问题。
        // 真实 npm 日志在 osascript 弹的 Terminal 里，不通过这条事件流。
        const { tool, stream, line } = e.payload;
        console.log(`[install:${tool}:${stream}] ${line}`);
      });

      un2 = await listen<InstallDone>("install-tool-done", (e) => {
        const { tool, code } = e.payload;
        setInstalling((prev) => {
          if (!prev.has(tool)) return prev;
          const next = new Set(prev);
          next.delete(tool);
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
    async (toolId: string, skipEnv: boolean = false) => {
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

      // 重试前先清掉上一次的错误，否则旧提示会一直挂着。
      setError(null);

      try {
        await invoke<number>("install_tool", { toolId, skipEnv });
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

  return { installing, install, error, clearError };
}
