import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * 主页"打开"按钮：调 Rust `launch_tool_cli`，让系统终端（Terminal.app /
 * iTerm2 等）打开新窗口跑对应工具的 CLI。子进程归属新开的终端，Ofox 主
 * 进程退出后 CLI 仍然存活，直到用户主动关闭那个终端窗口——这是"独立生命
 * 周期"的实现落点，跟 useToolInstall 的 osascript 弹窗同款机制。
 *
 * 用 `launching` set 兜住短暂 loading + 幂等（防连点起多个窗口）。invoke
 * 返回后 200ms 再摘掉标记，给用户视觉反馈；同一工具在窗口中的锁窗口内点
 * 第二次会被直接忽略。
 */
export interface UseToolLaunch {
  launching: Set<string>;
  launch: (toolId: string) => Promise<void>;
}

export function useToolLaunch(
  onError?: (toolId: string, err: unknown) => void,
): UseToolLaunch {
  const [launching, setLaunching] = useState<Set<string>>(new Set());

  const launch = useCallback(
    async (toolId: string) => {
      let already = false;
      setLaunching((prev) => {
        if (prev.has(toolId)) {
          already = true;
          return prev;
        }
        const next = new Set(prev);
        next.add(toolId);
        return next;
      });
      if (already) return;

      try {
        await invoke<void>("launch_tool_cli", { toolId });
      } catch (e) {
        console.error(`[useToolLaunch] launch_tool_cli(${toolId}) 失败`, e);
        onError?.(toolId, e);
      } finally {
        // 短延时清 loading——invoke 已返回但用户还没看到终端弹出，
        // 避免按钮瞬闪。
        setTimeout(() => {
          setLaunching((prev) => {
            if (!prev.has(toolId)) return prev;
            const next = new Set(prev);
            next.delete(toolId);
            return next;
          });
        }, 400);
      }
    },
    [onError],
  );

  return { launching, launch };
}
