import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Unknown capabilities fall back to upstream instructions, never a blind install. */
export function useToolInstallCapabilities() {
  const [installableTools, setInstallableTools] = useState<string[]>([]);
  useEffect(() => {
    let cancelled = false;
    void invoke<string[]>("get_tool_install_capabilities")
      .then((tools) => {
        if (!cancelled) setInstallableTools(tools);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);
  return installableTools;
}
