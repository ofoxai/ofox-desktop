import { useEffect, useRef, useState } from "react";
import { useToolJob } from "./useToolJob";
import {
  toolUninstallApi,
  type UninstallProgress,
  type UninstallResult,
} from "@/lib/api/toolUninstall";
import { readBoundTools, forgetUninstalledTool } from "@/lib/bindTools";

export function useToolUninstall(
  onCompleted: (result: UninstallResult) => void,
) {
  const { subscribe, captureLifetime } = useToolJob();
  const completed = useRef(onCompleted);
  useEffect(() => {
    completed.current = onCompleted;
  }, [onCompleted]);
  const busy = useRef(false);
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [progress, setProgress] = useState<UninstallProgress | null>(null);
  const [result, setResult] = useState<UninstallResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const run = async (tool: string) => {
    if (busy.current) return;
    const isCurrent = captureLifetime();
    busy.current = true;
    setRunning(true);
    setError(null);
    setLogs([]);
    setProgress(null);
    setResult(null);
    const operationId = crypto.randomUUID();
    let stop: (() => void) | undefined;
    try {
      stop = await subscribe<UninstallProgress>(
        "tool-uninstall-progress",
        (event) => {
          if (event.operationId !== operationId) return;
          setProgress(event);
          setLogs((lines) => [...lines.slice(-199), event.line]);
        },
      );
      if (!isCurrent()) return;
      const outcome = await toolUninstallApi.uninstall(
        tool,
        operationId,
        readBoundTools(),
      );
      // Even partial removal can have successfully restored configuration and retired a key.
      if (isCurrent()) setResult(outcome);
      await forgetUninstalledTool(tool, outcome);
      if (isCurrent()) completed.current(outcome);
    } catch (cause) {
      if (isCurrent()) setError(String(cause));
    } finally {
      stop?.();
      busy.current = false;
      if (isCurrent()) setRunning(false);
    }
  };
  return { run, running, logs, progress, result, error };
}
