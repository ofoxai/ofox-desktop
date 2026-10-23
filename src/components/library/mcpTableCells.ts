import type { LibraryInventory, McpDefinition } from "@/lib/api/library";
import { RESOURCE_TARGET_IDS } from "@/config/resourceTargets";

/** Bindable agents Ofox can write MCP servers for, in the tools page's order. */
export const MCP_TABLE_TOOLS = RESOURCE_TARGET_IDS;

export type McpCellState =
  /** Ofox wrote it and it is intact. */
  | "on"
  /** Assigned, not written yet. */
  | "pending"
  /** Already in the tool's config with the same content, written by someone else. */
  | "present"
  /** In the tool's config with the same content but switched off there. */
  | "presentOff"
  /** The tool holds a different entry under this name, or Ofox's write drifted. */
  | "conflict"
  /** The tool is not installed. */
  | "unavailable"
  /** This definition cannot be written in the tool's format. */
  | "unsupported"
  /** Written by an app into its own bundle; not managed here. */
  | "appOwned"
  | "off";

export interface McpCell {
  state: McpCellState;
  checked: boolean;
  /** Clicking toggles it; otherwise the cell only explains itself. */
  toggleable: boolean;
  reason?: string | null;
}

const ATTENTION = ["conflict", "needsReview", "unreadable", "failed"];

export function mcpCell(
  row: McpDefinition,
  toolId: string,
  inventory?: LibraryInventory,
): McpCell {
  const target = row.targets.find((target) => target.toolId === toolId);
  const entries = inventory?.entries ?? [];
  const sources = entries
    .filter((entry) => entry.matchedResourceIds.includes(row.id))
    .flatMap((entry) => entry.sources)
    .filter((source) => source.toolId === toolId);
  const sameNameElsewhere = entries.some(
    (entry) =>
      !entry.matchedResourceIds.includes(row.id) &&
      [row.id.toLowerCase(), row.name.toLowerCase()].includes(
        entry.name.toLowerCase(),
      ) &&
      entry.sources.some((source) => source.toolId === toolId),
  );
  if (row.appOwned)
    return { state: "appOwned", checked: false, toggleable: false };
  if (!target)
    return { state: "unsupported", checked: false, toggleable: false };
  if (ATTENTION.includes(target.status))
    return {
      state: "conflict",
      checked: !!target.desired,
      toggleable: false,
      reason: target.disabledReason,
    };
  if (target.desired)
    return {
      state: target.status === "applied" ? "on" : "pending",
      checked: true,
      toggleable: true,
    };
  if (target.discoveryStatus === "unsupported")
    return {
      state: "unsupported",
      checked: false,
      toggleable: false,
      reason: target.disabledReason,
    };
  if (sources.length || target.status === "shared") {
    const off =
      sources.length > 0 && sources.every((source) => source.disabled);
    return off
      ? { state: "presentOff", checked: false, toggleable: false }
      : { state: "present", checked: true, toggleable: true };
  }
  if (sameNameElsewhere)
    return { state: "conflict", checked: false, toggleable: false };
  if (!target.available)
    return {
      state: "unavailable",
      checked: false,
      toggleable: false,
      reason: target.disabledReason,
    };
  if (target.disabledReason)
    return {
      state: "unsupported",
      checked: false,
      toggleable: false,
      reason: target.disabledReason,
    };
  return { state: "off", checked: false, toggleable: true };
}
