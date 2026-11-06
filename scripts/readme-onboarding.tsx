/** README-only onboarding: production screens, synthetic discovery, no binding. */
import { useEffect, useState, type ReactNode } from "react";
import ToolDiscoveryPage from "@/components/onboarding/ToolDiscoveryPage";
import SetupCompletePage from "@/components/onboarding/SetupCompletePage";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import type { ToolInstallationInfo } from "@/lib/api/toolUpdates";
import { BOUND_TOOLS, toolVersions } from "./readme-showcase-data";

export function getReadmeOnboardingVersions(): ToolInstallationInfo[] {
  return TOOL_ORDER.map(
    (name) =>
      toolVersions.find((tool) => tool.name === name) ?? {
        name,
        version: null,
        error: null,
        installationStatus: "notInstalled" as const,
      },
  );
}

export function ReadmeOnboarding({
  scene,
  language,
  diagnostics,
  renderConsole,
  onOpenConsole,
}: {
  scene: string;
  language: string;
  diagnostics: { ready: boolean };
  renderConsole: () => ReactNode;
  onOpenConsole: () => void;
}) {
  const [stage, setStage] = useState<"discovery" | "complete" | "console">(
    scene === "complete"
      ? "complete"
      : ["console", "skills", "mcp-preview"].includes(scene)
        ? "console"
        : "discovery",
  );
  useEffect(() => {
    if (stage === "console") onOpenConsole();
  }, [stage, onOpenConsole]);
  const [selected, setSelected] = useState<string[]>(() =>
    scene === "complete" ? [...BOUND_TOOLS] : [],
  );
  useEffect(() => {
    // This flag certifies a committed fixture, not completion of native work.
    diagnostics.ready = true;
  }, [diagnostics, stage]);

  function openConsole() {
    onOpenConsole();
    setStage("console");
  }

  if (stage === "console") return renderConsole();

  return (
    <div
      data-readme-onboarding-stage={stage}
      data-readme-selected-count={selected.length}
    >
      {stage === "complete" ? (
        <SetupCompletePage boundTools={selected} onOpenConsole={openConsole} />
      ) : (
        <ToolDiscoveryPage
          onBind={(tools) => {
            // Only a React-state scene transition. Never call bindTools, issue
            // keys, change account state, or invoke configuration-writing IPC.
            setSelected([...new Set(tools)].filter((id) => id in TOOL_META));
            setStage("complete");
          }}
          onEnterLocal={openConsole}
        />
      )}
      <span className="pointer-events-none fixed bottom-2 right-3 z-[100] rounded-full border border-line bg-canvas/95 px-2.5 py-1 text-[10px] text-ink-muted">
        {{
          en: "Sample workspace · illustrative data",
          zh: "演示工作区 · 示例数据",
          ja: "デモワークスペース · サンプルデータ",
        }[language] ?? "Sample workspace"}
      </span>
    </div>
  );
}
