import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { TOOL_META } from "@/config/toolMeta";
import { settingsApi } from "@/lib/api";
import { toolUpdatesApi, type ToolUpdateInfo } from "@/lib/api/toolUpdates";
import {
  updateTools,
  useToolUpdates,
  type ToolUpdateOutcome,
} from "@/hooks/useToolUpdates";

/** A tool row as far as updating it goes. */
export interface UpdatableTool {
  id: string;
  version: string | null;
  installationKind?: "desktopApp" | "cli";
}

/**
 * Ofox can run this update itself, one after another with the others.
 * Desktop apps stay out of a batch: large downloads, and they may close.
 */
export const batchable = (entry: ToolUpdateInfo) =>
  entry.update_supported && entry.installationKind !== "desktopApp";

/**
 * Updates the console's tools in place, the way magpie's Agents list does:
 * a pill on each row that is behind, and an "Update all" line above the
 * list once two or more are. Each outcome is said once, as a toast.
 */
export function useInlineToolUpdates() {
  const { t } = useTranslation();
  const state = useToolUpdates();
  // A desktop app that has to close before it updates waits here for the user.
  const [confirmTool, setConfirmTool] = useState<string | null>(null);

  const label = (id: string) => TOOL_META[id]?.label ?? id;

  /** The newer version of exactly this installation, if one is out. */
  const updateFor = (tool: UpdatableTool) =>
    state.tools.find(
      (entry) =>
        entry.name === tool.id &&
        entry.version === tool.version &&
        (!tool.installationKind ||
          entry.installationKind === tool.installationKind) &&
        entry.update_status === "available",
    );

  const sayOne = (o: ToolUpdateOutcome) => {
    const tool = label(o.tool);
    if (o.status === "failed") {
      toast.error(
        t("toolUpdates.updateFailed", { tool, message: o.error ?? "" }),
        { duration: 12000 },
      );
    } else if (o.status === "unchanged") {
      const pnpm =
        state.tools.find((entry) => entry.name === o.tool)?.update_source ===
        "pnpm";
      toast.warning(
        t("toolUpdates.notReached", { tool, version: o.after ?? o.before }),
        pnpm ? { description: t("toolUpdates.pnpmPolicy") } : undefined,
      );
    } else if (o.status === "current") {
      toast.success(t("toolUpdates.alreadyCurrent", { tool }));
    } else if (o.before && o.after && o.before !== o.after) {
      toast.success(
        t("toolUpdates.updatedFromTo", { tool, from: o.before, to: o.after }),
      );
    } else {
      toast.success(t("toolUpdates.updatedTo", { tool, version: o.after }));
    }
  };

  const run = async (names: string[]) => {
    const outcomes = await updateTools(names);
    if (outcomes.length === 1) {
      sayOne(outcomes[0]);
      return;
    }
    if (!outcomes.length) return;
    const done = outcomes.filter(
      (o) => o.status === "updated" || o.status === "current",
    );
    const list = done
      .map((o) =>
        o.before && o.after && o.before !== o.after
          ? `${label(o.tool)} ${o.before} → ${o.after}`
          : label(o.tool),
      )
      .join(", ");
    if (done.length === outcomes.length) {
      toast.success(t("toolUpdates.batchDone", { count: done.length, list }));
      return;
    }
    const left = outcomes.filter((o) => !done.includes(o));
    toast.error(
      t("toolUpdates.batchPartial", {
        ok: done.length,
        count: outcomes.length,
        names: left.map((o) => label(o.tool)).join(", "),
      }),
      {
        description: left
          .map((o) => o.error)
          .filter(Boolean)
          .join("\n"),
        duration: 12000,
      },
    );
  };

  /** What the row's pill does for this kind of installation. */
  const updateOne = async (tool: UpdatableTool) => {
    const entry = updateFor(tool);
    if (!entry || state.batch) return;
    if (entry.update_source === "sparkle") {
      // ChatGPT on macOS updates through its own menu; Ofox only opens it.
      try {
        await toolUpdatesApi.openApp(entry.name);
        toast.info(t("toolUpdates.sparkleOpened"));
      } catch {
        toast.error(t("toolUpdates.openAppFailed"));
      }
      return;
    }
    if (!entry.update_supported) {
      toast.info(t("toolUpdates.manualHint", { tool: label(entry.name) }));
      void settingsApi
        .openExternal(TOOL_META[entry.name]?.projectUrl ?? "")
        .catch(() => toast.error(t("settings.openReleaseNotesFailed")));
      return;
    }
    if (entry.installationKind === "desktopApp") {
      // If the check fails, assume it runs rather than close it unannounced.
      const running = await toolUpdatesApi
        .isAppRunning(entry.name)
        .catch(() => true);
      if (running) {
        setConfirmTool(entry.name);
        return;
      }
    }
    await run([entry.name]);
  };

  const updateAll = (tools: UpdatableTool[]) =>
    run(
      tools
        .map(updateFor)
        .filter((entry): entry is ToolUpdateInfo => !!entry && batchable(entry))
        .map((entry) => entry.name),
    );

  const confirmDialog = (
    <ConfirmDialog
      isOpen={confirmTool !== null}
      variant="info"
      title={t("toolUpdates.closeAppTitle")}
      message={t("toolUpdates.closeAppMessage")}
      confirmText={t("toolUpdates.closeAppConfirm")}
      onConfirm={() => {
        const name = confirmTool;
        setConfirmTool(null);
        if (name) void run([name]);
      }}
      onCancel={() => setConfirmTool(null)}
    />
  );

  return { state, updateFor, updateOne, updateAll, confirmDialog };
}
