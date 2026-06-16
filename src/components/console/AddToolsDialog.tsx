import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { TOOL_META, TOOL_ORDER } from "@/config/toolMeta";
import { bindTools } from "@/lib/bindTools";

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

interface AddToolsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Tools already bound — they're hidden from the picker so we never offer
   *  duplicates. Order doesn't matter; we filter set-style. */
  alreadyBound: string[];
  /** Called with the FULL new bound list (alreadyBound ∪ newlyAdded) after
   *  bindTools() succeeds. Caller is responsible for re-reading
   *  localStorage / state to pick up the change. */
  onAdded: (newBoundList: string[]) => void;
}

/**
 * In-Console picker for binding additional tools.
 *
 * Mirrors the gate logic of `ToolDiscoveryPage` (which is the first-bind
 * full-page version), but adapted for a dialog: only candidates that aren't
 * already bound are shown, and confirmation merges them onto the existing
 * bound set rather than replacing it.
 *
 * Detection runs each time the dialog opens — versions can change if the
 * user installed a CLI between sessions.
 */
export default function AddToolsDialog({
  open,
  onOpenChange,
  alreadyBound,
  onAdded,
}: AddToolsDialogProps) {
  const [tools, setTools] = useState<ToolCard[]>([]);
  const [loading, setLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const alreadyBoundSet = useMemo(
    () => new Set(alreadyBound),
    [alreadyBound],
  );

  const detect = useCallback(async () => {
    setLoading(true);
    try {
      // Picker only needs the local "is it installed?" check — skip the
      // remote latest-version fetch that costs several seconds.
      const results = await invoke<ToolInfo[]>("get_tool_versions", {
        tools: null,
        wslShellByTool: null,
        includeLatest: false,
      });
      const detectedMap = new Map<string, ToolInfo>();
      for (const r of results) detectedMap.set(r.name, r);

      const candidates: ToolCard[] = TOOL_ORDER.filter(
        (id) => !alreadyBoundSet.has(id),
      ).map((id) => {
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
          enabled: detected,
        };
      });
      setTools(candidates);
    } catch (e) {
      console.error("[AddToolsDialog] detect failed", e);
      setTools(
        TOOL_ORDER.filter((id) => !alreadyBoundSet.has(id)).map((id) => ({
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
  }, [alreadyBoundSet]);

  useEffect(() => {
    if (open) {
      void detect();
    }
  }, [open, detect]);

  const toggle = (id: string) => {
    setTools((prev) =>
      prev.map((t) =>
        t.id === id && t.detected ? { ...t, enabled: !t.enabled } : t,
      ),
    );
  };

  const selected = tools.filter((t) => t.enabled).map((t) => t.id);
  const detectedCount = tools.filter((t) => t.detected).length;

  const handleConfirm = async () => {
    if (selected.length === 0) return;
    setSubmitting(true);
    try {
      // Merge with the existing bound list — `bindTools` overwrites
      // localStorage, so we must include the prior entries to keep them.
      const merged = Array.from(new Set([...alreadyBound, ...selected]));
      await bindTools(merged);
      toast.success(`已绑定 ${selected.length} 个工具`);
      onAdded(merged);
      onOpenChange(false);
    } catch (e) {
      console.error("[AddToolsDialog] bind failed", e);
      toast.error("绑定失败，请稍后重试");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>添加工具</DialogTitle>
          <DialogDescription>
            {loading
              ? "正在扫描已安装的 AI 工具..."
              : tools.length === 0
                ? "没有可添加的工具——所有已识别的工具都已绑定。"
                : `检测到 ${detectedCount} 个可绑定的工具，勾选后确认即可接入。`}
          </DialogDescription>
        </DialogHeader>

        {tools.length > 0 && (
          <div className="grid grid-cols-3 gap-3 py-2">
            {tools.map((tool) => (
              <button
                key={tool.id}
                onClick={() => toggle(tool.id)}
                disabled={!tool.detected || submitting}
                className={`flex flex-col items-center gap-1.5 rounded-xl border-2 px-3 py-3 transition-all ${
                  tool.enabled
                    ? "border-orange-400 bg-background shadow-sm"
                    : tool.detected
                      ? "border-border bg-background hover:border-orange-200"
                      : "border-border/50 bg-muted/30 opacity-50"
                }`}
              >
                <div
                  className={`flex h-8 w-8 items-center justify-center rounded-xl text-[11px] font-bold text-white ${
                    tool.detected ? tool.color : "bg-gray-400"
                  }`}
                >
                  {tool.abbr}
                </div>
                <span
                  className={`text-[12px] font-medium ${
                    tool.detected ? "text-foreground" : "text-muted-foreground"
                  }`}
                >
                  {tool.label}
                </span>
                <span className="text-[10px] text-muted-foreground">
                  {tool.version ?? "未安装"}
                </span>
              </button>
            ))}
          </div>
        )}

        <DialogFooter>
          <button
            onClick={() => onOpenChange(false)}
            disabled={submitting}
            className="rounded-md px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            取消
          </button>
          <button
            onClick={handleConfirm}
            disabled={selected.length === 0 || submitting || loading}
            className="rounded-md bg-gradient-to-r from-orange-400 to-orange-500 px-4 py-2 text-sm font-medium text-white shadow-sm hover:from-orange-500 hover:to-orange-600 disabled:opacity-50"
          >
            {submitting ? "正在绑定..." : `确认（${selected.length}）`}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
