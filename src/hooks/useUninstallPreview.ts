import { useEffect, useState } from "react";
import {
  toolUninstallApi,
  type UninstallPreview,
} from "@/lib/api/toolUninstall";
import { readBoundTools } from "@/lib/bindTools";

/** Re-read the renderer's bound registry on every open and retry. */
export function useUninstallPreview(tool: string | null, enabled = true) {
  const [state, setState] = useState<{
    tool: string;
    data?: UninstallPreview;
    error?: string;
  } | null>(null);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (!tool || !enabled) {
      setState(null);
      return;
    }
    let disposed = false;
    setState(null);
    void Promise.resolve()
      .then(() => toolUninstallApi.preview(tool, readBoundTools()))
      .then(
        (data) => {
          if (!disposed) setState({ tool, data });
        },
        (error) => {
          if (!disposed) setState({ tool, error: String(error) });
        },
      );
    return () => {
      disposed = true;
    };
  }, [tool, enabled, revision]);
  const current = enabled && state?.tool === tool ? state : null;
  return {
    data: current?.data,
    error: current?.error,
    available:
      !!current?.data?.installed &&
      current.data.blockedReason !== "unsupportedPlatform",
    retry: () => {
      setState(null);
      setRevision((n) => n + 1);
    },
  };
}
