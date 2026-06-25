import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { ToolBadge } from "@/components/tools/ToolBadge";

interface ToolInfo {
  name: string;
  version: string | null;
  error: string | null;
}

interface ToolCard {
  id: string;
  abbr: string;
  label: string;
  color: string;
  detected: boolean;
  version: string | null;
  enabled: boolean;
}

interface ToolDiscoveryPageProps {
  /**
   * Optional. When omitted, the "返回" button is hidden — useful when this
   * page is the entry point of onboarding (already-logged-in user with no
   * bound tools), where there's nothing meaningful to go back to.
   */
  onBack?: () => void;
  onBind: (selectedTools: string[]) => void;
}

export default function ToolDiscoveryPage({
  onBack,
  onBind,
}: ToolDiscoveryPageProps) {
  const [tools, setTools] = useState<ToolCard[]>([]);
  const [loading, setLoading] = useState(true);

  const detectTools = useCallback(async () => {
    setLoading(true);
    try {
      // Onboarding only renders the local version per tool — no "update
      // available" hint here — so skip the slow remote latest-version fetch.
      const results = await invoke<ToolInfo[]>("get_tool_versions", {
        tools: null,
        wslShellByTool: null,
        includeLatest: false,
      });

      const detectedMap = new Map<string, ToolInfo>();
      for (const r of results) {
        detectedMap.set(r.name, r);
      }

      const cards: ToolCard[] = TOOL_ORDER.map((id) => {
        const meta = TOOL_META[id];
        const info = detectedMap.get(id);
        const detected = !!info && !!info.version && !info.error;
        return {
          id,
          abbr: meta.abbr,
          label: meta.label,
          color: meta.color,
          detected,
          version: info?.version ?? null,
          enabled: detected, // 默认选中已检测到的工具
        };
      });

      setTools(cards);
    } catch (e) {
      console.error("Tool detection failed:", e);
      // 如果检测失败，显示所有工具为未检测状态
      setTools(
        TOOL_ORDER.map((id) => ({
          id,
          ...TOOL_META[id],
          detected: false,
          version: null,
          enabled: false,
        })),
      );
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    detectTools();
  }, [detectTools]);

  const toggleTool = (id: string) => {
    setTools((prev) =>
      prev.map((t) =>
        t.id === id && t.detected ? { ...t, enabled: !t.enabled } : t,
      ),
    );
  };

  const selectedCount = tools.filter((t) => t.enabled).length;
  const detectedCount = tools.filter((t) => t.detected).length;

  return (
    <div className="flex h-screen w-full items-center justify-center bg-gradient-to-br from-orange-50/80 via-white to-orange-50/40 dark:from-neutral-950 dark:via-neutral-900 dark:to-neutral-950">
      <div className="flex w-full max-w-2xl flex-col items-center px-8">
        {/* Title */}
        <h1 className="mb-2 text-3xl font-bold text-foreground">
          {loading
            ? "正在扫描 AI 工具..."
            : `已发现 ${detectedCount} 个 AI 工具`}
        </h1>
        <p className="mb-8 text-center text-sm text-muted-foreground">
          选择要接入 Ofox 的工具，我们将生成独立 API Key 并备份原配置
        </p>

        {/* Tool Grid */}
        <div className="mb-8 grid w-full grid-cols-3 gap-4">
          {tools.map((tool) => (
            <button
              key={tool.id}
              onClick={() => toggleTool(tool.id)}
              disabled={!tool.detected}
              className={`flex flex-col items-center gap-2 rounded-xl border-2 px-4 py-5 transition-all ${
                tool.enabled
                  ? "border-orange-400 bg-white shadow-sm dark:bg-neutral-800"
                  : tool.detected
                    ? "border-border bg-white hover:border-orange-200 dark:bg-neutral-800"
                    : "border-border/50 bg-muted/30 opacity-50"
              }`}
            >
              {/* Icon */}
              <ToolBadge
                toolId={tool.id}
                size={40}
                rounded="xl"
                dimmed={!tool.detected}
              />

              {/* Name */}
              <span
                className={`text-sm font-medium ${
                  tool.detected
                    ? "text-foreground"
                    : "text-muted-foreground"
                }`}
              >
                {tool.label}
              </span>

              {/* Version */}
              <span className="text-[11px] text-muted-foreground">
                {tool.version ?? "未安装"}
              </span>

              {/* Toggle */}
              <div
                className={`relative h-6 w-11 rounded-full transition-colors ${
                  tool.enabled
                    ? "bg-orange-500"
                    : tool.detected
                      ? "bg-gray-300 dark:bg-gray-600"
                      : "bg-gray-200 dark:bg-gray-700"
                }`}
              >
                <div
                  className={`absolute top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${
                    tool.enabled ? "translate-x-5" : "translate-x-0.5"
                  }`}
                />
              </div>
            </button>
          ))}
        </div>

        {/* Actions */}
        <div className="flex w-full max-w-sm items-center justify-center gap-4">
          {onBack && (
            <button
              onClick={onBack}
              className="px-6 py-2.5 text-sm text-muted-foreground hover:text-foreground"
            >
              返回
            </button>
          )}
          <button
            onClick={() =>
              onBind(tools.filter((t) => t.enabled).map((t) => t.id))
            }
            disabled={selectedCount === 0 || loading}
            className="flex-1 rounded-xl bg-gradient-to-r from-orange-400 to-orange-500 px-6 py-3 text-base font-semibold text-white shadow-md shadow-orange-200 transition-all hover:from-orange-500 hover:to-orange-600 disabled:opacity-50 dark:shadow-orange-900/20"
          >
            {loading
              ? "扫描中..."
              : `开始绑定（${selectedCount}）→`}
          </button>
        </div>
      </div>
    </div>
  );
}
