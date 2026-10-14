import { useCallback, useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Own subscriptions across async registration, unmount and StrictMode remount. */
export function useToolJob() {
  const generation = useRef(0);
  const mounted = useRef(true);
  const subscriptions = useRef(new Set<UnlistenFn>());
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      for (const stop of subscriptions.current) stop();
      subscriptions.current.clear();
    };
  }, []);
  const subscribe = useCallback(
    async <T>(name: string, handler: (payload: T) => void) => {
      const epoch = generation.current;
      const off = await listen<T>(name, (event) => {
        if (mounted.current && epoch === generation.current)
          handler(event.payload);
      });
      if (!mounted.current || epoch !== generation.current) {
        off();
        throw new Error("Tool operation subscription was cancelled");
      }
      let stopped = false;
      const stop = () => {
        if (stopped) return;
        stopped = true;
        subscriptions.current.delete(stop);
        off();
      };
      subscriptions.current.add(stop);
      return stop;
    },
    [],
  );
  const captureLifetime = useCallback(() => {
    const epoch = generation.current;
    return () => mounted.current && epoch === generation.current;
  }, []);
  return { subscribe, captureLifetime };
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
export function parseProgressLine(line: string): InstallProgress | null {
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
